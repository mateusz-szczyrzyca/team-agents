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
    RunUsage, StreamOutcome,
};
use agent_core::config::{EnvName, ModelName};
use agent_core::kinds::CliKind;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{
    CallUsage, Progress, Server, UsageTally, clip, json_file, path_under, read_event, serena_excludes, servers,
    text_event, tool_activity,
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
    /// The window opencode compacts a session by; absent leaves it what its catalogue knows of the model.
    #[serde(skip_serializing_if = "Option::is_none")]
    limit: Option<ModelLimit>,
}

/// The output limit written when the agent sets a `context` but no `output`: the published schema requires `output` next
/// to `context`, and the harness knows no output limit of a model. `0` is no limit of the model's: opencode replaces
/// what its catalogue knows of the model's output with it and requests its own 32000 tokens, so an agent whose
/// catalogue output matters sets `output`.
const UNKNOWN_OUTPUT_LIMIT: u64 = 0;

/// The limit of a model in opencode's configuration: its context window in tokens.
#[derive(Debug, Serialize)]
struct ModelLimit {
    /// The context window in tokens.
    context: u64,
    /// The most tokens the model writes: the agent's `output`, else [`UNKNOWN_OUTPUT_LIMIT`].
    output: u64,
}

/// The window the agent configured for its model: its context, and the output limit when it set one.
#[derive(Debug, Clone, Copy)]
struct Window {
    /// The context window in tokens, the agent's `context`.
    context: u64,
    /// The most tokens the model writes, the agent's `output`; `None` where it set none.
    output: Option<u64>,
}

/// The provider `known` for a run of `model`: the model is registered under it when it is one of its own.
fn provider(known: &KnownProvider, model: Option<&str>, window: Option<Window>) -> Provider {
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
            .map(|name| {
                (
                    name.to_owned(),
                    ModelEntry {
                        name: name.to_owned(),
                        limit: window.map(|window| ModelLimit {
                            context: window.context,
                            output: window.output.unwrap_or(UNKNOWN_OUTPUT_LIMIT),
                        }),
                    },
                )
            })
            .into_iter()
            .collect(),
    }
}

/// The configuration of a run of `model` that gets `servers`; a model registered under a provider gets `window`.
fn config(servers: &[Server], model: Option<&str>, window: Option<Window>) -> Config {
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
            .map(|known| (known.key, provider(known, model, window)))
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
        // An `output` without a `context` is no window: there is nothing to put it next to.
        let window = request.context.map(|context| Window {
            context,
            output: request.output,
        });
        let config = config(&servers, request.model.as_ref().map(ModelName::as_str), window);
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
            usage: UsageTally::default(),
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
    /// The step's ID; the usage of a step is told under it.
    id: Option<String>,
    /// The tokens the step used; absent when the stream does not show them.
    tokens: Option<StepTokens>,
    /// The step's cost in US dollars, as the CLI reports it.
    cost: Option<f64>,
}

/// The tokens of one step as opencode reports them. Each part is optional: a part that is absent is not told.
#[derive(Debug, Deserialize)]
struct StepTokens {
    /// Tokens of the input that was not read from the cache.
    input: Option<u64>,
    /// Tokens the model wrote.
    output: Option<u64>,
    /// The cache of the step: what was read from it and what was written into it.
    cache: Option<StepCache>,
}

/// The cache part of a step's tokens.
#[derive(Debug, Deserialize)]
struct StepCache {
    /// Tokens read from the cache.
    read: Option<u64>,
    /// Tokens written into the cache.
    write: Option<u64>,
}

impl FinishPart {
    /// The step as the tally keeps it: its context is the input with what the cache read and wrote; none unless all
    /// of those are told.
    fn call(&self) -> CallUsage {
        let context = self.tokens.as_ref().and_then(|tokens| {
            let cache = tokens.cache.as_ref()?;
            tokens.input?.checked_add(cache.read?)?.checked_add(cache.write?)
        });
        CallUsage {
            context,
            written: self.tokens.as_ref().and_then(|tokens| tokens.output),
            cost: self.cost,
        }
    }
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

/// Extracts a [`RefusalSummary`] from an opencode error event, when the error has the structured fields.
fn refusal_from_error(error: &Value) -> Option<agent_core::decide::outcome::RefusalSummary> {
    let error_type = error.get("type").and_then(Value::as_str).unwrap_or("");
    let reason = match error_type {
        "provider.quota" => agent_core::decide::outcome::PauseReason::Quota,
        _ => agent_core::decide::outcome::PauseReason::RateLimit,
    };
    let message = error
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_owned();
    let status = error
        .get("status")
        .map(|s| s.to_string())
        .unwrap_or_default();
    // The provider's code is inside `response.body` which is a JSON string.
    let code = error
        .pointer("/response/body")
        .and_then(Value::as_str)
        .and_then(|body| serde_json::from_str::<Value>(body).ok())
        .and_then(|body| body.pointer("/error/code").and_then(Value::as_str).map(str::to_owned))
        .unwrap_or_default();
    Some(agent_core::decide::outcome::RefusalSummary::from_fields(
        &status, &message, &code, reason,
    ))
}

/// The parser of one opencode run.
#[derive(Debug)]
struct OpencodeParser {
    /// What the stream has shown.
    progress: Progress,
    /// The steps of the run that report their usage, each counted once.
    usage: UsageTally,
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
                // A step without an ID has no name to count it under: its usage is not told.
                if let Some(id) = part.id.clone() {
                    self.usage.record(id, part.call());
                }
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
                if self.progress.refusal.is_none() {
                    self.progress.refusal = refusal_from_error(&error);
                }
                vec![ActivityEvent::Error(error_message(&error))]
            }
            Event::Other => Vec::new(),
        })
    }

    fn finish(self: Box<Self>, stderr: &str, tail_lines: NonZeroU32) -> RunOutput {
        let Self {
            mut progress,
            ends_on_text,
            ..
        } = *self;
        // The recorded runs of a finished agent end on its text: opencode prints a `step_finish` with the reason
        // `stop` only sometimes.
        if ends_on_text && progress.stream == StreamOutcome::Incomplete {
            progress.stream = StreamOutcome::Succeeded;
        }
        progress.finish(stderr, tail_lines)
    }

    fn usage(&self) -> RunUsage {
        self.usage.usage()
    }

    fn latest_context(&self) -> Option<u64> {
        self.usage.latest_context()
    }
}
