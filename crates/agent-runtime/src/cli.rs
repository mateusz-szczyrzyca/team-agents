//! The adapters of the four model CLIs — Claude Code, Codex, Antigravity (`agy`) and opencode — and what they share.
//!
//! Each adapter implements [`CliAdapter`] of `agent-core`: it is pure, so the runtime starts what
//! [`CliAdapter::build_run`] returns and feeds the CLI's stdout into the
//! [`OutputParser`](agent_core::cli::OutputParser) of [`CliAdapter::parser`]. A CLI's quirks stay in its module: its
//! flags, how it takes an MCP server, how it names a session, what its stream looks like. These rules hold for all
//! four, and the run engine relies on them:
//!
//! - **MCP servers.** `RunRequest::mcp_servers` carries the complete command of every server the run is given. The one
//!   thing an adapter adds is Serena's `--context <client>`, which depends on the CLI. A server that is not given is
//!   not configured: the run gets none. What an adapter puts inside the worktree (`.agents/mcp_config.json`) or
//!   Serena's folder there (`.serena/`) is listed in
//!   [`RunSpec::worktree_excludes`](agent_core::cli::RunSpec::worktree_excludes). The runtime creates the directories
//!   of a [`RunFile`] before it writes it.
//! - **The final message** is what the CLI itself marks as its answer: Claude's and Antigravity's `result`, Codex's last
//!   `agent_message` of a turn that completed, opencode's last `text` part. A stream cut before that has none.
//! - **The session ID** is the latest one the stream names.
//! - **The error tail** is the CLI's stderr followed by the lines of the stream that report an error: the classifier of
//!   run outcomes reads only the tail (and the final message) to recognise a quota or a rate limit, and a CLI such as
//!   opencode reports the provider's refusal on stdout.
//! - **A line that cannot be read** is an [`OutputError`] and the parser goes on: `Json` for text that is not JSON (a
//!   cut line, a banner), `Unrecognized` for JSON without the fields the adapter needs, `SessionId` for an ID the
//!   harness refuses. An event of a type the adapter does not know is not an error: the CLIs add events and this
//!   code reads only the few it needs.
//! - **The activity** of a step is the one the old `agentbus normalize` showed. Token usage is not read: accounting is
//!   not part of this rewrite.

use std::collections::BTreeMap;
use std::num::NonZeroU32;
use std::path::Path;

use agent_core::cli::{
    ActivityEvent, BuildRunError, CliAdapter, ErrorTail, McpServerKind, OutputCause, OutputError, RunFile, RunOutput,
    RunRequest, SessionId, StreamOutcome,
};
use agent_core::config::{AbsolutePath, InvalidValue, ValueRule};
use agent_core::kinds::CliKind;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;

pub mod agy;
pub mod claude;
pub mod codex;
pub mod opencode;

/// The longest part of an agentbus call's arguments that an [`ActivityEvent::BusCall`] keeps, in characters.
const MAX_BUS_ARGUMENTS_CHARS: usize = 200;

/// The longest summary of an [`ActivityEvent::ToolCall`], in characters.
const MAX_TOOL_SUMMARY_CHARS: usize = 160;

/// The longest line of the stream that is kept for the error tail, in characters: a provider's error can carry a
/// whole response body.
const MAX_STREAM_ERROR_CHARS: usize = 2000;

/// How many lines of the stream that report an error a parser keeps: the latest ones.
const MAX_STREAM_ERROR_LINES: usize = 100;

/// The folder Serena creates in the worktree.
const SERENA_FOLDER: &str = ".serena/";

/// The prefixes of an agentbus tool's name in the CLIs' tool calls: Claude Code's, one of opencode's and two forms of
/// the others'.
const BUS_TOOL_PREFIXES: [&str; 4] = ["mcp__agentbus__", "mcp_agentbus_", "agentbus_", "agentbus."];

/// The names, in lower case, of the tools that run a shell command.
const COMMAND_TOOLS: [&str; 4] = ["bash", "execute", "run_command", "shell"];

/// The inputs a command can be in, first one present wins.
const COMMAND_KEYS: [&str; 3] = ["command", "CommandLine", "cmd"];

/// The inputs a file can be named in by a tool that changes one.
const EDIT_PATH_KEYS: [&str; 6] = ["file_path", "filePath", "path", "TargetFile", "AbsolutePath", "file"];

/// The inputs that say best what any other tool is asked to do, first one present wins.
const SUMMARY_KEYS: [&str; 9] = [
    "file_path",
    "filePath",
    "path",
    "pattern",
    "query",
    "url",
    "TargetFile",
    "AbsolutePath",
    "Query",
];

/// The adapter for `kind`.
#[must_use]
pub fn adapter_for(kind: CliKind) -> Box<dyn CliAdapter> {
    match kind {
        CliKind::Claude => Box::new(claude::ClaudeAdapter),
        CliKind::Codex => Box::new(codex::CodexAdapter),
        CliKind::Gemini => Box::new(agy::AgyAdapter),
        CliKind::Opencode => Box::new(opencode::OpencodeAdapter),
    }
}

/// An MCP server of a run with its command as texts, the way a CLI's configuration takes it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Server {
    /// Which server it is; its text is its name in the CLI's configuration.
    pub(crate) kind: McpServerKind,
    /// The program that starts it.
    pub(crate) program: String,
    /// Its arguments, with Serena's context added.
    pub(crate) args: Vec<String>,
}

impl Server {
    /// The server's name in the CLI's configuration.
    pub(crate) fn name(&self) -> &'static str {
        self.kind.into()
    }

    /// The program and its arguments as one list, the way opencode takes a command.
    pub(crate) fn words(&self) -> Vec<String> {
        std::iter::once(self.program.clone())
            .chain(self.args.iter().cloned())
            .collect()
    }
}

/// The MCP servers of `request` as texts. Serena gets `--context <serena_context>`: the client it is told it serves
/// decides which of its tools it leaves to the CLI.
///
/// # Errors
/// [`BuildRunError::Path`] when a program's path is not UTF-8.
pub(crate) fn servers(cli: CliKind, request: &RunRequest, serena_context: &str) -> Result<Vec<Server>, BuildRunError> {
    request
        .mcp_servers
        .iter()
        .map(|(&kind, command)| {
            let mut args = command.args().as_slice().to_vec();
            if kind == McpServerKind::Serena {
                args.extend(["--context".to_owned(), serena_context.to_owned()]);
            }
            Ok(Server {
                kind,
                program: path_text(cli, command.program().as_path())?.to_owned(),
                args,
            })
        })
        .collect()
}

/// The paths, relative to the worktree, that `servers` make and the harness keeps out of git.
pub(crate) fn serena_excludes(servers: &[Server]) -> Vec<String> {
    servers
        .iter()
        .filter(|server| server.kind == McpServerKind::Serena)
        .map(|_| SERENA_FOLDER.to_owned())
        .collect()
}

/// The text of `path`.
///
/// Every path of a request was a text once; this is the proof for the compiler.
///
/// # Errors
/// [`BuildRunError::Path`] when `path` is not UTF-8: the invalid bytes show as `U+FFFD` in the error.
pub(crate) fn path_text(cli: CliKind, path: &Path) -> Result<&str, BuildRunError> {
    path.to_str().ok_or_else(|| BuildRunError::Path {
        kind: cli,
        source: InvalidValue {
            what: "path",
            value: path.to_string_lossy().into_owned(),
            rule: ValueRule::ForbiddenChar(char::REPLACEMENT_CHARACTER),
        },
    })
}

/// `relative` under `base`.
///
/// # Errors
/// [`BuildRunError::Path`] when `base` is not UTF-8.
pub(crate) fn path_under(cli: CliKind, base: &AbsolutePath, relative: &str) -> Result<AbsolutePath, BuildRunError> {
    let joined = path_text(cli, &base.as_path().join(relative))?.to_owned();
    AbsolutePath::try_from(joined).map_err(|source| BuildRunError::Path { kind: cli, source })
}

/// The file `path` holding `value` as JSON.
///
/// # Errors
/// [`BuildRunError::Json`] when `value` cannot be serialized.
pub(crate) fn json_file(cli: CliKind, path: AbsolutePath, value: &impl Serialize) -> Result<RunFile, BuildRunError> {
    match serde_json::to_string_pretty(value) {
        Ok(mut content) => {
            content.push('\n');
            Ok(RunFile { path, content })
        }
        Err(source) => Err(BuildRunError::Json {
            kind: cli,
            path,
            source,
        }),
    }
}

/// One server in the `mcpServers` of Claude Code's and Antigravity's configuration files.
#[derive(Debug, Serialize)]
struct StdioServer<'a> {
    /// The program that starts the server.
    command: &'a str,
    /// Its arguments.
    args: &'a [String],
}

/// The `{"mcpServers": {...}}` file of Claude Code and Antigravity.
#[derive(Debug, Serialize)]
struct McpFile<'a> {
    /// The servers by name; the empty object for a run that gets none.
    #[serde(rename = "mcpServers")]
    mcp_servers: BTreeMap<&'static str, StdioServer<'a>>,
}

/// The MCP configuration file of `servers` at `path`.
///
/// # Errors
/// [`BuildRunError::Json`] when the file cannot be serialized.
pub(crate) fn mcp_file(cli: CliKind, path: AbsolutePath, servers: &[Server]) -> Result<RunFile, BuildRunError> {
    let mcp_servers = servers
        .iter()
        .map(|server| {
            (
                server.name(),
                StdioServer {
                    command: &server.program,
                    args: &server.args,
                },
            )
        })
        .collect();
    json_file(cli, path, &McpFile { mcp_servers })
}

/// `text` cut after `max_chars` characters, with `…` after the cut; a shorter text as it is.
pub(crate) fn clip(text: &str, max_chars: usize) -> String {
    let mut chars = text.chars();
    let mut kept: String = chars.by_ref().take(max_chars).collect();
    if chars.next().is_some() {
        kept.push('…');
    }
    kept
}

/// The first of `keys` that `input` has as a non-empty text.
fn first_text<'a>(input: &'a Value, keys: &[&str]) -> Option<&'a str> {
    keys.iter()
        .find_map(|key| input.get(*key).and_then(Value::as_str).filter(|text| !text.is_empty()))
}

/// The tool of agentbus that the name `name` of a tool call stands for, if it is one.
fn bus_tool(name: &str) -> Option<&str> {
    BUS_TOOL_PREFIXES.iter().find_map(|prefix| {
        let (head, tool) = name.split_at_checked(prefix.len())?;
        head.eq_ignore_ascii_case(prefix).then_some(tool)
    })
}

/// The step that a call of the tool `name` with `input` is: a command, a file change, a call of agentbus, or any other
/// tool.
pub(crate) fn tool_activity(name: &str, input: &Value) -> ActivityEvent {
    let lower = name.to_ascii_lowercase();
    if let Some(tool) = bus_tool(name) {
        return ActivityEvent::BusCall {
            tool: tool.to_owned(),
            arguments: clip(&input.to_string(), MAX_BUS_ARGUMENTS_CHARS),
        };
    }
    let runs_commands = COMMAND_TOOLS.contains(&lower.as_str()) || lower.contains("command");
    if runs_commands && let Some(command) = first_text(input, &COMMAND_KEYS) {
        return ActivityEvent::Command(command.to_owned());
    }
    let changes_files = ["edit", "write", "replace"].iter().any(|word| lower.contains(word));
    if changes_files && let Some(path) = first_text(input, &EDIT_PATH_KEYS) {
        return ActivityEvent::FileEdit { path: path.to_owned() };
    }
    let summary = first_text(input, &SUMMARY_KEYS).map_or_else(|| input.to_string(), str::to_owned);
    ActivityEvent::ToolCall {
        name: name.to_owned(),
        summary: clip(&summary, MAX_TOOL_SUMMARY_CHARS),
    }
}

/// A message of the agent as a step: its text without the white space around it; no step for a blank one.
pub(crate) fn text_event(text: &str) -> Option<ActivityEvent> {
    let text = text.trim();
    (!text.is_empty()).then(|| ActivityEvent::Text(text.to_owned()))
}

/// The error for the line `line` that serde refused with `source`: [`OutputCause::Json`] for text that is not JSON,
/// [`OutputCause::Unrecognized`] for JSON that is not an event of the shape the adapter reads (no type, a missing or a
/// mistyped field). The line is in the error, which is all the diagnosis a reader needs, so the serializer's own text
/// is not kept for the second.
pub(crate) fn refused(line: &str, source: serde_json::Error) -> OutputError {
    if source.is_data() {
        OutputError::new(line, OutputCause::Unrecognized)
    } else {
        OutputError::new(line, source)
    }
}

/// The line of the output `line` as the event type `T`: nothing for a blank line.
///
/// # Errors
/// [`OutputError`] as [`refused`] makes it.
pub(crate) fn read_event<T: DeserializeOwned>(line: &str) -> Result<Option<T>, OutputError> {
    if line.trim().is_empty() {
        return Ok(None);
    }
    serde_json::from_str(line)
        .map(Some)
        .map_err(|source| refused(line, source))
}

/// What a parser has learned of a run so far; the part the four CLIs have in common.
#[derive(Debug)]
pub(crate) struct Progress {
    /// The latest session ID the stream named.
    session_id: Option<SessionId>,
    /// The agent's final message, once the stream has shown it.
    pub(crate) final_message: Option<String>,
    /// How the stream says the run ended; incomplete until it says so.
    pub(crate) stream: StreamOutcome,
    /// The latest lines of the stream that report an error, each cut, oldest first.
    error_lines: Vec<String>,
}

impl Progress {
    /// The progress of a run that has shown nothing.
    pub(crate) fn new() -> Self {
        Self {
            session_id: None,
            final_message: None,
            stream: StreamOutcome::Incomplete,
            error_lines: Vec::new(),
        }
    }

    /// Takes `id`, which the line `line` named, as the session of the run.
    ///
    /// # Errors
    /// [`OutputError`] with [`OutputCause::SessionId`] when `id` is not a session ID; the run keeps its earlier one.
    pub(crate) fn note_session(&mut self, line: &str, id: &str) -> Result<(), OutputError> {
        if self.session_id.as_ref().is_some_and(|known| known.as_str() == id) {
            return Ok(());
        }
        let parsed = id.parse().map_err(|source| OutputError::new(line, source))?;
        self.session_id = Some(parsed);
        Ok(())
    }

    /// Keeps `text`, a line of the stream that reports an error, for the error tail.
    pub(crate) fn note_error_line(&mut self, text: &str) {
        if self.error_lines.len() == MAX_STREAM_ERROR_LINES {
            self.error_lines.remove(0);
        }
        self.error_lines.push(clip(text.trim(), MAX_STREAM_ERROR_CHARS));
    }

    /// The end of the run: what the stream showed, and the last `tail_lines` lines of `stderr` and of the stream's
    /// error lines.
    pub(crate) fn finish(self, stderr: &str, tail_lines: NonZeroU32) -> RunOutput {
        let mut errors = stderr.to_owned();
        for line in &self.error_lines {
            errors.push('\n');
            errors.push_str(line);
        }
        RunOutput {
            final_message: self.final_message,
            session_id: self.session_id,
            stream: self.stream,
            error_tail: ErrorTail::from_text(&errors, tail_lines),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::ffi::OsStr;
    use std::os::unix::ffi::OsStrExt;

    use agent_core::cli::RunMode;
    use agent_core::config::{CliArgs, CommandLine};
    use serde_json::json;

    use super::*;

    fn command(words: &[&str]) -> CommandLine {
        CommandLine::try_from(words.iter().map(|word| (*word).to_owned()).collect::<Vec<_>>()).unwrap()
    }

    fn request_with(servers: &[(McpServerKind, CommandLine)]) -> RunRequest {
        RunRequest {
            program: "/opt/bin/cli".parse().unwrap(),
            model: None,
            extra_args: CliArgs::default(),
            mode: RunMode::New {
                session_id: "s-1".parse().unwrap(),
            },
            prompt: String::new(),
            working_dir: "/work/tree".parse().unwrap(),
            run_dir: "/work/run".parse().unwrap(),
            mcp_servers: servers.iter().cloned().collect(),
        }
    }

    #[test]
    fn every_kind_has_its_own_adapter() {
        for &kind in CliKind::ALL {
            assert_eq!(adapter_for(kind).kind(), kind);
        }
    }

    #[test]
    fn clip_keeps_a_short_text_and_cuts_a_long_one_after_whole_characters() {
        assert_eq!(clip("abc", 3), "abc");
        assert_eq!(clip("abcd", 3), "abc…");
        assert_eq!(clip("", 3), "");
        assert_eq!(clip("żółćżółć", 4), "żółć…");
        assert_eq!(clip("日本語日本語", 2), "日本…");
        assert_eq!(clip("abc", 0), "…");
    }

    #[test]
    fn a_path_that_is_not_utf8_is_refused_with_its_rule() {
        let path = Path::new(OsStr::from_bytes(b"/work/\xff/tree"));

        let err = path_text(CliKind::Claude, path).unwrap_err();

        assert!(matches!(
            &err,
            BuildRunError::Path { kind: CliKind::Claude, source } if source.rule == ValueRule::ForbiddenChar('\u{fffd}')
        ));
        assert_eq!(
            path_text(CliKind::Claude, Path::new("/work/tree")).unwrap(),
            "/work/tree"
        );
    }

    #[test]
    fn a_path_under_a_base_is_the_base_and_the_relative_path() {
        let base: AbsolutePath = "/work/tree".parse().unwrap();

        let path = path_under(CliKind::Gemini, &base, ".agents/mcp_config.json").unwrap();

        assert_eq!(path.as_path().to_str(), Some("/work/tree/.agents/mcp_config.json"));
    }

    #[test]
    fn serena_gets_the_context_of_the_cli_and_agentbus_stays_as_it_is() {
        let request = request_with(&[
            (
                McpServerKind::Agentbus,
                command(&["/opt/bin/agentbus", "serve", "--agent", "a"]),
            ),
            (McpServerKind::Serena, command(&["serena", "start-mcp-server"])),
        ]);

        let servers = servers(CliKind::Codex, &request, "codex").unwrap();

        assert_eq!(
            servers,
            [
                Server {
                    kind: McpServerKind::Agentbus,
                    program: "/opt/bin/agentbus".to_owned(),
                    args: vec!["serve".to_owned(), "--agent".to_owned(), "a".to_owned()],
                },
                Server {
                    kind: McpServerKind::Serena,
                    program: "serena".to_owned(),
                    args: vec![
                        "start-mcp-server".to_owned(),
                        "--context".to_owned(),
                        "codex".to_owned()
                    ],
                },
            ]
        );
        assert_eq!(serena_excludes(&servers), [".serena/"]);
        assert_eq!(serena_excludes(&servers[..1]), Vec::<String>::new());
        assert_eq!(servers[1].words(), ["serena", "start-mcp-server", "--context", "codex"]);
        assert_eq!(servers[1].name(), "serena");
    }

    #[test]
    fn a_request_without_servers_has_none() {
        let servers = servers(CliKind::Claude, &request_with(&[]), "claude-code").unwrap();

        assert_eq!(servers, []);
    }

    #[test]
    fn tool_calls_are_classified_by_their_name_and_input() {
        let bus = |tool: &str, arguments: &str| ActivityEvent::BusCall {
            tool: tool.to_owned(),
            arguments: arguments.to_owned(),
        };
        let call = |name: &str, summary: &str| ActivityEvent::ToolCall {
            name: name.to_owned(),
            summary: summary.to_owned(),
        };
        let edit = |path: &str| ActivityEvent::FileEdit { path: path.to_owned() };
        let run = |command: &str| ActivityEvent::Command(command.to_owned());
        let table = [
            ("Bash", json!({"command": "ls"}), run("ls")),
            ("bash", json!({"command": "ls -l"}), run("ls -l")),
            ("shell", json!({"command": "x"}), run("x")),
            ("run_command", json!({"CommandLine": "y"}), run("y")),
            ("execute", json!({"cmd": "z"}), run("z")),
            ("send_command", json!({"command": "w"}), run("w")),
            ("shell", json!({}), call("shell", "{}")),
            ("Edit", json!({"file_path": "/a"}), edit("/a")),
            ("MultiEdit", json!({"file_path": "/b"}), edit("/b")),
            ("Write", json!({"file_path": "/c"}), edit("/c")),
            ("write_to_file", json!({"TargetFile": "/d"}), edit("/d")),
            ("edit", json!({"path": "/e"}), edit("/e")),
            ("Edit", json!({}), call("Edit", "{}")),
            ("Read", json!({"file_path": "/f"}), call("Read", "/f")),
            ("Grep", json!({"pattern": "foo"}), call("Grep", "foo")),
            (
                "WebFetch",
                json!({"url": "https://example.com"}),
                call("WebFetch", "https://example.com"),
            ),
            ("search", json!({"query": "q"}), call("search", "q")),
            ("Task", json!({"prompt": "p"}), call("Task", r#"{"prompt":"p"}"#)),
            ("Task", Value::Null, call("Task", "null")),
            (
                "mcp__agentbus__board_add",
                json!({"title": "T"}),
                bus("board_add", r#"{"title":"T"}"#),
            ),
            ("agentbus.read_messages", json!({}), bus("read_messages", "{}")),
            (
                "AgentBus_send_message",
                json!({"to": "x"}),
                bus("send_message", r#"{"to":"x"}"#),
            ),
            ("agentbus", json!({}), call("agentbus", "{}")),
            (
                "mcp__serena__find_symbol",
                json!({"name_path": "Foo"}),
                call("mcp__serena__find_symbol", r#"{"name_path":"Foo"}"#),
            ),
            (
                "replace_symbol_body",
                json!({"relative_path": "a.rs"}),
                call("replace_symbol_body", r#"{"relative_path":"a.rs"}"#),
            ),
        ];

        for (name, input, expected) in table {
            assert_eq!(tool_activity(name, &input), expected, "{name} {input}");
        }
    }

    #[test]
    fn a_long_argument_summary_is_cut_for_a_bus_call_at_200_and_for_another_tool_at_160() {
        let long = json!({ "title": "x".repeat(300) });

        let bus = tool_activity("mcp__agentbus__board_add", &long);
        let other = tool_activity("Task", &long);

        let (ActivityEvent::BusCall { arguments, .. }, ActivityEvent::ToolCall { summary, .. }) = (bus, other) else {
            panic!("wrong kinds");
        };
        assert_eq!(arguments.chars().count(), 201);
        assert!(arguments.ends_with('…'));
        assert_eq!(summary.chars().count(), 161);
        assert!(summary.ends_with('…'));
    }

    #[test]
    fn a_value_that_cannot_be_serialized_is_a_json_error_naming_the_file() {
        struct Refuses;
        impl Serialize for Refuses {
            fn serialize<S: serde::Serializer>(&self, _: S) -> Result<S::Ok, S::Error> {
                Err(serde::ser::Error::custom("refused"))
            }
        }
        let path: AbsolutePath = "/work/run/x.json".parse().unwrap();

        let err = json_file(CliKind::Opencode, path.clone(), &Refuses).unwrap_err();

        assert!(matches!(
            &err,
            BuildRunError::Json { kind: CliKind::Opencode, path: at, .. } if *at == path
        ));
        let file = json_file(CliKind::Opencode, path.clone(), &json!({"a": 1})).unwrap();
        assert_eq!(file.path, path);
        assert_eq!(file.content, "{\n  \"a\": 1\n}\n");
    }

    #[test]
    fn a_message_is_trimmed_and_a_blank_one_is_no_step() {
        assert_eq!(text_event("  hi\n"), Some(ActivityEvent::Text("hi".to_owned())));
        assert_eq!(text_event(" \n\t"), None);
        assert_eq!(text_event(""), None);
    }

    #[test]
    fn the_progress_keeps_the_latest_session_and_refuses_a_bad_one_without_losing_the_old() {
        let mut progress = Progress::new();

        progress.note_session("l1", "s-1").unwrap();
        progress.note_session("l2", "s-1").unwrap();
        let err = progress.note_session("l3", "a b").unwrap_err();
        progress.note_session("l4", "s-2").unwrap();

        assert_eq!(err.line(), "l3");
        assert!(matches!(err.cause(), OutputCause::SessionId(_)));
        let output = progress.finish("", NonZeroU32::MIN);
        assert_eq!(output.session_id.map(|id| id.to_string()), Some("s-2".to_owned()));
    }

    #[test]
    fn the_progress_ends_with_stderr_and_then_the_error_lines_and_keeps_the_last_ones() {
        let mut progress = Progress::new();
        progress.note_error_line("  first error  ");
        progress.note_error_line("second error");

        let all = progress.finish("warn\n\nboom\n", NonZeroU32::new(10).unwrap());

        assert_eq!(all.error_tail.lines(), ["warn", "boom", "first error", "second error"]);
        let mut progress = Progress::new();
        progress.note_error_line("one");
        progress.note_error_line("two");
        let last = progress.finish("boom", NonZeroU32::new(2).unwrap());
        assert_eq!(last.error_tail.lines(), ["one", "two"]);
    }

    #[test]
    fn the_progress_keeps_a_bounded_number_of_error_lines_each_of_bounded_length() {
        let mut progress = Progress::new();
        for number in 0..(MAX_STREAM_ERROR_LINES + 5) {
            progress.note_error_line(&number.to_string());
        }
        progress.note_error_line(&"x".repeat(MAX_STREAM_ERROR_CHARS + 50));

        let output = progress.finish("", NonZeroU32::new(1000).unwrap());

        let lines = output.error_tail.lines();
        assert_eq!(lines.len(), MAX_STREAM_ERROR_LINES);
        assert_eq!(lines[0], "6");
        assert_eq!(
            lines[MAX_STREAM_ERROR_LINES - 1].chars().count(),
            MAX_STREAM_ERROR_CHARS + 1
        );
    }

    #[test]
    fn an_event_is_read_by_its_type_and_a_line_that_is_not_one_says_why() {
        #[derive(serde::Deserialize, Debug, PartialEq)]
        #[serde(tag = "type", rename_all = "snake_case")]
        enum Event {
            Ping {
                n: u32,
            },
            #[serde(other)]
            Other,
        }

        assert_eq!(
            read_event::<Event>(r#"{"type":"ping","n":3}"#).unwrap(),
            Some(Event::Ping { n: 3 })
        );
        assert_eq!(read_event::<Event>(r#"{"type":"later"}"#).unwrap(), Some(Event::Other));
        assert_eq!(read_event::<Event>("  ").unwrap(), None);
        let cause = |line: &str| read_event::<Event>(line).unwrap_err();
        assert!(matches!(cause("{").cause(), OutputCause::Json(_)));
        assert!(matches!(cause("not json").cause(), OutputCause::Json(_)));
        assert!(matches!(cause(r#"{"type":"ping"}"#).cause(), OutputCause::Unrecognized));
        assert!(matches!(cause(r#"{"n":1}"#).cause(), OutputCause::Unrecognized));
        assert!(matches!(cause("[1]").cause(), OutputCause::Unrecognized));
    }
}
