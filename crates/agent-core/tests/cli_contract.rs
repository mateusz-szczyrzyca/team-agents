//! The CLI contract with a test adapter: it implements [`CliAdapter`] from strings alone, spawning nothing.
//!
//! The adapter speaks a made-up line protocol (`text: …`, `cmd: …`, `edit: …`, `session: …`, `error: …`,
//! `result: ok|error …`), which stands in for the JSON streams of the real CLIs.

use std::collections::BTreeMap;
use std::num::NonZeroU32;

use agent_core::cli::{
    ActivityEvent, BuildRunError, CliAdapter, ErrorTail, McpServerKind, OutputCause, OutputError, OutputParser,
    RunFile, RunMode, RunOutput, RunRequest, RunSpec, SessionId, StreamOutcome,
};
use agent_core::config::{AbsolutePath, CliArgs, CommandLine, ModelName, Program};
use agent_core::kinds::CliKind;

/// A test adapter standing in for a CLI of kind `kind`.
struct TestAdapter {
    kind: CliKind,
}

impl CliAdapter for TestAdapter {
    fn kind(&self) -> CliKind {
        self.kind
    }

    fn build_run(&self, request: &RunRequest) -> Result<RunSpec, BuildRunError> {
        let mut args = vec!["--print".to_owned()];
        if let Some(model) = &request.model {
            args.extend(["--model".to_owned(), model.as_str().to_owned()]);
        }
        match &request.mode {
            RunMode::New { session_id } => args.extend(["--session-id".to_owned(), session_id.to_string()]),
            RunMode::Resume { session_id } => args.extend(["--resume".to_owned(), session_id.to_string()]),
        }
        args.extend(request.extra_args.as_slice().iter().cloned());
        args.push(request.prompt.clone());

        let servers: Vec<String> = request
            .mcp_servers
            .iter()
            .map(|(kind, command)| format!("{kind}={}", command.program().as_path().display()))
            .collect();
        let path =
            AbsolutePath::try_from(format!("{}/mcp.txt", request.run_dir.as_path().display())).map_err(|source| {
                BuildRunError::Path {
                    kind: self.kind,
                    source,
                }
            })?;
        Ok(RunSpec {
            program: request.program.clone(),
            args,
            env: BTreeMap::new(),
            working_dir: request.working_dir.clone(),
            files: vec![RunFile {
                path,
                content: servers.join("\n"),
            }],
            worktree_excludes: Vec::new(),
        })
    }

    fn parser(&self) -> Box<dyn OutputParser> {
        Box::new(TestParser::default())
    }
}

/// What a run of the test protocol has shown so far.
#[derive(Default)]
struct TestParser {
    last_text: Option<String>,
    session: Option<SessionId>,
    result: Option<StreamOutcome>,
}

impl OutputParser for TestParser {
    fn push_line(&mut self, line: &str) -> Result<Vec<ActivityEvent>, OutputError> {
        if line.trim().is_empty() {
            return Ok(Vec::new());
        }
        let Some((kind, rest)) = line.split_once(": ") else {
            return Err(OutputError::new(line, OutputCause::Unrecognized));
        };
        let rest = rest.to_owned();
        Ok(match kind {
            "text" => {
                self.last_text = Some(rest.clone());
                vec![ActivityEvent::Text(rest)]
            }
            "cmd" => vec![ActivityEvent::Command(rest)],
            "edit" => vec![ActivityEvent::FileEdit { path: rest }],
            "session" => {
                self.session = Some(rest.parse().map_err(|source| OutputError::new(line, source))?);
                vec![ActivityEvent::Info(format!("session {rest}"))]
            }
            "error" => vec![ActivityEvent::Error(rest)],
            "result" => {
                self.result = Some(if rest.starts_with("ok") {
                    StreamOutcome::Succeeded
                } else {
                    StreamOutcome::Failed
                });
                Vec::new()
            }
            _ => return Err(OutputError::new(line, OutputCause::Unrecognized)),
        })
    }

    fn finish(self: Box<Self>, stderr: &str, tail_lines: NonZeroU32) -> RunOutput {
        RunOutput {
            final_message: self.last_text,
            session_id: self.session,
            stream: self.result.unwrap_or(StreamOutcome::Incomplete),
            error_tail: ErrorTail::from_text(stderr, tail_lines),
        }
    }
}

fn request(mode: RunMode) -> Result<RunRequest, Box<dyn std::error::Error>> {
    let mut mcp_servers = BTreeMap::new();
    mcp_servers.insert(
        McpServerKind::Agentbus,
        CommandLine::try_from(vec!["/opt/agentbus".to_owned(), "serve".to_owned()])?,
    );
    Ok(RunRequest {
        program: "/usr/bin/test-cli".parse::<Program>()?,
        model: Some("sonnet".parse::<ModelName>()?),
        extra_args: CliArgs::try_from(vec!["--flag".to_owned()])?,
        mode,
        prompt: "Do the item. Quote: \"it's\"".to_owned(),
        working_dir: "/work/tree".parse()?,
        run_dir: "/work/run".parse()?,
        mcp_servers,
    })
}

fn session(text: &str) -> Result<SessionId, Box<dyn std::error::Error>> {
    Ok(text.parse()?)
}

#[test]
fn a_new_run_is_built_from_the_request() -> Result<(), Box<dyn std::error::Error>> {
    let adapter = TestAdapter { kind: CliKind::Claude };

    let spec = adapter.build_run(&request(RunMode::New {
        session_id: session("s-1")?,
    })?)?;

    assert_eq!(spec.program.as_path().to_str(), Some("/usr/bin/test-cli"));
    assert_eq!(
        spec.args,
        [
            "--print",
            "--model",
            "sonnet",
            "--session-id",
            "s-1",
            "--flag",
            "Do the item. Quote: \"it's\""
        ]
    );
    assert_eq!(spec.working_dir.as_path().to_str(), Some("/work/tree"));
    assert_eq!(spec.files.len(), 1);
    assert_eq!(spec.files[0].path.as_path().to_str(), Some("/work/run/mcp.txt"));
    assert_eq!(spec.files[0].content, "agentbus=/opt/agentbus");
    Ok(())
}

#[test]
fn a_resumed_run_names_its_session() -> Result<(), Box<dyn std::error::Error>> {
    let adapter = TestAdapter { kind: CliKind::Codex };

    let spec = adapter.build_run(&request(RunMode::Resume {
        session_id: session("s-9")?,
    })?)?;

    assert_eq!(spec.args[3..5], ["--resume", "s-9"]);
    Ok(())
}

#[test]
fn the_output_becomes_activity_and_a_result() -> Result<(), Box<dyn std::error::Error>> {
    let adapter = TestAdapter { kind: CliKind::Gemini };
    let mut parser = adapter.parser();
    let stdout = "session: s-7\ntext: starting\ncmd: cargo test\nedit: src/lib.rs\n\ntext: done\nresult: ok\n";

    let mut events = Vec::new();
    for line in stdout.lines() {
        events.extend(parser.push_line(line)?);
    }
    let output = parser.finish("warn: a\n\nboom\n", NonZeroU32::MIN);

    assert_eq!(
        events,
        [
            ActivityEvent::Info("session s-7".to_owned()),
            ActivityEvent::Text("starting".to_owned()),
            ActivityEvent::Command("cargo test".to_owned()),
            ActivityEvent::FileEdit {
                path: "src/lib.rs".to_owned()
            },
            ActivityEvent::Text("done".to_owned()),
        ]
    );
    assert_eq!(output.final_message.as_deref(), Some("done"));
    assert_eq!(output.session_id, Some(session("s-7")?));
    assert_eq!(output.stream, StreamOutcome::Succeeded);
    assert_eq!(output.error_tail.lines(), ["boom"]);
    Ok(())
}

#[test]
fn a_run_without_a_result_is_incomplete_and_a_failed_result_is_failed() {
    let adapter = TestAdapter {
        kind: CliKind::Opencode,
    };

    let mut killed = adapter.parser();
    assert_eq!(killed.push_line("text: half").unwrap().len(), 1);
    let killed = killed.finish("", NonZeroU32::MIN);
    assert_eq!(killed.stream, StreamOutcome::Incomplete);
    assert_eq!(killed.final_message.as_deref(), Some("half"));
    assert_eq!(killed.session_id, None);
    assert!(killed.error_tail.is_empty());

    let mut failed = adapter.parser();
    assert_eq!(failed.push_line("result: error 429").unwrap(), []);
    assert_eq!(failed.finish("", NonZeroU32::MIN).stream, StreamOutcome::Failed);
}

#[test]
fn an_unreadable_line_is_an_error_and_the_parser_goes_on() {
    let adapter = TestAdapter { kind: CliKind::Claude };
    let mut parser = adapter.parser();

    let err = parser.push_line("not the protocol").unwrap_err();
    assert_eq!(err.line(), "not the protocol");
    assert!(matches!(err.cause(), OutputCause::Unrecognized));
    assert_eq!(parser.push_line("nope: x").unwrap_err().line(), "nope: x");
    let bad_session = parser.push_line("session: a b").unwrap_err();
    assert!(matches!(bad_session.cause(), OutputCause::SessionId(_)));

    assert_eq!(
        parser.push_line("text: still here").unwrap(),
        [ActivityEvent::Text("still here".to_owned())]
    );
    assert_eq!(
        parser.finish("", NonZeroU32::MIN).final_message.as_deref(),
        Some("still here")
    );
}

#[test]
fn adapters_are_chosen_at_run_time_by_kind() {
    let adapters: Vec<Box<dyn CliAdapter>> = CliKind::ALL
        .iter()
        .map(|&kind| Box::new(TestAdapter { kind }) as Box<dyn CliAdapter>)
        .collect();

    let kinds: Vec<CliKind> = adapters.iter().map(|adapter| adapter.kind()).collect();

    assert_eq!(kinds, CliKind::ALL);
}
