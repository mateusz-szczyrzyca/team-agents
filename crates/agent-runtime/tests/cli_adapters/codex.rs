//! The Codex adapter: what it starts for a new and for a resumed run, the `-c` settings that give it its MCP servers,
//! and what it makes of the recorded `exec --json` event stream.

use agent_core::cli::{ActivityEvent, CliAdapter, OutputCause, StreamOutcome};
use agent_core::kinds::CliKind;
use agent_runtime::cli::codex::CodexAdapter;

use crate::support::{EARLIER_SESSION, Servers, TestResult, fixture, new_run, replay, request, resumed_run};

fn strings(words: &[&str]) -> Vec<String> {
    words.iter().map(|word| (*word).to_owned()).collect()
}

/// The `-c` pairs of agentbus: its command, its arguments as a TOML array and its long tool timeout.
const BUS_SETTINGS: [&str; 6] = [
    "-c",
    r#"mcp_servers.agentbus.command="/opt/bin/agentbus""#,
    "-c",
    r#"mcp_servers.agentbus.args=["serve","--agent","claude-1","--team","team-7"]"#,
    "-c",
    "mcp_servers.agentbus.tool_timeout_sec=900",
];

#[test]
fn it_is_the_adapter_of_codex() {
    assert_eq!(CodexAdapter.kind(), CliKind::Codex);
}

#[test]
fn a_new_run_has_these_exact_arguments() -> TestResult {
    let spec = CodexAdapter.build_run(&request(new_run()?, Servers::BusAndSerena)?)?;

    let mut expected = strings(&[
        "exec",
        "--json",
        "--dangerously-bypass-approvals-and-sandbox",
        "-C",
        "/work/tree",
    ]);
    expected.extend(strings(&BUS_SETTINGS));
    expected.extend(strings(&[
            "-c",
            r#"mcp_servers.serena.command="/opt/bin/serena""#,
            "-c",
            r#"mcp_servers.serena.args=["start-mcp-server","--project","/work/tree","--open-web-dashboard","false","--context","codex"]"#,
            "-c",
            "mcp_servers.serena.startup_timeout_sec=120",
            "-c",
            "mcp_servers.serena.tool_timeout_sec=300",
            "-m",
            "model-1",
            "--add-dir",
            "/extra",
            "Do the item.\nQuote: \"it's\"",
        ]));
    assert_eq!(spec.program.as_path().to_str(), Some("/opt/bin/cli"));
    assert_eq!(spec.args, expected);
    assert!(spec.env.is_empty());
    assert_eq!(spec.files, []);
    assert_eq!(spec.working_dir.as_path().to_str(), Some("/work/tree"));
    assert_eq!(spec.worktree_excludes, strings(&[".serena/"]));
    Ok(())
}

#[test]
fn a_resumed_run_has_these_exact_arguments() -> TestResult {
    let spec = CodexAdapter.build_run(&request(resumed_run()?, Servers::Bus)?)?;

    let mut expected = strings(&[
        "exec",
        "--json",
        "--dangerously-bypass-approvals-and-sandbox",
        "-C",
        "/work/tree",
    ]);
    expected.extend(strings(&BUS_SETTINGS));
    expected.extend(strings(&[
        "-m",
        "model-1",
        "--add-dir",
        "/extra",
        "resume",
        EARLIER_SESSION,
        "Do the item.\nQuote: \"it's\"",
    ]));
    assert_eq!(spec.args, expected);
    assert_eq!(spec.worktree_excludes, Vec::<String>::new());
    Ok(())
}

#[test]
fn a_run_without_mcp_servers_and_without_a_model_has_neither_setting() -> TestResult {
    let mut request = request(new_run()?, Servers::None)?;
    request.model = None;

    let spec = CodexAdapter.build_run(&request)?;

    assert_eq!(
        spec.args,
        strings(&[
            "exec",
            "--json",
            "--dangerously-bypass-approvals-and-sandbox",
            "-C",
            "/work/tree",
            "--add-dir",
            "/extra",
            "Do the item.\nQuote: \"it's\"",
        ])
    );
    Ok(())
}

#[test]
fn a_setting_with_a_quote_or_a_backslash_is_an_escaped_toml_string() -> TestResult {
    let mut request = request(new_run()?, Servers::None)?;
    request.mcp_servers.insert(
        agent_core::cli::McpServerKind::Agentbus,
        crate::support::command(&["/opt/my \"bus\"/agentbus", "serve", "a\\b"])?,
    );

    let spec = CodexAdapter.build_run(&request)?;

    assert!(
        spec.args
            .contains(&r#"mcp_servers.agentbus.command="/opt/my \"bus\"/agentbus""#.to_owned())
    );
    assert!(
        spec.args
            .contains(&r#"mcp_servers.agentbus.args=["serve","a\\b"]"#.to_owned())
    );
    Ok(())
}

#[test]
fn a_delete_character_is_escaped_because_toml_does_not_allow_it_raw() -> TestResult {
    let mut request = request(new_run()?, Servers::None)?;
    request.mcp_servers.insert(
        agent_core::cli::McpServerKind::Agentbus,
        crate::support::command(&["/opt/bus\u{7f}/agentbus"])?,
    );

    let spec = CodexAdapter.build_run(&request)?;

    assert!(
        spec.args
            .contains(&"mcp_servers.agentbus.command=\"/opt/bus\\u007f/agentbus\"".to_owned()),
        "{:?}",
        spec.args
    );
    Ok(())
}

#[test]
fn the_recorded_run_gives_its_activity_final_message_and_session() -> TestResult {
    let replay = replay(
        &CodexAdapter,
        &fixture("codex", "done.jsonl")?,
        "Reading additional input from stdin...\n",
    )?;

    assert!(replay.errors.is_empty(), "{:?}", replay.errors);
    assert_eq!(replay.events.len(), 12, "{:?}", replay.events);
    assert_eq!(
        replay.events[0],
        ActivityEvent::Info("session started 01a1aba7-2479-78e1-80a8-ac17afd18b33".to_owned())
    );
    assert_eq!(
        replay.events[1],
        ActivityEvent::Text(
            "I’ll check the team state and coordinator messages before starting round 7 work.".to_owned()
        )
    );
    let bus_tools: Vec<&str> = replay
        .events
        .iter()
        .filter_map(|event| match event {
            ActivityEvent::BusCall { tool, .. } => Some(tool.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(
        bus_tools,
        [
            "team_status",
            "read_messages",
            "notebook_read",
            "claim_lead",
            "wait_for_message",
            "read_messages",
            "board_list",
            "set_status"
        ]
    );
    assert!(replay.events.contains(&ActivityEvent::BusCall {
        tool: "wait_for_message".to_owned(),
        arguments: r#"{"from":"gemini-1","timeout_seconds":45}"#.to_owned()
    }));
    assert!(
        replay
            .events
            .contains(&ActivityEvent::Command("/usr/bin/zsh -lc 'cat AGENTS.md'".to_owned()))
    );
    let final_message = replay.output.final_message.ok_or("no final message")?;
    assert!(
        final_message.starts_with("STATUS: SLEEPING\nRole: member (normal)\n"),
        "{final_message}"
    );
    assert_eq!(
        replay.output.session_id,
        Some("01a1aba7-2479-78e1-80a8-ac17afd18b33".parse()?)
    );
    assert_eq!(replay.output.stream, StreamOutcome::Succeeded);
    assert_eq!(
        replay.output.error_tail.lines(),
        ["Reading additional input from stdin..."]
    );
    Ok(())
}

#[test]
fn a_file_change_is_an_edit_and_a_started_command_is_a_command() -> TestResult {
    let replay = replay(&CodexAdapter, &fixture("codex", "edit.jsonl")?, "")?;

    assert!(replay.errors.is_empty(), "{:?}", replay.errors);
    assert_eq!(
        replay.events,
        [
            ActivityEvent::FileEdit {
                path: "/home/example/agents/tasks/20261102-172403-codex-1/questions-1.md".to_owned()
            },
            ActivityEvent::Command("/usr/bin/zsh -lc 'cat AGENTS.md'".to_owned()),
        ]
    );
    Ok(())
}

#[test]
fn a_run_that_was_stopped_is_incomplete_and_has_no_final_message() -> TestResult {
    let replay = replay(&CodexAdapter, &fixture("codex", "cut.jsonl")?, "")?;

    assert!(replay.errors.is_empty(), "{:?}", replay.errors);
    assert_eq!(replay.output.stream, StreamOutcome::Incomplete);
    assert_eq!(replay.output.final_message, None);
    assert_eq!(
        replay.output.session_id,
        Some("01a1b1cd-aaa4-71f0-92c7-c15aee8ed288".parse()?)
    );
    Ok(())
}

#[test]
fn a_failed_turn_fails_and_reports_the_provider_message() -> TestResult {
    let replay = replay(&CodexAdapter, &fixture("codex", "at-capacity.jsonl")?, "")?;

    assert_eq!(
        replay.events,
        [
            ActivityEvent::Info("session started 01a1a785-93d4-78f3-9cba-c909c3040ff3".to_owned()),
            ActivityEvent::Error("Selected model is at capacity. Please try a different model.".to_owned()),
            ActivityEvent::Error("Selected model is at capacity. Please try a different model.".to_owned()),
        ]
    );
    assert_eq!(replay.output.stream, StreamOutcome::Failed);
    assert_eq!(replay.output.final_message, None);
    Ok(())
}

// Rule: the classifier of run outcomes reads only the error tail, so a provider's refusal that Codex reports on its
// stream (an `error` event and a failed turn) must be in it after stderr, once for each of the two lines.
#[test]
fn the_provider_message_of_a_failed_turn_is_in_the_error_tail_after_stderr() -> TestResult {
    let replay = replay(
        &CodexAdapter,
        &fixture("codex", "at-capacity.jsonl")?,
        "warning: retrying",
    )?;

    assert_eq!(
        replay.output.error_tail.lines(),
        [
            "warning: retrying",
            "Selected model is at capacity. Please try a different model.",
            "Selected model is at capacity. Please try a different model.",
        ]
    );
    Ok(())
}

#[test]
fn a_failed_command_and_an_error_item_are_errors() -> TestResult {
    let replay = replay(&CodexAdapter, &fixture("codex", "failed.jsonl")?, "")?;

    assert_eq!(
        replay.events,
        [
            ActivityEvent::Info("session started 019a0000-0000-7000-8000-000000000001".to_owned()),
            ActivityEvent::Error("exit 101: /usr/bin/bash -lc 'cargo test'".to_owned()),
            ActivityEvent::Error("stream disconnected before completion".to_owned()),
            ActivityEvent::Error("stream disconnected before completion".to_owned()),
        ]
    );
    assert_eq!(replay.output.stream, StreamOutcome::Failed);
    Ok(())
}

// Rule: an output cut anywhere is a typed error for the cut line and an incomplete run, never a panic.
#[test]
fn an_output_cut_at_any_byte_gives_a_typed_error_and_no_panic() -> TestResult {
    let text = fixture("codex", "done.jsonl")?;
    let complete = text.trim_end().len();

    for (cut, _) in text.char_indices() {
        let replay = replay(&CodexAdapter, text.get(..cut).ok_or("a cut inside a character")?, "")?;

        assert!(replay.errors.len() <= 1, "cut {cut}: {:?}", replay.errors);
        assert!(
            replay
                .errors
                .iter()
                .all(|error| matches!(error.cause(), OutputCause::Json(_))),
            "cut {cut}: {:?}",
            replay.errors
        );
        if cut < complete {
            assert_eq!(replay.output.stream, StreamOutcome::Incomplete, "cut {cut}");
            assert_eq!(replay.output.final_message, None, "cut {cut}");
        }
    }
    Ok(())
}

#[test]
fn unreadable_lines_are_errors_and_unknown_events_are_not() -> TestResult {
    let mut parser = CodexAdapter.parser();

    let not_json = parser
        .push_line("Reading prompt from stdin...")
        .err()
        .ok_or("no error")?;
    assert!(matches!(not_json.cause(), OutputCause::Json(_)));
    let no_type = parser.push_line(r#"{"thread_id":"x"}"#).err().ok_or("no error")?;
    assert!(matches!(no_type.cause(), OutputCause::Unrecognized));
    let no_thread = parser
        .push_line(r#"{"type":"thread.started"}"#)
        .err()
        .ok_or("no error")?;
    assert!(matches!(no_thread.cause(), OutputCause::Unrecognized));
    let bad_session = parser
        .push_line(r#"{"type":"thread.started","thread_id":"a b"}"#)
        .err()
        .ok_or("no error")?;
    assert!(matches!(bad_session.cause(), OutputCause::SessionId(_)));
    assert_eq!(parser.push_line(r#"{"type":"turn.started"}"#)?, []);
    assert_eq!(
        parser.push_line(r#"{"type":"item.completed","item":{"type":"reasoning","text":"x"}}"#)?,
        []
    );
    assert_eq!(parser.push_line("")?, []);
    Ok(())
}

#[test]
fn other_tools_become_tool_calls_and_searches() -> TestResult {
    let mut parser = CodexAdapter.parser();

    let serena = parser.push_line(
            r#"{"type":"item.completed","item":{"type":"mcp_tool_call","server":"serena","tool":"find_symbol","arguments":{"name_path":"Foo"}}}"#,
        )?;
    let search = parser.push_line(r#"{"type":"item.completed","item":{"type":"web_search","query":"rust strum"}}"#)?;

    assert_eq!(
        serena,
        [ActivityEvent::ToolCall {
            name: "serena.find_symbol".to_owned(),
            summary: r#"{"name_path":"Foo"}"#.to_owned()
        }]
    );
    assert_eq!(
        search,
        [ActivityEvent::ToolCall {
            name: "web_search".to_owned(),
            summary: "rust strum".to_owned()
        }]
    );
    Ok(())
}
