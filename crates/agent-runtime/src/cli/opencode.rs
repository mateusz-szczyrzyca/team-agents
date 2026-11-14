//! The opencode adapter: `opencode run --format json`.
//!
//! opencode reads its configuration from the environment (`OPENCODE_CONFIG_CONTENT`), so a run's MCP servers need no
//! file in the worktree; the same text is kept in the run's directory for whoever looks into the run. The
//! configuration also tells opencode how to reach `OpenRouter` and `DeepInfra`, with the API keys left as `{env:NAME}`
//! references that the runtime's own environment answers. A model is named `provider/model`; a model of one of those
//! two providers that opencode's catalogue may not know is registered under its provider, so that opencode accepts
//! it. Shell commands may run for ten minutes. A resumed run names the session (`--session`); the session of a new run
//! is the `sessionID` that every event of the stream carries.
//!
//! The stream is one JSON object a line: `step_start`, `text`, `tool_use`, `step_finish` and `error`. opencode has no
//! event for the end of a run: a run that finished ends on a `step_finish` with the reason `stop` or, as most recorded
//! runs do, on its last `text`; a text that a tool call, a finished step or a new step follows is not the end. The
//! final message is the last text.

use std::collections::BTreeMap;
use std::num::NonZeroU32;

use agent_core::cli::{
    ActivityEvent, BuildRunError, CliAdapter, OutputError, OutputParser, RunMode, RunOutput, RunRequest, RunSpec,
    StreamOutcome,
};
use agent_core::config::{EnvName, ModelName};
use agent_core::kinds::CliKind;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{
    Progress, Server, clip, json_file, path_under, read_event, serena_excludes, servers, text_event, tool_activity,
};

/// How long a shell command of the agent may run, in milliseconds.
const BASH_TIMEOUT_MS: &str = "600000";

/// Serena's name for opencode as its client: opencode is an IDE-like client.
const SERENA_CONTEXT: &str = "ide";

/// The name of the file of the run that keeps the configuration.
const CONFIG_FILE: &str = "opencode-config.json";

/// The longest part of an error's JSON that an [`ActivityEvent::Error`] keeps, in characters.
const MAX_ERROR_CHARS: usize = 300;

/// The adapter of opencode.
#[derive(Debug, Clone, Copy)]
pub struct OpencodeAdapter;

/// A provider opencode is told how to reach.
#[derive(Debug, Clone, Copy)]
struct KnownProvider {
    /// Its key in opencode's configuration, and the first part of a model's name.
    key: &'static str,
    /// The package that speaks to it when opencode's catalogue does not know it.
    npm: Option<&'static str>,
    /// Its display name.
    name: Option<&'static str>,
    /// Its API when it is not in the catalogue.
    base_url: Option<&'static str>,
    /// The environment variable that holds its API key.
    api_key_env: &'static str,
}

/// The providers the harness's agents use.
const PROVIDERS: [KnownProvider; 2] = [
    KnownProvider {
        key: "openrouter",
        npm: None,
        name: None,
        base_url: None,
        api_key_env: "OPENROUTER_API_KEY",
    },
    KnownProvider {
        key: "deepinfra",
        npm: Some("@ai-sdk/openai-compatible"),
        name: Some("DeepInfra"),
        base_url: Some("https://api.deepinfra.com/v1/openai"),
        api_key_env: "DEEPINFRA_API_KEY",
    },
];

/// opencode's configuration of a run.
#[derive(Debug, Serialize)]
struct Config {
    /// The schema the file follows.
    #[serde(rename = "$schema")]
    schema: &'static str,
    /// Whether opencode updates itself; never.
    autoupdate: bool,
    /// The MCP servers by name.
    mcp: BTreeMap<&'static str, McpServer>,
    /// The providers by key.
    provider: BTreeMap<&'static str, Provider>,
}

/// A local MCP server in the configuration.
#[derive(Debug, Serialize)]
struct McpServer {
    /// How the server is reached.
    #[serde(rename = "type")]
    kind: &'static str,
    /// The program and its arguments.
    command: Vec<String>,
    /// Whether opencode starts it.
    enabled: bool,
}

/// A provider in the configuration.
#[derive(Debug, Serialize)]
struct Provider {
    /// The package that speaks to it.
    #[serde(skip_serializing_if = "Option::is_none")]
    npm: Option<&'static str>,
    /// Its display name.
    #[serde(skip_serializing_if = "Option::is_none")]
    name: Option<&'static str>,
    /// How to reach it.
    options: ProviderOptions,
    /// The models registered with it.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    models: BTreeMap<String, ModelEntry>,
}

/// How to reach a provider.
#[derive(Debug, Serialize)]
struct ProviderOptions {
    /// Its API, where opencode does not know it.
    #[serde(rename = "baseURL", skip_serializing_if = "Option::is_none")]
    base_url: Option<&'static str>,
    /// The key, as a reference to an environment variable.
    #[serde(rename = "apiKey")]
    api_key: String,
}

/// A model registered with a provider.
#[derive(Debug, Serialize)]
struct ModelEntry {
    /// The model's display name.
    name: String,
}

/// The provider `known` for a run of `model`: the model is registered under it when it is one of its own.
fn provider(known: &KnownProvider, model: Option<&str>) -> Provider {
    let registered = model
        .and_then(|model| model.strip_prefix(known.key))
        .and_then(|rest| rest.strip_prefix('/'))
        .filter(|name| !name.is_empty());
    Provider {
        npm: known.npm,
        name: known.name,
        options: ProviderOptions {
            base_url: known.base_url,
            api_key: format!("{{env:{}}}", known.api_key_env),
        },
        models: registered
            .map(|name| (name.to_owned(), ModelEntry { name: name.to_owned() }))
            .into_iter()
            .collect(),
    }
}

/// The configuration of a run of `model` that gets `servers`.
fn config(servers: &[Server], model: Option<&str>) -> Config {
    Config {
        schema: "https://opencode.ai/config.json",
        autoupdate: false,
        mcp: servers
            .iter()
            .map(|server| {
                (
                    server.name(),
                    McpServer {
                        kind: "local",
                        command: server.words(),
                        enabled: true,
                    },
                )
            })
            .collect(),
        provider: PROVIDERS
            .iter()
            .map(|known| (known.key, provider(known, model)))
            .collect(),
    }
}

/// The environment variable `literal`.
#[expect(
    clippy::expect_used,
    reason = "the names are literals of this file and every opencode run built in the tests uses all three"
)]
fn env_name(literal: &'static str) -> EnvName {
    literal
        .parse()
        .expect("an environment variable name written in this file is valid")
}

impl CliAdapter for OpencodeAdapter {
    fn kind(&self) -> CliKind {
        CliKind::Opencode
    }

    fn build_run(&self, request: &RunRequest) -> Result<RunSpec, BuildRunError> {
        let servers = servers(CliKind::Opencode, request, SERENA_CONTEXT)?;
        let config = config(&servers, request.model.as_ref().map(ModelName::as_str));
        let file = json_file(
            CliKind::Opencode,
            path_under(CliKind::Opencode, &request.run_dir, CONFIG_FILE)?,
            &config,
        )?;

        let mut args: Vec<String> = ["run", "--standalone", "--format", "json", "--auto"]
            .map(str::to_owned)
            .into();
        if let Some(model) = &request.model {
            args.extend(["-m".to_owned(), model.as_str().to_owned()]);
        }
        args.extend(request.extra_args.as_slice().iter().cloned());
        if let RunMode::Resume { session_id } = &request.mode {
            args.extend(["--session".to_owned(), session_id.to_string()]);
        }
        args.extend(["--".to_owned(), request.prompt.clone()]);

        Ok(RunSpec {
            program: request.program.clone(),
            args,
            env: BTreeMap::from([
                (env_name("OPENCODE_CONFIG_CONTENT"), file.content.trim_end().to_owned()),
                (env_name("OPENCODE_DISABLE_AUTOUPDATE"), "1".to_owned()),
                (
                    env_name("OPENCODE_EXPERIMENTAL_BASH_DEFAULT_TIMEOUT_MS"),
                    BASH_TIMEOUT_MS.to_owned(),
                ),
            ]),
            working_dir: request.working_dir.clone(),
            files: vec![file],
            worktree_excludes: serena_excludes(&servers),
        })
    }

    fn parser(&self) -> Box<dyn OutputParser> {
        Box::new(OpencodeParser {
            ends_on_text: false,
            progress: Progress::new(),
        })
    }
}

/// A line of the stream: its session and the event.
#[derive(Debug, Deserialize)]
struct Line {
    /// The session of the run; every event carries it.
    #[serde(rename = "sessionID")]
    session_id: Option<String>,
    /// The event, by its `type`.
    #[serde(flatten)]
    event: Event,
}

/// An event of the stream.
#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum Event {
    /// A step starts.
    StepStart,
    /// The agent wrote something.
    Text {
        /// The text.
        part: TextPart,
    },
    /// The agent called a tool.
    #[serde(alias = "tool")]
    ToolUse {
        /// The call.
        part: ToolPart,
    },
    /// A step is over.
    StepFinish {
        /// How it ended.
        part: FinishPart,
    },
    /// The run failed.
    Error {
        /// What the provider or opencode said; its shape varies.
        #[serde(default)]
        error: Value,
    },
    /// Any other event.
    #[serde(other)]
    Other,
}

/// A text of the agent.
#[derive(Debug, Deserialize)]
struct TextPart {
    /// What it says.
    text: Option<String>,
}

/// A tool call.
#[derive(Debug, Deserialize)]
struct ToolPart {
    /// The tool: `shell`, `edit`, `read`, `execute`, ...
    tool: String,
    /// The call and its outcome.
    state: Option<ToolState>,
}

/// The state of a tool call.
#[derive(Debug, Deserialize)]
struct ToolState {
    /// How the call went.
    status: Option<ToolStatus>,
    /// What the tool was called with.
    #[serde(default)]
    input: Value,
    /// Why the call failed.
    error: Option<String>,
}

/// How a tool call went.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ToolStatus {
    /// It failed.
    Error,
    /// Any other state.
    #[serde(other)]
    Other,
}

/// How a step ended.
#[derive(Debug, Deserialize)]
struct FinishPart {
    /// Why the step ended.
    reason: Option<FinishReason>,
}

/// Why a step ended.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "kebab-case")]
enum FinishReason {
    /// The model has said all it had to say: the run is over.
    Stop,
    /// The step failed.
    Error,
    /// Any other: `tool-calls` goes on with another step.
    #[serde(other)]
    Other,
}

/// What an error event says: the provider's message, else the error's name, else the error itself.
fn error_message(error: &Value) -> String {
    let text = |value: Option<&Value>| {
        value
            .and_then(Value::as_str)
            .filter(|text| !text.is_empty())
            .map(str::to_owned)
    };
    text(error.pointer("/data/message"))
        .or_else(|| text(error.get("message")))
        .or_else(|| text(error.get("name")))
        .unwrap_or_else(|| clip(&error.to_string(), MAX_ERROR_CHARS))
}

/// The parser of one opencode run.
#[derive(Debug)]
struct OpencodeParser {
    /// What the stream has shown.
    progress: Progress,
    /// Whether the latest step of the stream is a text of the agent: no tool call, finished step or new step has
    /// followed it.
    ends_on_text: bool,
}

impl OpencodeParser {
    /// The steps a tool call shows.
    fn tool(part: ToolPart) -> Vec<ActivityEvent> {
        let Some(state) = part.state else {
            return vec![tool_activity(&part.tool, &Value::Null)];
        };
        let mut events = vec![tool_activity(&part.tool, &state.input)];
        if matches!(state.status, Some(ToolStatus::Error)) {
            let message = state.error.unwrap_or_else(|| "a tool call failed".to_owned());
            events.push(ActivityEvent::Error(message));
        }
        events
    }
}

impl OutputParser for OpencodeParser {
    fn session(&self) -> Option<&agent_core::cli::SessionId> {
        self.progress.session_id.as_ref()
    }

    fn push_line(&mut self, line: &str) -> Result<Vec<ActivityEvent>, OutputError> {
        let Some(Line { session_id, event }) = read_event::<Line>(line)? else {
            return Ok(Vec::new());
        };
        // An error event of a refused request carries an empty ID: it names no session, and its error is the news.
        if let Some(id) = session_id.filter(|id| !id.is_empty()) {
            self.progress.note_session(line, &id)?;
        }
        Ok(match event {
            Event::Text { part } => {
                let text = part.text.unwrap_or_default();
                let event = text_event(&text);
                if event.is_some() {
                    self.progress.final_message = Some(text);
                    self.ends_on_text = true;
                }
                event.into_iter().collect()
            }
            Event::ToolUse { part } => {
                self.ends_on_text = false;
                Self::tool(part)
            }
            Event::StepStart => {
                self.ends_on_text = false;
                Vec::new()
            }
            Event::StepFinish { part } => {
                self.ends_on_text = false;
                match part.reason {
                    Some(FinishReason::Stop) => self.progress.stream = StreamOutcome::Succeeded,
                    Some(FinishReason::Error) => self.progress.stream = StreamOutcome::Failed,
                    Some(FinishReason::Other) | None => {}
                }
                Vec::new()
            }
            Event::Error { error } => {
                // The provider's status and kind sit next to its message, and the classifier of run outcomes looks
                // for them in the error tail: the whole line goes there.
                self.progress.note_error_line(line);
                self.progress.stream = StreamOutcome::Failed;
                vec![ActivityEvent::Error(error_message(&error))]
            }
            Event::Other => Vec::new(),
        })
    }

    fn finish(self: Box<Self>, stderr: &str, tail_lines: NonZeroU32) -> RunOutput {
        let Self {
            mut progress,
            ends_on_text,
        } = *self;
        // The recorded runs of a finished agent end on its text: opencode prints a `step_finish` with the reason
        // `stop` only sometimes.
        if ends_on_text && progress.stream == StreamOutcome::Incomplete {
            progress.stream = StreamOutcome::Succeeded;
        }
        progress.finish(stderr, tail_lines)
    }
}
