//! The adapters of the four model CLIs, and what they share.
//!
//! RED skeleton: signatures with wrong bodies.

use std::num::NonZeroU32;
use std::path::Path;

use agent_core::cli::{
    ActivityEvent, BuildRunError, CliAdapter, ErrorTail, McpServerKind, OutputError, OutputParser, RunOutput,
    RunRequest, RunSpec, StreamOutcome,
};
use agent_core::config::AbsolutePath;
use agent_core::kinds::CliKind;

pub mod agy;
pub mod claude;
pub mod codex;
pub mod opencode;

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

/// An MCP server of a run with its command resolved to texts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Server {
    /// Which server it is.
    pub(crate) kind: McpServerKind,
    /// The program.
    pub(crate) program: String,
    /// The arguments.
    pub(crate) args: Vec<String>,
}

/// The servers of `request`.
pub(crate) fn servers(
    _cli: CliKind,
    _request: &RunRequest,
    _serena_context: &str,
) -> Result<Vec<Server>, BuildRunError> {
    Ok(Vec::new())
}

/// The text of `path`.
pub(crate) fn path_text(_cli: CliKind, _path: &Path) -> Result<&str, BuildRunError> {
    Ok("")
}

/// `relative` under `base`.
pub(crate) fn path_under(_cli: CliKind, base: &AbsolutePath, _relative: &str) -> Result<AbsolutePath, BuildRunError> {
    Ok(base.clone())
}

/// `text` cut to `max_chars`.
pub(crate) fn clip(text: &str, _max_chars: usize) -> String {
    text.to_owned()
}

/// The step a call of the tool `name` is.
pub(crate) fn tool_activity(_name: &str, _input: &serde_json::Value) -> ActivityEvent {
    ActivityEvent::Info(String::new())
}

/// A spec that starts nothing.
pub(crate) fn stub_spec(request: &RunRequest) -> RunSpec {
    RunSpec {
        program: request.program.clone(),
        args: Vec::new(),
        env: std::collections::BTreeMap::new(),
        working_dir: request.working_dir.clone(),
        files: Vec::new(),
        worktree_excludes: Vec::new(),
    }
}

/// A parser that sees nothing.
#[derive(Debug)]
pub(crate) struct StubParser;

impl OutputParser for StubParser {
    fn push_line(&mut self, _line: &str) -> Result<Vec<ActivityEvent>, OutputError> {
        Ok(Vec::new())
    }

    fn finish(self: Box<Self>, _stderr: &str, tail_lines: NonZeroU32) -> RunOutput {
        RunOutput {
            final_message: None,
            session_id: None,
            stream: StreamOutcome::Incomplete,
            error_tail: ErrorTail::from_text("", tail_lines),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::ffi::OsStr;
    use std::os::unix::ffi::OsStrExt;

    use agent_core::cli::{McpServerKind, RunMode};
    use agent_core::config::{CliArgs, CommandLine, ValueRule};
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
        assert_eq!(servers_of(&request_with(&[])), []);
    }

    fn servers_of(request: &RunRequest) -> Vec<Server> {
        servers(CliKind::Claude, request, "claude-code").unwrap()
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
            (
                "mcp__agentbus__board_add",
                json!({"title": "T"}),
                bus("board_add", r#"{"title":"T"}"#),
            ),
            ("agentbus.read_messages", json!({}), bus("read_messages", "{}")),
            (
                "agentbus_send_message",
                json!({"to": "x"}),
                bus("send_message", r#"{"to":"x"}"#),
            ),
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
}
