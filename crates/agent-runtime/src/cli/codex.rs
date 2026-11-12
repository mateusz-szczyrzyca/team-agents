//! The Codex adapter: `codex exec --json`.
//!
//! Codex takes its MCP servers as `-c key=value` overrides of its configuration; a value is TOML, and a JSON string
//! or array of strings is one. Serena starts its language servers before it answers, so it gets a longer start-up
//! timeout than Codex's default. A resumed run is `codex exec … resume <id> <prompt>`; the session of a new run is the
//! `thread_id` of the stream's first event, since Codex makes up its own.
//!
//! The stream is one JSON object a line: `thread.started`, `turn.started`, `item.started` and `item.completed` (an
//! item is a message, a command, a file change, a tool call, ...), and `turn.completed` or `turn.failed`. The final
//! message is the last message of the turn once the turn has completed.

use std::collections::BTreeMap;
use std::num::NonZeroU32;

use agent_core::cli::{
    ActivityEvent, BuildRunError, CliAdapter, McpServerKind, OutputError, OutputParser, RunMode, RunOutput, RunRequest,
    RunSpec, StreamOutcome,
};
use agent_core::kinds::CliKind;
use serde::Deserialize;
use serde_json::Value;

use super::{Progress, Server, path_text, read_event, serena_excludes, servers, text_event, tool_activity};

/// How long agentbus may take to answer a call, in seconds: its `wait_for_message` waits for other agents.
const BUS_TOOL_TIMEOUT_SECS: u32 = 900;

/// How long Serena may take to start, in seconds: it starts its language servers first.
const SERENA_STARTUP_TIMEOUT_SECS: u32 = 120;

/// How long Serena may take to answer a call, in seconds.
const SERENA_TOOL_TIMEOUT_SECS: u32 = 300;

/// Serena's name for Codex as its client.
const SERENA_CONTEXT: &str = "codex";

/// The adapter of Codex.
#[derive(Debug, Clone, Copy)]
pub struct CodexAdapter;

/// `value` as a TOML basic string: JSON's escapes are TOML's, but for the one character JSON writes as it is and TOML
/// does not allow, U+007F.
fn toml_string(value: &str) -> String {
    Value::from(value).to_string().replace('\u{7f}', "\\u007f")
}

/// `values` as a TOML array of strings.
fn toml_array(values: &[String]) -> String {
    let items: Vec<String> = values.iter().map(|value| toml_string(value)).collect();
    format!("[{}]", items.join(","))
}

/// The `-c` arguments that give Codex `server`.
fn server_settings(server: &Server) -> Vec<String> {
    let name = server.name();
    let mut settings = vec![
        format!("mcp_servers.{name}.command={}", toml_string(&server.program)),
        format!("mcp_servers.{name}.args={}", toml_array(&server.args)),
    ];
    match server.kind {
        McpServerKind::Agentbus => {
            settings.push(format!("mcp_servers.{name}.tool_timeout_sec={BUS_TOOL_TIMEOUT_SECS}"));
        }
        McpServerKind::Serena => {
            settings.push(format!(
                "mcp_servers.{name}.startup_timeout_sec={SERENA_STARTUP_TIMEOUT_SECS}"
            ));
            settings.push(format!(
                "mcp_servers.{name}.tool_timeout_sec={SERENA_TOOL_TIMEOUT_SECS}"
            ));
        }
    }
    settings
}

impl CliAdapter for CodexAdapter {
    fn kind(&self) -> CliKind {
        CliKind::Codex
    }

    fn build_run(&self, request: &RunRequest) -> Result<RunSpec, BuildRunError> {
        let servers = servers(CliKind::Codex, request, SERENA_CONTEXT)?;
        let mut args: Vec<String> = [
            "exec",
            "--json",
            "--dangerously-bypass-approvals-and-sandbox",
            "-C",
            path_text(CliKind::Codex, request.working_dir.as_path())?,
        ]
        .map(str::to_owned)
        .into();
        for setting in servers.iter().flat_map(server_settings) {
            args.extend(["-c".to_owned(), setting]);
        }
        if let Some(model) = &request.model {
            args.extend(["-m".to_owned(), model.as_str().to_owned()]);
        }
        args.extend(request.extra_args.as_slice().iter().cloned());
        if let RunMode::Resume { session_id } = &request.mode {
            args.extend(["resume".to_owned(), session_id.to_string()]);
        }
        args.push(request.prompt.clone());

        Ok(RunSpec {
            program: request.program.clone(),
            args,
            env: BTreeMap::new(),
            working_dir: request.working_dir.clone(),
            files: Vec::new(),
            worktree_excludes: serena_excludes(&servers),
        })
    }

    fn parser(&self) -> Box<dyn OutputParser> {
        Box::new(CodexParser {
            progress: Progress::new(),
            last_message: None,
        })
    }
}

/// A line of the stream, by its `type`.
#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
enum Event {
    /// The conversation exists; names it.
    #[serde(rename = "thread.started")]
    ThreadStarted {
        /// The ID of the conversation, with which a run is resumed.
        thread_id: String,
    },
    /// The agent starts on an item.
    #[serde(rename = "item.started")]
    ItemStarted {
        /// The item.
        item: Item,
    },
    /// The agent is done with an item.
    #[serde(rename = "item.completed")]
    ItemCompleted {
        /// The item.
        item: Item,
    },
    /// The turn ended well.
    #[serde(rename = "turn.completed")]
    TurnCompleted,
    /// The turn failed.
    #[serde(rename = "turn.failed")]
    TurnFailed {
        /// Why.
        error: Failure,
    },
    /// An error of the run; the turn may go on or fail after it.
    #[serde(rename = "error")]
    Error {
        /// What went wrong.
        message: String,
    },
    /// Any other event: `turn.started`, `item.updated`, ...
    #[serde(other)]
    Other,
}

/// Why a turn failed.
#[derive(Debug, Deserialize)]
struct Failure {
    /// What went wrong, as the provider said.
    message: String,
}

/// An item of a turn.
#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum Item {
    /// A message of the agent.
    AgentMessage {
        /// What it says.
        text: String,
    },
    /// A shell command.
    CommandExecution {
        /// The command line.
        command: String,
        /// Its exit code once it ran.
        exit_code: Option<i64>,
    },
    /// Changes to files.
    FileChange {
        /// The files.
        #[serde(default)]
        changes: Vec<Change>,
    },
    /// A call of an MCP tool.
    McpToolCall {
        /// The server of the tool.
        server: Option<String>,
        /// The tool.
        tool: String,
        /// Its arguments.
        #[serde(default)]
        arguments: Value,
    },
    /// A web search.
    WebSearch {
        /// What it looks for.
        query: String,
    },
    /// A non-fatal error the agent saw.
    Error {
        /// What went wrong.
        message: String,
    },
    /// Anything else: reasoning, plans, ...
    #[serde(other)]
    Other,
}

/// A change to one file.
#[derive(Debug, Deserialize)]
struct Change {
    /// The file.
    path: String,
}

/// The parser of one Codex run.
#[derive(Debug)]
struct CodexParser {
    /// What the stream has shown.
    progress: Progress,
    /// The latest message of the agent; it is the final message once the turn completes.
    last_message: Option<String>,
}

impl CodexParser {
    /// The steps an item that just started shows.
    fn started(item: Item) -> Vec<ActivityEvent> {
        match item {
            Item::CommandExecution { command, .. } => vec![ActivityEvent::Command(command)],
            _ => Vec::new(),
        }
    }

    /// The steps an item that was just completed shows; remembers the agent's message.
    fn completed(&mut self, item: Item) -> Vec<ActivityEvent> {
        match item {
            Item::AgentMessage { text } => {
                let event = text_event(&text);
                self.last_message = Some(text);
                event.into_iter().collect()
            }
            Item::CommandExecution {
                command,
                exit_code: Some(code),
            } if code != 0 => vec![ActivityEvent::Error(format!("exit {code}: {command}"))],
            Item::FileChange { changes } => changes
                .into_iter()
                .map(|change| ActivityEvent::FileEdit { path: change.path })
                .collect(),
            Item::McpToolCall {
                server,
                tool,
                arguments,
            } => {
                let name = match server {
                    Some(server) => format!("{server}.{tool}"),
                    None => tool,
                };
                vec![tool_activity(&name, &arguments)]
            }
            Item::WebSearch { query } => vec![ActivityEvent::ToolCall {
                name: "web_search".to_owned(),
                summary: query,
            }],
            Item::Error { message } => vec![ActivityEvent::Error(message)],
            Item::CommandExecution { .. } | Item::Other => Vec::new(),
        }
    }

    /// An error that ends or may end the run: its message is a step and a line of the error tail.
    fn failure(&mut self, message: String) -> Vec<ActivityEvent> {
        self.progress.note_error_line(&message);
        vec![ActivityEvent::Error(message)]
    }
}

impl OutputParser for CodexParser {
    fn push_line(&mut self, line: &str) -> Result<Vec<ActivityEvent>, OutputError> {
        let Some(event) = read_event::<Event>(line)? else {
            return Ok(Vec::new());
        };
        Ok(match event {
            Event::ThreadStarted { thread_id } => {
                self.progress.note_session(line, &thread_id)?;
                vec![ActivityEvent::Info(format!("session started {thread_id}"))]
            }
            Event::ItemStarted { item } => Self::started(item),
            Event::ItemCompleted { item } => self.completed(item),
            Event::TurnCompleted => {
                self.progress.stream = StreamOutcome::Succeeded;
                self.progress.final_message = self.last_message.take().filter(|text| !text.trim().is_empty());
                Vec::new()
            }
            Event::TurnFailed { error } => {
                self.progress.stream = StreamOutcome::Failed;
                self.failure(error.message)
            }
            Event::Error { message } => self.failure(message),
            Event::Other => Vec::new(),
        })
    }

    fn finish(self: Box<Self>, stderr: &str, tail_lines: NonZeroU32) -> RunOutput {
        self.progress.finish(stderr, tail_lines)
    }
}
