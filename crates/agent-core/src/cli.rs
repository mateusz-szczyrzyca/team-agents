//! The contract every model CLI adapter implements: build a run from a typed request, turn the CLI's output into
//! typed activity events, and extract the final message, the session ID and the error tail.
//!
//! The adapters (`claude`, `codex`, `agy`, `opencode`) are pure: they never spawn a process, read a file or look at
//! the clock. [`CliAdapter::build_run`] returns a [`RunSpec`] — what to start and which files to lay down first —
//! and the runtime carries it out; the runtime feeds the CLI's stdout line by line into an [`OutputParser`] and its
//! stderr at the end. So an adapter is tested with strings alone.

use std::collections::BTreeMap;
use std::error::Error;
use std::num::NonZeroU32;

use serde::{Deserialize, Serialize};

use crate::config::{AbsolutePath, CliArgs, CommandLine, EnvName, InvalidValue, ModelName, Program, ValueRule};
use crate::kinds::{CliKind, keyword_enum};

/// Longest session ID, in bytes: the bound of a model name.
const MAX_SESSION_ID_LEN: usize = 200;

/// Longest part of an unreadable line that an [`OutputError`] keeps, in characters.
const MAX_ERROR_LINE_CHARS: usize = 200;

/// The ID of a CLI's conversation, with which a run is resumed: 1 to 200 bytes without whitespace or control
/// characters. Refused, with the [`ValueRule`] it breaks: the empty text, a longer one, whitespace and control
/// characters.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct SessionId(String);

/// Checks that `text` is a session ID.
fn check_session_id(text: &str) -> Result<(), InvalidValue> {
    let refuse = |rule| InvalidValue {
        what: "session ID",
        value: text.to_owned(),
        rule,
    };
    if text.is_empty() {
        return Err(refuse(ValueRule::Empty));
    }
    if text.len() > MAX_SESSION_ID_LEN {
        return Err(refuse(ValueRule::TooLong {
            max: MAX_SESSION_ID_LEN,
        }));
    }
    match text.chars().find(|c| c.is_control() || c.is_whitespace()) {
        Some(c) => Err(refuse(ValueRule::ForbiddenChar(c))),
        None => Ok(()),
    }
}

impl SessionId {
    /// The ID's text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for SessionId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::str::FromStr for SessionId {
    type Err = InvalidValue;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        Ok(Self(text.to_owned()))
    }
}

impl TryFrom<String> for SessionId {
    type Error = InvalidValue;

    fn try_from(text: String) -> Result<Self, Self::Error> {
        Ok(Self(text))
    }
}

impl From<SessionId> for String {
    fn from(id: SessionId) -> Self {
        id.0
    }
}

keyword_enum! {
    /// An MCP server a run is given.
    pub enum McpServerKind: "MCP server" {
        /// The shared bus of the team, `agentbus serve`.
        Agentbus => "agentbus",
        /// Serena, which finds code through language servers.
        Serena => "serena",
    }
}

/// Starting a conversation or going on with one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunMode {
    /// A new conversation. `session_id` is an ID the harness made up: a CLI that lets the caller choose it (Claude
    /// Code) uses it, the others ignore it and report their own in [`RunOutput::session_id`].
    New {
        /// The ID to give the conversation, where the CLI takes one.
        session_id: SessionId,
    },
    /// The next turn of an earlier conversation.
    Resume {
        /// The conversation to go on with.
        session_id: SessionId,
    },
}

/// What the runtime asks an adapter to start.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunRequest {
    /// The CLI's program, from the configuration.
    pub program: Program,
    /// The model to run; without one the CLI runs its default.
    pub model: Option<ModelName>,
    /// The agent's extra arguments from the configuration, added as they are.
    pub extra_args: CliArgs,
    /// A new conversation or the next turn of one.
    pub mode: RunMode,
    /// The prompt of this turn, complete.
    pub prompt: String,
    /// The directory the CLI runs in: the task's worktree.
    pub working_dir: AbsolutePath,
    /// A directory of the run for the files the adapter lays down outside the worktree (MCP configuration,
    /// settings). It exists when the run starts.
    pub run_dir: AbsolutePath,
    /// The MCP servers the run is given, each kind at most once; none for a run without any.
    pub mcp_servers: BTreeMap<McpServerKind, CommandLine>,
}

/// A file to write before the CLI starts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunFile {
    /// Where to write it.
    pub path: AbsolutePath,
    /// What to write.
    pub content: String,
}

/// What to start: the result of [`CliAdapter::build_run`].
///
/// It holds no secret: secrets reach the CLI through the environment the runtime itself starts the CLI with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunSpec {
    /// The program to start.
    pub program: Program,
    /// Its arguments, each one argument: never joined, never through a shell.
    pub args: Vec<String>,
    /// Variables added to the environment the runtime starts the CLI with.
    pub env: BTreeMap<EnvName, String>,
    /// The directory to start it in.
    pub working_dir: AbsolutePath,
    /// The files to write first, in order.
    pub files: Vec<RunFile>,
    /// Paths, relative to `working_dir`, that the runtime keeps out of git in the worktree (a file the adapter laid
    /// down inside it), so that the agent never commits them.
    pub worktree_excludes: Vec<String>,
}

/// Why an adapter cannot build a run.
#[derive(Debug, thiserror::Error)]
pub enum BuildRunError {
    /// The content of a file of the run could not be produced.
    #[error("cannot produce the content of {path:?} for a {kind} run", path = .path.as_path(), kind = .kind)]
    File {
        /// The CLI the run is for.
        kind: CliKind,
        /// The file that could not be produced.
        path: AbsolutePath,
        /// Why not.
        #[source]
        source: Box<dyn Error + Send + Sync>,
    },
}

/// One step of a run, as the activity log shows it. The runtime adds the time and cuts long texts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ActivityEvent {
    /// A message of the agent.
    Text(String),
    /// A shell command the agent runs: its command line.
    Command(String),
    /// A file the agent changes.
    FileEdit {
        /// The file, as the CLI names it.
        path: String,
    },
    /// A call of an agentbus tool.
    BusCall {
        /// The tool.
        tool: String,
        /// Its arguments, as the CLI shows them.
        arguments: String,
    },
    /// A call of any other tool.
    ToolCall {
        /// The tool.
        name: String,
        /// What it is asked to do, in a line.
        summary: String,
    },
    /// An error the CLI reports or a tool call that failed.
    Error(String),
    /// Metadata of the run: the model, the session.
    Info(String),
}

/// A line of the CLI's output the parser could not read. The run goes on: the runtime logs it once and feeds the
/// next line.
#[derive(Debug, thiserror::Error)]
#[error("cannot read a line of CLI output: {line:?}")]
pub struct OutputError {
    /// The line, cut after 200 characters.
    line: String,
    /// What was wrong with it.
    #[source]
    source: Box<dyn Error + Send + Sync>,
}

impl OutputError {
    /// An error for `line`, which is kept cut after 200 characters.
    #[must_use]
    pub fn new(line: &str, source: impl Error + Send + Sync + 'static) -> Self {
        Self {
            line: line.to_owned(),
            source: Box::new(source),
        }
    }

    /// The line, cut after 200 characters.
    #[must_use]
    pub fn line(&self) -> &str {
        &self.line
    }
}

/// The last lines of a run's error output: what the runtime reads to recognise a quota or a rate limit.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ErrorTail(Vec<String>);

impl ErrorTail {
    /// The last `lines` non-blank lines of `text`, trimmed, oldest first.
    #[must_use]
    pub fn from_text(text: &str, lines: NonZeroU32) -> Self {
        let _ = (text, lines);
        Self(Vec::new())
    }

    /// The lines, oldest first.
    #[must_use]
    pub fn lines(&self) -> &[String] {
        &self.0
    }

    /// Whether there is no line.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

/// What the CLI's own stream says about how the run ended; the exit status is the runtime's to look at.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamOutcome {
    /// The stream ended with a successful result.
    Succeeded,
    /// The stream ended with a result that reports an error, whatever the exit status.
    Failed,
    /// The stream has no result: the run was killed or the CLI broke off.
    Incomplete,
}

/// What an adapter extracts from a finished run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunOutput {
    /// The agent's final message, which ends with its `STATUS:` line; none when the run produced none.
    pub final_message: Option<String>,
    /// The conversation's ID as the CLI reported it; none when it did not.
    pub session_id: Option<SessionId>,
    /// How the stream says the run ended.
    pub stream: StreamOutcome,
    /// The last lines of the error output.
    pub error_tail: ErrorTail,
}

/// Reads the output of one run. Made by [`CliAdapter::parser`], one per run: it may keep what it has seen.
pub trait OutputParser: Send {
    /// Reads one line of the CLI's stdout and returns the steps it shows, none for a line that shows no step.
    ///
    /// # Errors
    /// [`OutputError`] for a line that is not what the CLI writes; the parser stays usable.
    fn push_line(&mut self, line: &str) -> Result<Vec<ActivityEvent>, OutputError>;

    /// Ends the run: the final message, the session ID and the error tail, from what the stream showed and the
    /// CLI's stderr. `tail_lines` is how many lines of the error output the tail keeps.
    fn finish(self: Box<Self>, stderr: &str, tail_lines: NonZeroU32) -> RunOutput;
}

/// What the runtime needs of a model CLI. Pure: an adapter never spawns anything.
pub trait CliAdapter: Send + Sync {
    /// The CLI this adapter is for.
    fn kind(&self) -> CliKind;

    /// What to start for `request`.
    ///
    /// # Errors
    /// [`BuildRunError`] when a file of the run cannot be produced.
    fn build_run(&self, request: &RunRequest) -> Result<RunSpec, BuildRunError>;

    /// A new parser for the output of one run. It is a trait object because the CLI is chosen at run time, from
    /// the configuration.
    fn parser(&self) -> Box<dyn OutputParser>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kinds::tests::{assert_keywords, assert_refuses};

    fn tail(text: &str, lines: u32) -> Vec<String> {
        ErrorTail::from_text(text, NonZeroU32::new(lines).unwrap())
            .lines()
            .to_vec()
    }

    // Rule: a session ID is 1 to 200 bytes without whitespace or control characters.
    #[test]
    fn session_ids_follow_their_rule() {
        for text in [
            "a",
            "0a1b2c3d-1111-2222-3333-444455556666",
            "ses_01HZ",
            "żółw",
            &"x".repeat(MAX_SESSION_ID_LEN),
        ] {
            let id: SessionId = text.parse().unwrap();
            assert_eq!(id.as_str(), text);
            assert_eq!(id.to_string(), text);
            assert_eq!(SessionId::try_from(text.to_owned()).unwrap(), id);
            assert_eq!(String::from(id), text);
        }
        let refused = [
            ("", ValueRule::Empty),
            (" ", ValueRule::ForbiddenChar(' ')),
            ("a b", ValueRule::ForbiddenChar(' ')),
            ("a\n", ValueRule::ForbiddenChar('\n')),
            ("a\0", ValueRule::ForbiddenChar('\0')),
            ("a\u{a0}b", ValueRule::ForbiddenChar('\u{a0}')),
        ];
        for (text, rule) in refused {
            let err = text.parse::<SessionId>().unwrap_err();
            assert_eq!(
                (err.what, err.value.as_str(), err.rule),
                ("session ID", text, rule),
                "{text:?}"
            );
        }
        let long = "x".repeat(MAX_SESSION_ID_LEN + 1);
        let err = long.parse::<SessionId>().unwrap_err();
        assert_eq!(
            err.rule,
            ValueRule::TooLong {
                max: MAX_SESSION_ID_LEN
            }
        );
    }

    #[test]
    fn session_ids_go_through_json_as_text_and_are_checked_there() {
        let id: SessionId = "ses_1".parse().unwrap();

        assert_eq!(serde_json::to_string(&id).unwrap(), "\"ses_1\"");
        assert_eq!(serde_json::from_str::<SessionId>("\"ses_1\"").unwrap(), id);
        assert!(serde_json::from_str::<SessionId>("\"a b\"").is_err());
        assert!(serde_json::from_str::<SessionId>("\"\"").is_err());
    }

    #[test]
    fn mcp_server_kinds_have_their_texts() {
        const TABLE: &[(McpServerKind, &str)] =
            &[(McpServerKind::Agentbus, "agentbus"), (McpServerKind::Serena, "serena")];
        assert_keywords(TABLE);
        assert_refuses("MCP server", TABLE, &["bus"]);
    }

    // Rule: the tail is the last non-blank lines, trimmed, oldest first.
    #[test]
    fn the_error_tail_keeps_the_last_lines() {
        let text = "one\ntwo\n  three  \n\n\nfour\n";

        assert_eq!(tail(text, 2), ["three", "four"]);
        assert_eq!(tail(text, 4), ["one", "two", "three", "four"]);
        assert_eq!(tail(text, 30), ["one", "two", "three", "four"]);
        assert_eq!(tail(text, 1), ["four"]);
    }

    #[test]
    fn the_error_tail_of_nothing_is_empty() {
        for text in ["", "\n", "  \n \t\n"] {
            let tail = ErrorTail::from_text(text, NonZeroU32::MIN);
            assert!(tail.is_empty(), "{text:?}");
            assert_eq!(tail, ErrorTail::default());
        }
        assert!(!ErrorTail::from_text("x", NonZeroU32::MIN).is_empty());
    }

    #[test]
    fn the_error_tail_handles_other_line_endings_and_non_ascii() {
        assert_eq!(tail("a\r\nżółć\r\n日本語", 2), ["żółć", "日本語"]);
    }

    // Rule: a huge line does not become a huge error; the cut never splits a character.
    #[test]
    fn an_output_error_keeps_a_cut_line_and_its_cause() {
        let cause = std::io::Error::from(std::io::ErrorKind::InvalidData);
        let line = "ż".repeat(MAX_ERROR_LINE_CHARS + 50);

        let err = OutputError::new(&line, cause);

        assert_eq!(err.line(), "ż".repeat(MAX_ERROR_LINE_CHARS));
        let source = std::error::Error::source(&err).and_then(|s| s.downcast_ref::<std::io::Error>());
        assert_eq!(source.map(std::io::Error::kind), Some(std::io::ErrorKind::InvalidData));
        assert_eq!(OutputError::new("short", std::io::Error::other("x")).line(), "short");
    }
}
