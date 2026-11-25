//! The Claude Code adapter: `claude -p --output-format stream-json`.
//!
//! A run is `claude -p` with an MCP file and a settings file in the run's directory. The MCP file names the servers of
//! the run; the settings turn Claude Code's own attribution off (the commit trailers are the harness's), switch its
//! tool search off so that the agentbus tools are all there from the first turn, forbid background tasks (a headless
//! run ends with its turn and would kill them) and give its Bash tool a timeout long enough for a gate. The session of
//! a new run is the ID the harness made up (`--session-id`); a resumed run names it (`--resume`).
//!
//! The stream is one JSON object a line: `system` (`init` names the session and the model), `assistant` and `user`
//! (their content blocks: text, tool calls and their results) and `result`, which ends the run and holds the final
//! message.

use std::collections::BTreeMap;
use std::num::NonZeroU32;
use std::time::Duration;

use agent_core::cli::{
    ActivityEvent, BuildRunError, CliAdapter, OutputError, OutputParser, RunMode, RunOutput, RunRequest, RunSpec,
    RunUsage, StreamOutcome,
};
use agent_core::config::Limits;
use agent_core::kinds::CliKind;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{
    CallUsage, Progress, UsageTally, json_file, mcp_file, path_text, path_under, read_event, refused, serena_excludes,
    servers, text_event, tool_activity,
};

/// The longest a command of the agent's Bash tool may run, by default and at most: a gate runs in the foreground.
const BASH_TIMEOUT: Duration = Duration::from_mins(20);

/// Serena's name for Claude Code as its client.
const SERENA_CONTEXT: &str = "claude-code";

/// The name of the MCP configuration file in the run's directory.
const MCP_FILE: &str = "mcp.json";

/// The name of the settings file in the run's directory.
const SETTINGS_FILE: &str = "claude-settings.json";

/// The subtype of a `result` that ended well.
const SUCCESS_SUBTYPE: &str = "success";

/// The subtype of a `result` that used up the run's `--max-turns`.
const MAX_TURNS_SUBTYPE: &str = "error_max_turns";

/// The start of the key of a model call whose message the stream gives no ID: such a call is counted once, under its
/// own number.
const ANONYMOUS_CALL: &str = "anonymous-call-";

/// The adapter of Claude Code.
#[derive(Debug, Clone, Copy)]
pub struct ClaudeAdapter;

/// Claude Code's `attribution` setting: the text it adds to a commit and to a pull request.
#[derive(Debug, Serialize)]
struct Attribution {
    /// The trailer of a commit; empty for none.
    commit: &'static str,
    /// The line of a pull request; empty for none.
    pr: &'static str,
}

/// The settings of a run, as `--settings` takes them.
#[derive(Debug, Serialize)]
struct Settings {
    /// Whether Claude Code adds a `Co-Authored-By` trailer; never.
    #[serde(rename = "includeCoAuthoredBy")]
    include_co_authored_by: bool,
    /// Its attribution of commits and pull requests; none.
    attribution: Attribution,
    /// Environment variables of the run's tools, which win over the human's own settings.
    env: BTreeMap<&'static str, String>,
}

/// The settings of every run.
fn settings() -> Settings {
    let bash_timeout_ms = BASH_TIMEOUT.as_millis().to_string();
    Settings {
        include_co_authored_by: false,
        attribution: Attribution { commit: "", pr: "" },
        env: BTreeMap::from([
            ("CLAUDE_CODE_DISABLE_BACKGROUND_TASKS", "1".to_owned()),
            ("BASH_DEFAULT_TIMEOUT_MS", bash_timeout_ms.clone()),
            ("BASH_MAX_TIMEOUT_MS", bash_timeout_ms),
            ("ENABLE_TOOL_SEARCH", "false".to_owned()),
        ]),
    }
}

impl CliAdapter for ClaudeAdapter {
    fn kind(&self) -> CliKind {
        CliKind::Claude
    }

    fn build_run(&self, request: &RunRequest) -> Result<RunSpec, BuildRunError> {
        self.build_run_with_max_turns(request, Limits::default().max_turns)
    }

    fn build_run_with_max_turns(&self, request: &RunRequest, max_turns: NonZeroU32) -> Result<RunSpec, BuildRunError> {
        let max_turns = max_turns.to_string();
        let servers = servers(CliKind::Claude, request, SERENA_CONTEXT)?;
        let mcp_path = path_under(CliKind::Claude, &request.run_dir, MCP_FILE)?;
        let settings_path = path_under(CliKind::Claude, &request.run_dir, SETTINGS_FILE)?;

        let mut args: Vec<String> = [
            "-p",
            "--output-format",
            "stream-json",
            "--verbose",
            "--dangerously-skip-permissions",
            "--mcp-config",
            path_text(CliKind::Claude, mcp_path.as_path())?,
            "--max-turns",
            &max_turns,
        ]
        .map(str::to_owned)
        .into();
        // A run with no server is given none of the human's own either.
        if servers.is_empty() {
            args.push("--strict-mcp-config".to_owned());
        }
        args.extend([
            "--settings".to_owned(),
            path_text(CliKind::Claude, settings_path.as_path())?.to_owned(),
        ]);
        if let Some(model) = &request.model {
            args.extend(["--model".to_owned(), model.as_str().to_owned()]);
        }
        match &request.mode {
            RunMode::New { session_id } => args.extend(["--session-id".to_owned(), session_id.to_string()]),
            RunMode::Resume { session_id } => args.extend(["--resume".to_owned(), session_id.to_string()]),
        }
        args.extend(request.extra_args.as_slice().iter().cloned());
        args.push(request.prompt.clone());

        Ok(RunSpec {
            program: request.program.clone(),
            args,
            env: BTreeMap::new(),
            working_dir: request.working_dir.clone(),
            files: vec![
                mcp_file(CliKind::Claude, mcp_path, &servers)?,
                json_file(CliKind::Claude, settings_path, &settings())?,
            ],
            worktree_excludes: serena_excludes(&servers),
        })
    }

    fn parser(&self) -> Box<dyn OutputParser> {
        Box::new(ClaudeParser {
            progress: Progress::new(),
            rate_limit_reset: None,
            usage: UsageTally::default(),
            total_cost_usd: None,
        })
    }
}

/// A line of the stream, by its `type`.
#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum Event {
    /// Metadata of the run; `init` opens it.
    System(System),
    /// A message of the model.
    Assistant(Turn),
    /// What the tools answered.
    User(Turn),
    /// The end of the run.
    Result(Finish),
    /// A rate limit event: the provider refused the model.
    RateLimit(RateLimitEvent),
    /// Any other event: partial messages, ...
    #[serde(other)]
    Other,
}

/// What a `system` event says.
#[derive(Debug, Deserialize)]
struct System {
    /// Which kind of system event it is.
    subtype: Option<SystemSubtype>,
    /// The session of the run.
    session_id: Option<String>,
    /// The model that runs.
    model: Option<String>,
}

/// The kinds of `system` event the adapter tells apart.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
enum SystemSubtype {
    /// The first event of the run.
    Init,
    /// Anything else: hooks, token estimates, ...
    #[serde(other)]
    Other,
}

/// An `assistant` or `user` event.
#[derive(Debug, Deserialize)]
struct Turn {
    /// The message.
    message: Message,
}

/// The message of a turn.
#[derive(Debug, Deserialize)]
struct Message {
    /// Its content: blocks, or for the prompt of the user turn a bare text.
    #[serde(default)]
    content: Value,
    /// The API message's ID: the content blocks of one model call share it.
    id: Option<String>,
    /// What the model call used; absent on the prompt of the user turn.
    usage: Option<ClaudeUsage>,
}

/// The usage of one model call as the API reports it. Each part is optional: a part that is absent is not told.
#[derive(Debug, Clone, Copy, Deserialize)]
struct ClaudeUsage {
    /// Tokens of the input that was not read from or written to the cache.
    #[serde(rename = "input_tokens")]
    input: Option<u64>,
    /// Tokens of the input read from the cache.
    #[serde(rename = "cache_read_input_tokens")]
    cache_read: Option<u64>,
    /// Tokens of the input written to the cache.
    #[serde(rename = "cache_creation_input_tokens")]
    cache_creation: Option<u64>,
    /// Tokens the model wrote.
    #[serde(rename = "output_tokens")]
    output: Option<u64>,
}

impl ClaudeUsage {
    /// The context of the call: its input with what came from the cache and what went into it. None unless all three
    /// parts are told, since a missing one would make the size a guess.
    fn context(self) -> Option<u64> {
        self.input?
            .checked_add(self.cache_read?)?
            .checked_add(self.cache_creation?)
    }

    /// The call as the tally keeps it: no cost, which the result of the run reports as a total.
    fn call(self) -> CallUsage {
        CallUsage {
            context: self.context(),
            written: self.output,
            cost: None,
        }
    }
}

/// A block of a message.
#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum Block {
    /// Something the model said.
    Text {
        /// What it said.
        text: String,
    },
    /// A call of a tool.
    ToolUse {
        /// The tool.
        name: String,
        /// Its input.
        #[serde(default)]
        input: Value,
    },
    /// The answer of a tool.
    ToolResult {
        /// Whether the call failed; absent or null for a call that did not.
        #[serde(default)]
        is_error: Option<bool>,
        /// What the tool answered: a text or blocks.
        #[serde(default)]
        content: Value,
    },
    /// Anything else: thinking, images, ...
    #[serde(other)]
    Other,
}

/// The `rate_limit_event` that Claude Code writes when its provider refuses.
#[derive(Debug, Deserialize)]
struct RateLimitEvent {
    /// The details of the limit.
    #[serde(default)]
    rate_limit_info: RateLimitInfo,
}

/// The structured info of a rate limit.
#[derive(Debug, Default, Deserialize)]
struct RateLimitInfo {
    /// `rejected`: the call was refused.
    #[serde(default)]
    status: String,
    /// The Unix epoch second when the rate limit resets.
    #[serde(default)]
    #[serde(rename = "resetsAt")]
    resets_at: Option<u64>,
    /// E.g. `five_hour`, `daily`.
    #[serde(default)]
    #[serde(rename = "rateLimitType")]
    rate_limit_type: String,
}

/// The `result` event: how the run ended.
#[derive(Debug, Deserialize)]
struct Finish {
    /// `success`, or `error_max_turns`, `error_during_execution`, ...
    subtype: Option<String>,
    /// Whether the result reports an error, whatever its subtype; absent or null for one that does not.
    #[serde(default)]
    is_error: Option<bool>,
    /// The HTTP status the API gave for an error result, e.g. 429.
    #[serde(default)]
    api_error_status: Option<u16>,
    /// The final message; an error result may have none.
    result: Option<String>,
    /// The messages of an error result (`error_during_execution`, ...), which may come with no `result`.
    #[serde(default)]
    errors: Vec<String>,
    /// The session of the run.
    session_id: Option<String>,
    /// How many turns the run took.
    num_turns: Option<u64>,
    /// The cost of the whole run in US dollars, as the CLI reports it.
    total_cost_usd: Option<f64>,
    /// What the whole run used; only its output tokens are read.
    usage: Option<ClaudeUsage>,
}

impl Finish {
    /// Whether the run ended in failure.
    fn failed(&self) -> bool {
        self.is_error == Some(true)
            || self
                .subtype
                .as_deref()
                .is_some_and(|subtype| subtype != SUCCESS_SUBTYPE)
    }

    /// The line the activity log shows for the end of the run.
    fn summary(&self) -> String {
        let subtype = self.subtype.as_deref().unwrap_or("unknown");
        match self.num_turns {
            Some(turns) => format!("result: {subtype}, turns: {turns}"),
            None => format!("result: {subtype}"),
        }
    }
}

/// The blocks of a turn's `content`; a bare text, such as the prompt, has none.
fn blocks(line: &str, content: Value) -> Result<Vec<Block>, OutputError> {
    match content {
        Value::Array(_) => serde_json::from_value(content).map_err(|source| refused(line, source)),
        _ => Ok(Vec::new()),
    }
}

/// The text of what a tool answered: a text as it is, blocks by their texts, anything else as JSON.
fn answer_text(content: &Value) -> String {
    let text = match content {
        Value::String(text) => text.clone(),
        Value::Array(blocks) => blocks
            .iter()
            .filter_map(|block| block.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join("\n"),
        other => other.to_string(),
    };
    let text = text.trim();
    if text.is_empty() {
        content.to_string()
    } else {
        text.to_owned()
    }
}

/// The parser of one Claude Code run.
#[derive(Debug)]
struct ClaudeParser {
    /// What the stream has shown.
    progress: Progress,
    /// The resetsAt epoch second from the most recent rate_limit_event; none when none was seen.
    rate_limit_reset: Option<u64>,
    /// The model calls of the run, each counted once.
    usage: UsageTally,
    /// The cost of the run the result line reports; none until it comes.
    total_cost_usd: Option<f64>,
}

impl ClaudeParser {
    /// The steps a block shows.
    fn block(block: Block) -> Option<ActivityEvent> {
        match block {
            Block::Text { text } => text_event(&text),
            Block::ToolUse { name, input } => Some(tool_activity(&name, &input)),
            Block::ToolResult {
                is_error: Some(true),
                content,
            } => Some(ActivityEvent::Error(answer_text(&content))),
            Block::ToolResult { .. } | Block::Other => None,
        }
    }
}

impl OutputParser for ClaudeParser {
    fn session(&self) -> Option<&agent_core::cli::SessionId> {
        self.progress.session_id.as_ref()
    }

    fn push_line(&mut self, line: &str) -> Result<Vec<ActivityEvent>, OutputError> {
        let Some(event) = read_event::<Event>(line)? else {
            return Ok(Vec::new());
        };
        match event {
            Event::System(System {
                subtype: Some(SystemSubtype::Init),
                session_id,
                model,
            }) => {
                // An empty ID names no session; refusing it would drop the event.
                if let Some(id) = session_id.filter(|id| !id.is_empty()) {
                    self.progress.note_session(line, &id)?;
                }
                let model = model.unwrap_or_default();
                Ok(vec![ActivityEvent::Info(format!("session started, model {model}"))])
            }
            Event::Assistant(Turn { message }) => {
                // A model call that tells no usage is kept as a call that tells nothing, so that the call before it is
                // not told as the last one. A message without an ID is its own call.
                let id = message
                    .id
                    .unwrap_or_else(|| format!("{ANONYMOUS_CALL}{}", self.usage.calls()));
                self.usage
                    .record(id, message.usage.map_or_else(CallUsage::default, ClaudeUsage::call));
                Ok(blocks(line, message.content)?
                    .into_iter()
                    .filter_map(Self::block)
                    .collect())
            }
            Event::User(Turn { message }) => Ok(blocks(line, message.content)?
                .into_iter()
                .filter_map(Self::block)
                .collect()),
            Event::Result(finish) => {
                // An empty ID names no session; refusing it would lose the outcome below.
                if let Some(id) = finish.session_id.as_deref().filter(|id| !id.is_empty()) {
                    self.progress.note_session(line, id)?;
                }
                self.progress.stream = if finish.subtype.as_deref() == Some(MAX_TURNS_SUBTYPE) {
                    StreamOutcome::TurnLimit
                } else if finish.failed() {
                    StreamOutcome::Failed
                } else {
                    StreamOutcome::Succeeded
                };
                let mut events: Vec<ActivityEvent> = Vec::new();
                for message in &finish.errors {
                    self.progress.note_error_line(message);
                    events.push(ActivityEvent::Error(message.clone()));
                }
                events.push(ActivityEvent::Info(finish.summary()));
                self.progress.final_message = finish.result.filter(|text| !text.trim().is_empty());
                // Build a structured refusal from the error result, when the API gave a status.
                if finish.failed() {
                    if let Some(status) = finish.api_error_status {
                        let message = finish.result.as_deref().unwrap_or("");
                        // The refusal kind is inferred by RefusalSummary::from_fields.
                        self.progress.refusal = Some(agent_core::decide::refusal::RefusalSummary::from_fields(
                            &status.to_string(),
                            message,
                            "",
                            agent_core::decide::outcome::PauseReason::RateLimit,
                        ));
                    }
                }
                // The result's usage is the total of the run, never a context: only its output tokens and its cost are
                // taken from it.
                self.usage.report_written(finish.usage.and_then(|usage| usage.output));
                self.total_cost_usd = finish.total_cost_usd;
                Ok(events)
            }
            Event::System(_) | Event::Other => Ok(Vec::new()),
            Event::RateLimit(rate) => {
                if let Some(reset) = rate.rate_limit_info.resets_at {
                    self.rate_limit_reset = Some(reset);
                }
                Ok(Vec::new())
            }
        }
    }

    fn usage(&self) -> RunUsage {
        RunUsage {
            cost_usd: self.total_cost_usd,
            ..self.usage.usage()
        }
    }

    fn latest_context(&self) -> Option<u64> {
        self.usage.latest_context()
    }

    fn finish(self: Box<Self>, stderr: &str, tail_lines: NonZeroU32) -> RunOutput {
        self.progress.finish(stderr, tail_lines)
    }
}
