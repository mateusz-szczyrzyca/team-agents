//! The Antigravity adapter: `agy -p --output-format stream-json`.
//!
//! `agy` has a five-minute limit on a print-mode turn unless it is given a longer one (`--print-timeout`). It reads its
//! MCP servers from the workspace's `.agents/mcp_config.json`, so the file is laid down in the worktree and kept out of
//! git. Flags the agent's configuration adds, such as `--effort high`, are passed on as they are. A resumed run names
//! the conversation (`--conversation`); the conversation of a new run is the one `agy` reports.
//!
//! The stream is one JSON object a line: `init` (names the conversation), `step_update` (a step of the agent: its
//! response in pieces, or a tool call, each first active and then done) and `result`, which ends the run and holds the
//! final message.

use std::collections::BTreeMap;
use std::num::NonZeroU32;

use agent_core::cli::{
    ActivityEvent, BuildRunError, CliAdapter, OutputCause, OutputError, OutputParser, RunMode, RunOutput, RunRequest,
    RunSpec, StreamOutcome,
};
use agent_core::kinds::CliKind;
use serde::Deserialize;
use serde_json::Value;

use super::{Progress, mcp_file, path_under, read_event, serena_excludes, servers, text_event, tool_activity};

/// The longest a print-mode turn may take: `agy`'s own limit is five minutes.
const PRINT_TIMEOUT: &str = "8h";

/// Serena's name for Antigravity as its client.
const SERENA_CONTEXT: &str = "antigravity";

/// The MCP configuration file of the workspace, relative to the worktree.
const MCP_FILE: &str = ".agents/mcp_config.json";

/// The `status` of a `result` that ended well.
const SUCCESS_STATUS: &str = "SUCCESS";

/// The tool through which `agy` calls an MCP tool.
const MCP_TOOL: &str = "call_mcp_tool";

/// The most bytes of one response that are put together from its pieces: a bound on a stream that never says a step is
/// done.
const MAX_RESPONSE_BYTES: usize = 1 << 20;

/// The most responses that may be unfinished at once: with [`MAX_RESPONSE_BYTES`] it bounds what the parser holds. A
/// real run has one or two open.
const MAX_OPEN_RESPONSES: usize = 64;

/// The adapter of Antigravity.
#[derive(Debug, Clone, Copy)]
pub struct AgyAdapter;

impl CliAdapter for AgyAdapter {
    fn kind(&self) -> CliKind {
        CliKind::Gemini
    }

    fn build_run(&self, request: &RunRequest) -> Result<RunSpec, BuildRunError> {
        let servers = servers(CliKind::Gemini, request, SERENA_CONTEXT)?;
        let mcp_path = path_under(CliKind::Gemini, &request.working_dir, MCP_FILE)?;

        let mut args: Vec<String> = [
            "--output-format",
            "stream-json",
            "--dangerously-skip-permissions",
            "--print-timeout",
            PRINT_TIMEOUT,
        ]
        .map(str::to_owned)
        .into();
        if let Some(model) = &request.model {
            args.extend(["--model".to_owned(), model.as_str().to_owned()]);
        }
        args.extend(request.extra_args.as_slice().iter().cloned());
        if let RunMode::Resume { session_id } = &request.mode {
            args.extend(["--conversation".to_owned(), session_id.to_string()]);
        }
        args.extend(["-p".to_owned(), request.prompt.clone()]);

        let mut worktree_excludes = vec![MCP_FILE.to_owned()];
        worktree_excludes.extend(serena_excludes(&servers));
        Ok(RunSpec {
            program: request.program.clone(),
            args,
            env: BTreeMap::new(),
            working_dir: request.working_dir.clone(),
            files: vec![mcp_file(CliKind::Gemini, mcp_path, &servers)?],
            worktree_excludes,
        })
    }

    fn parser(&self) -> Box<dyn OutputParser> {
        Box::new(AgyParser {
            progress: Progress::new(),
            pieces: BTreeMap::new(),
        })
    }
}

/// A line of the stream, by its `event`.
#[derive(Debug, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
enum Event {
    /// The first event; names the conversation.
    Init {
        /// The conversation of the run.
        conversation_id: Option<String>,
    },
    /// A step of the agent changed.
    StepUpdate {
        /// The step.
        step_update: Step,
    },
    /// The end of the run.
    #[serde(rename = "result")]
    Finished {
        /// How it ended.
        result: Finish,
    },
    /// Any other event.
    #[serde(other)]
    Other,
}

/// A step of the agent.
#[derive(Debug, Deserialize)]
struct Step {
    /// The number of the step in the conversation.
    #[serde(default, rename = "step_index")]
    index: u64,
    /// Whether the step is over.
    state: Option<StepState>,
    /// What kind of step it is.
    #[serde(rename = "step_type")]
    kind: Option<StepType>,
    /// A piece of the agent's response.
    text_delta: Option<String>,
    /// The tool a tool step calls.
    tool_name: Option<String>,
    /// The call and what came of it.
    tool_info: Option<ToolInfo>,
}

/// Whether a step is over.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
enum StepState {
    /// The step is over.
    Done,
    /// Any other state: the step is still going.
    #[serde(other)]
    Going,
}

/// The kinds of step the adapter tells apart.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
enum StepType {
    /// The agent answers in text.
    AgentResponse,
    /// The agent calls a tool.
    Tool,
    /// Any other: the user's input, system messages, ...
    #[serde(other)]
    Other,
}

/// A tool call of a step.
#[derive(Debug, Deserialize)]
struct ToolInfo {
    /// The arguments the tool was called with.
    #[serde(default)]
    parameters: Value,
    /// Why the call failed, if it did.
    error: Option<ToolError>,
}

/// A failed tool call.
#[derive(Debug, Deserialize)]
struct ToolError {
    /// What went wrong.
    message: Option<String>,
    /// The kind of failure.
    #[serde(rename = "type")]
    kind: Option<String>,
}

/// The `result` event.
#[derive(Debug, Deserialize)]
struct Finish {
    /// `SUCCESS`, `ERROR`, ...
    status: Option<String>,
    /// The final message; empty for a run that failed.
    response: Option<String>,
    /// Why the run failed, if it did.
    error: Option<String>,
    /// The conversation of the run.
    conversation_id: Option<String>,
}

/// The call of an MCP tool through `call_mcp_tool`, as a step: a call of agentbus, or of another server's tool.
fn mcp_call(parameters: &Value) -> ActivityEvent {
    let text = |key: &str| parameters.get(key).and_then(Value::as_str).unwrap_or_default();
    let name = match text("ServerName") {
        "" => text("ToolName").to_owned(),
        server => format!("{server}.{}", text("ToolName")),
    };
    tool_activity(&name, parameters.get("Arguments").unwrap_or(&Value::Null))
}

/// The first `max` bytes of `text`, fewer when that would cut a character.
fn head(text: &str, max: usize) -> &str {
    match text.split_at_checked(text.floor_char_boundary(max)) {
        Some((head, _)) => head,
        None => text,
    }
}

/// The parser of one `agy` run.
#[derive(Debug)]
struct AgyParser {
    /// What the stream has shown.
    progress: Progress,
    /// The pieces of the responses that are not done, by step.
    pieces: BTreeMap<u64, String>,
}

impl AgyParser {
    /// The steps a change of `step` shows; `line` is the line it came from. A response that would be one open
    /// response too many is refused with [`OutputCause::TooManyOpenSteps`] and not kept.
    fn step(&mut self, line: &str, step: Step) -> Result<Vec<ActivityEvent>, OutputError> {
        let done = step.state == Some(StepState::Done);
        Ok(match step.kind {
            Some(StepType::AgentResponse) => {
                if self.pieces.len() >= MAX_OPEN_RESPONSES && !self.pieces.contains_key(&step.index) {
                    return Err(OutputError::new(line, OutputCause::TooManyOpenSteps));
                }
                let text = self.pieces.entry(step.index).or_default();
                if let Some(delta) = &step.text_delta {
                    text.push_str(head(delta, MAX_RESPONSE_BYTES.saturating_sub(text.len())));
                }
                if !done {
                    return Ok(Vec::new());
                }
                let finished = std::mem::take(text);
                self.pieces.remove(&step.index);
                text_event(&finished).into_iter().collect()
            }
            Some(StepType::Tool) if done => Self::tool(step.tool_name.as_deref(), step.tool_info),
            Some(StepType::Tool | StepType::Other) | None => Vec::new(),
        })
    }

    /// The steps a finished tool step shows: the call and, if it failed, the error.
    fn tool(name: Option<&str>, info: Option<ToolInfo>) -> Vec<ActivityEvent> {
        let Some(info) = info else {
            return Vec::new();
        };
        let name = name.unwrap_or_default();
        let call = if name == MCP_TOOL {
            mcp_call(&info.parameters)
        } else {
            tool_activity(name, &info.parameters)
        };
        let mut events = vec![call];
        if let Some(error) = info.error {
            let message = error
                .message
                .or(error.kind)
                .unwrap_or_else(|| "a tool call failed".to_owned());
            events.push(ActivityEvent::Error(message));
        }
        events
    }

    /// The end of the run.
    fn finished(&mut self, line: &str, result: Finish) -> Result<Vec<ActivityEvent>, OutputError> {
        // An empty ID names no session; refusing it here would lose the outcome below.
        if let Some(id) = result.conversation_id.as_deref().filter(|id| !id.is_empty()) {
            self.progress.note_session(line, id)?;
        }
        let mut events = Vec::new();
        if let Some(error) = result.error.as_deref().filter(|error| !error.trim().is_empty()) {
            self.progress.note_error_line(error);
            events.push(ActivityEvent::Error(error.trim().to_owned()));
        }
        let status = result.status.as_deref().unwrap_or("unknown");
        events.push(ActivityEvent::Info(format!("result: {status}")));
        self.progress.stream = if result.status.as_deref() == Some(SUCCESS_STATUS) {
            StreamOutcome::Succeeded
        } else {
            StreamOutcome::Failed
        };
        self.progress.final_message = result.response.filter(|text| !text.trim().is_empty());
        Ok(events)
    }
}

impl OutputParser for AgyParser {
    fn push_line(&mut self, line: &str) -> Result<Vec<ActivityEvent>, OutputError> {
        let Some(event) = read_event::<Event>(line)? else {
            return Ok(Vec::new());
        };
        match event {
            Event::Init { conversation_id } => {
                let Some(id) = conversation_id.filter(|id| !id.is_empty()) else {
                    return Ok(Vec::new());
                };
                self.progress.note_session(line, &id)?;
                Ok(vec![ActivityEvent::Info(format!("session started {id}"))])
            }
            Event::StepUpdate { step_update } => self.step(line, step_update),
            Event::Finished { result } => self.finished(line, result),
            Event::Other => Ok(Vec::new()),
        }
    }

    fn finish(self: Box<Self>, stderr: &str, tail_lines: NonZeroU32) -> RunOutput {
        self.progress.finish(stderr, tail_lines)
    }
}
