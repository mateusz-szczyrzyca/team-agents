//! The Antigravity (`agy`) adapter: what it starts for a new and for a resumed run, the MCP file it lays down inside
//! the worktree, and what it makes of the recorded `stream-json` output.

mod cli_support;

#[cfg(test)]
mod agy_adapter {
    use agent_core::cli::{ActivityEvent, CliAdapter, OutputCause, StreamOutcome};
    use agent_core::config::CliArgs;
    use agent_core::kinds::CliKind;
    use agent_runtime::cli::agy::AgyAdapter;
    use serde_json::json;

    use crate::cli_support::{
        EARLIER_SESSION, Servers, TestResult, fixture, json_of, new_run, replay, request, resumed_run,
    };

    /// The conversation of the recorded run `agy/done.jsonl`.
    const RECORDED_CONVERSATION: &str = "521e03ee-4a0b-4cc2-8e56-37681e9304b3";

    fn strings(words: &[&str]) -> Vec<String> {
        words.iter().map(|word| (*word).to_owned()).collect()
    }

    #[test]
    fn it_is_the_adapter_of_gemini() {
        assert_eq!(AgyAdapter.kind(), CliKind::Gemini);
    }

    #[test]
    fn a_new_run_has_these_exact_arguments() -> TestResult {
        let spec = AgyAdapter.build_run(&request(new_run()?, Servers::BusAndSerena)?)?;

        assert_eq!(spec.program.as_path().to_str(), Some("/opt/bin/cli"));
        assert_eq!(
            spec.args,
            strings(&[
                "--output-format",
                "stream-json",
                "--dangerously-skip-permissions",
                "--print-timeout",
                "8h",
                "--model",
                "model-1",
                "--add-dir",
                "/extra",
                "-p",
                "Do the item.\nQuote: \"it's\"",
            ])
        );
        assert!(spec.env.is_empty());
        assert_eq!(spec.working_dir.as_path().to_str(), Some("/work/tree"));
        assert_eq!(
            spec.worktree_excludes,
            strings(&[".agents/mcp_config.json", ".serena/"])
        );
        Ok(())
    }

    #[test]
    fn a_resumed_run_has_these_exact_arguments() -> TestResult {
        let spec = AgyAdapter.build_run(&request(resumed_run()?, Servers::Bus)?)?;

        assert_eq!(
            spec.args,
            strings(&[
                "--output-format",
                "stream-json",
                "--dangerously-skip-permissions",
                "--print-timeout",
                "8h",
                "--model",
                "model-1",
                "--add-dir",
                "/extra",
                "--conversation",
                EARLIER_SESSION,
                "-p",
                "Do the item.\nQuote: \"it's\"",
            ])
        );
        assert_eq!(spec.worktree_excludes, strings(&[".agents/mcp_config.json"]));
        Ok(())
    }

    #[test]
    fn an_effort_flag_of_the_agent_reaches_agy_before_the_prompt() -> TestResult {
        let mut request = request(new_run()?, Servers::Bus)?;
        request.model = None;
        request.extra_args = CliArgs::try_from(strings(&["--effort", "high"]))?;

        let spec = AgyAdapter.build_run(&request)?;

        assert_eq!(
            spec.args,
            strings(&[
                "--output-format",
                "stream-json",
                "--dangerously-skip-permissions",
                "--print-timeout",
                "8h",
                "--effort",
                "high",
                "-p",
                "Do the item.\nQuote: \"it's\"",
            ])
        );
        Ok(())
    }

    #[test]
    fn the_mcp_file_is_in_the_worktree_and_names_agentbus_serve_and_serena_with_its_context() -> TestResult {
        let spec = AgyAdapter.build_run(&request(new_run()?, Servers::BusAndSerena)?)?;

        assert_eq!(spec.files.len(), 1);
        let file = &spec.files[0];
        assert_eq!(file.path.as_path().to_str(), Some("/work/tree/.agents/mcp_config.json"));
        assert_eq!(
            json_of(file)?,
            json!({
                "mcpServers": {
                    "agentbus": {
                        "command": "/opt/bin/agentbus",
                        "args": ["serve", "--agent", "claude-1", "--team", "team-7"]
                    },
                    "serena": {
                        "command": "/opt/bin/serena",
                        "args": [
                            "start-mcp-server", "--project", "/work/tree", "--open-web-dashboard", "false",
                            "--context", "antigravity"
                        ]
                    }
                }
            })
        );
        Ok(())
    }

    #[test]
    fn a_run_without_mcp_servers_gets_an_empty_file() -> TestResult {
        let spec = AgyAdapter.build_run(&request(new_run()?, Servers::None)?)?;

        assert_eq!(json_of(&spec.files[0])?, json!({ "mcpServers": {} }));
        Ok(())
    }

    #[test]
    fn the_recorded_run_gives_its_activity_final_message_and_session() -> TestResult {
        let replay = replay(&AgyAdapter, &fixture("agy", "done.jsonl")?, "")?;

        assert!(replay.errors.is_empty(), "{:?}", replay.errors);
        let bus = |tool: &str, arguments: &str| ActivityEvent::BusCall {
            tool: tool.to_owned(),
            arguments: arguments.to_owned(),
        };
        assert_eq!(
            replay.events,
            [
                ActivityEvent::Info(format!("session started {RECORDED_CONVERSATION}")),
                bus("team_status", "{}"),
                bus("read_messages", "{}"),
                bus("board_list", "{}"),
                bus("list_reservations", "{}"),
                bus("set_status", r#"{"status":"sleeping"}"#),
                ActivityEvent::Text(
                    "STATUS: SLEEPING\nRole: member (normal)\nDone: Checked team status, read bus messages (including review findings #260 and #261 for PR #76), verified board status (no open or assigned items), and updated agent status to sleeping.\nNot done: Awaiting board items or instructions from coordinator codex-1 regarding review findings for PR #76.\nUnclear: None."
                        .to_owned()
                ),
                ActivityEvent::Info("result: SUCCESS".to_owned()),
            ]
        );
        let final_message = replay.output.final_message.ok_or("no final message")?;
        assert!(final_message.starts_with("STATUS: SLEEPING\nRole: member (normal)\n"));
        assert!(final_message.ends_with("Unclear: None.\n"));
        assert_eq!(replay.output.session_id, Some(RECORDED_CONVERSATION.parse()?));
        assert_eq!(replay.output.stream, StreamOutcome::Succeeded);
        Ok(())
    }

    #[test]
    fn a_result_with_an_error_fails_and_has_no_final_message() -> TestResult {
        let replay = replay(&AgyAdapter, &fixture("agy", "error.jsonl")?, "")?;

        assert!(replay.errors.is_empty(), "{:?}", replay.errors);
        assert_eq!(
            replay.events,
            [
                ActivityEvent::Info("session started dc63a7b2-9187-4673-b670-600faf7cf9e5".to_owned()),
                ActivityEvent::Error("interrupted".to_owned()),
                ActivityEvent::Info("result: ERROR".to_owned()),
            ]
        );
        assert_eq!(replay.output.stream, StreamOutcome::Failed);
        assert_eq!(replay.output.final_message, None);
        assert_eq!(
            replay.output.session_id,
            Some("dc63a7b2-9187-4673-b670-600faf7cf9e5".parse()?)
        );
        Ok(())
    }

    #[test]
    fn the_error_tail_comes_from_stderr() -> TestResult {
        let replay = replay(&AgyAdapter, &fixture("agy", "error.jsonl")?, "a\n\nquota exceeded\n")?;

        assert_eq!(replay.output.error_tail.lines(), ["a", "quota exceeded"]);
        Ok(())
    }

    // Rule: an output cut anywhere is a typed error for the cut line and an incomplete run, never a panic.
    #[test]
    fn an_output_cut_at_any_byte_gives_a_typed_error_and_no_panic() -> TestResult {
        let text = fixture("agy", "done.jsonl")?;
        let complete = text.trim_end().len();

        for (cut, _) in text.char_indices() {
            let replay = replay(&AgyAdapter, &text[..cut], "")?;

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
    fn a_run_cut_before_its_result_knows_its_conversation() -> TestResult {
        let text = fixture("agy", "done.jsonl")?;
        let without_result: String = text.lines().take(10).map(|line| format!("{line}\n")).collect();

        let replay = replay(&AgyAdapter, &without_result, "")?;

        assert_eq!(replay.output.stream, StreamOutcome::Incomplete);
        assert_eq!(replay.output.session_id, Some(RECORDED_CONVERSATION.parse()?));
        Ok(())
    }

    #[test]
    fn unreadable_lines_are_errors_and_unknown_events_are_not() -> TestResult {
        let mut parser = AgyAdapter.parser();

        let not_json = parser.push_line("warning: slow").err().ok_or("no error")?;
        assert!(matches!(not_json.cause(), OutputCause::Json(_)));
        let no_event = parser.push_line(r#"{"hello":"world"}"#).err().ok_or("no error")?;
        assert!(matches!(no_event.cause(), OutputCause::Unrecognized));
        let bad_session = parser
            .push_line(r#"{"event":"init","conversation_id":"a b"}"#)
            .err()
            .ok_or("no error")?;
        assert!(matches!(bad_session.cause(), OutputCause::SessionId(_)));
        assert_eq!(parser.push_line(r#"{"event":"heartbeat"}"#)?, []);
        assert_eq!(parser.push_line("")?, []);
        Ok(())
    }

    #[test]
    fn a_response_in_pieces_is_one_text_when_its_step_is_done() -> TestResult {
        let mut parser = AgyAdapter.parser();
        let piece = |state: &str, step: u32, text: &str| {
            format!(
                r#"{{"event":"step_update","step_update":{{"step_index":{step},"state":"{state}","step_type":"agent_response","text_delta":"{text}"}}}}"#
            )
        };

        let mut events = Vec::new();
        for line in [
            piece("ACTIVE", 3, "Hel"),
            piece("ACTIVE", 4, "other "),
            piece("ACTIVE", 3, "lo "),
            piece("DONE", 3, "world"),
            piece("DONE", 4, "step"),
        ] {
            events.extend(parser.push_line(&line)?);
        }

        assert_eq!(
            events,
            [
                ActivityEvent::Text("Hello world".to_owned()),
                ActivityEvent::Text("other step".to_owned())
            ]
        );
        Ok(())
    }

    #[test]
    fn tools_become_commands_edits_bus_calls_and_other_tool_calls_and_a_failed_tool_adds_an_error() -> TestResult {
        let mut parser = AgyAdapter.parser();
        let tool = |name: &str, parameters: &str, extra: &str| {
            format!(
                r#"{{"event":"step_update","step_update":{{"step_index":1,"state":"DONE","step_type":"tool","tool_name":"{name}","tool_info":{{"parameters":{parameters}{extra}}}}}}}"#
            )
        };
        let lines = [
            tool("run_command", r#"{"CommandLine":"cargo test"}"#, ""),
            tool("write_to_file", r#"{"TargetFile":"/w/a.rs"}"#, ""),
            tool("view_file", r#"{"AbsolutePath":"/w/b.rs"}"#, ""),
            tool(
                "call_mcp_tool",
                r#"{"ServerName":"serena","ToolName":"find_symbol","Arguments":{"name_path":"Foo"}}"#,
                "",
            ),
            tool(
                "run_command",
                r#"{"CommandLine":"false"}"#,
                r#","error":{"message":"exit status 1"}"#,
            ),
        ];

        let mut events = Vec::new();
        for line in lines {
            events.extend(parser.push_line(&line)?);
        }

        assert_eq!(
            events,
            [
                ActivityEvent::Command("cargo test".to_owned()),
                ActivityEvent::FileEdit {
                    path: "/w/a.rs".to_owned()
                },
                ActivityEvent::ToolCall {
                    name: "view_file".to_owned(),
                    summary: "/w/b.rs".to_owned()
                },
                ActivityEvent::ToolCall {
                    name: "serena.find_symbol".to_owned(),
                    summary: r#"{"name_path":"Foo"}"#.to_owned()
                },
                ActivityEvent::Command("false".to_owned()),
                ActivityEvent::Error("exit status 1".to_owned()),
            ]
        );
        Ok(())
    }
}
