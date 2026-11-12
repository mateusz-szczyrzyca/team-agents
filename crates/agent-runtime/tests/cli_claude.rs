//! The Claude Code adapter: what it starts for a new and for a resumed run, the MCP configuration and settings it lays
//! down, and what it makes of the recorded `stream-json` output.

mod cli_support;

#[cfg(test)]
mod claude_adapter {
    use agent_core::cli::{ActivityEvent, CliAdapter, OutputCause, StreamOutcome};
    use agent_core::kinds::CliKind;
    use agent_runtime::cli::claude::ClaudeAdapter;
    use serde_json::json;

    use crate::cli_support::{
        EARLIER_SESSION, MADE_UP_SESSION, Servers, TestResult, fixture, json_of, new_run, replay, request, resumed_run,
    };

    /// The session ID of the recorded run `claude/done.jsonl`.
    const RECORDED_SESSION: &str = "2fff8ec8-76a8-483a-9e7a-57201308420e";

    fn strings(words: &[&str]) -> Vec<String> {
        words.iter().map(|word| (*word).to_owned()).collect()
    }

    #[test]
    fn it_is_the_adapter_of_claude() {
        assert_eq!(ClaudeAdapter.kind(), CliKind::Claude);
    }

    #[test]
    fn a_new_run_has_these_exact_arguments() -> TestResult {
        let spec = ClaudeAdapter.build_run(&request(new_run()?, Servers::BusAndSerena)?)?;

        assert_eq!(spec.program.as_path().to_str(), Some("/opt/bin/cli"));
        assert_eq!(
            spec.args,
            strings(&[
                "-p",
                "--output-format",
                "stream-json",
                "--verbose",
                "--dangerously-skip-permissions",
                "--mcp-config",
                "/work/run/mcp.json",
                "--max-turns",
                "300",
                "--settings",
                "/work/run/claude-settings.json",
                "--model",
                "model-1",
                "--session-id",
                MADE_UP_SESSION,
                "--add-dir",
                "/extra",
                "Do the item.\nQuote: \"it's\"",
            ])
        );
        assert!(spec.env.is_empty());
        assert_eq!(spec.working_dir.as_path().to_str(), Some("/work/tree"));
        assert_eq!(spec.worktree_excludes, strings(&[".serena/"]));
        Ok(())
    }

    #[test]
    fn a_resumed_run_has_these_exact_arguments() -> TestResult {
        let spec = ClaudeAdapter.build_run(&request(resumed_run()?, Servers::Bus)?)?;

        assert_eq!(
            spec.args,
            strings(&[
                "-p",
                "--output-format",
                "stream-json",
                "--verbose",
                "--dangerously-skip-permissions",
                "--mcp-config",
                "/work/run/mcp.json",
                "--max-turns",
                "300",
                "--settings",
                "/work/run/claude-settings.json",
                "--model",
                "model-1",
                "--resume",
                EARLIER_SESSION,
                "--add-dir",
                "/extra",
                "Do the item.\nQuote: \"it's\"",
            ])
        );
        assert!(spec.env.is_empty());
        assert!(spec.worktree_excludes.is_empty());
        Ok(())
    }

    #[test]
    fn a_run_without_a_model_gets_no_model_flag() -> TestResult {
        let mut request = request(new_run()?, Servers::Bus)?;
        request.model = None;

        let spec = ClaudeAdapter.build_run(&request)?;

        assert!(!spec.args.contains(&"--model".to_owned()), "{:?}", spec.args);
        assert!(!spec.args.contains(&"model-1".to_owned()), "{:?}", spec.args);
        Ok(())
    }

    #[test]
    fn the_mcp_configuration_names_agentbus_serve_and_serena_with_its_context() -> TestResult {
        let spec = ClaudeAdapter.build_run(&request(new_run()?, Servers::BusAndSerena)?)?;

        let file = &spec.files[0];
        assert_eq!(file.path.as_path().to_str(), Some("/work/run/mcp.json"));
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
                            "--context", "claude-code"
                        ]
                    }
                }
            })
        );
        Ok(())
    }

    #[test]
    fn a_run_without_mcp_servers_gets_an_empty_configuration_and_only_that() -> TestResult {
        let spec = ClaudeAdapter.build_run(&request(new_run()?, Servers::None)?)?;

        assert_eq!(json_of(&spec.files[0])?, json!({ "mcpServers": {} }));
        let max_turns = spec
            .args
            .iter()
            .position(|arg| arg == "--max-turns")
            .ok_or("no --max-turns")?;
        assert_eq!(
            spec.args.get(max_turns + 2).map(String::as_str),
            Some("--strict-mcp-config")
        );
        let strict = spec.args.iter().filter(|arg| *arg == "--strict-mcp-config").count();
        assert_eq!(strict, 1);
        Ok(())
    }

    #[test]
    fn a_run_with_agentbus_is_not_strict() -> TestResult {
        let spec = ClaudeAdapter.build_run(&request(new_run()?, Servers::Bus)?)?;

        assert!(
            !spec.args.contains(&"--strict-mcp-config".to_owned()),
            "{:?}",
            spec.args
        );
        Ok(())
    }

    #[test]
    fn the_settings_turn_attribution_off_and_give_a_long_bash_timeout() -> TestResult {
        let spec = ClaudeAdapter.build_run(&request(new_run()?, Servers::Bus)?)?;

        let file = &spec.files[1];
        assert_eq!(file.path.as_path().to_str(), Some("/work/run/claude-settings.json"));
        assert_eq!(
            json_of(file)?,
            json!({
                "includeCoAuthoredBy": false,
                "attribution": { "commit": "", "pr": "" },
                "env": {
                    "CLAUDE_CODE_DISABLE_BACKGROUND_TASKS": "1",
                    "BASH_DEFAULT_TIMEOUT_MS": "1200000",
                    "BASH_MAX_TIMEOUT_MS": "1200000",
                    "ENABLE_TOOL_SEARCH": "false"
                }
            })
        );
        assert_eq!(spec.files.len(), 2);
        Ok(())
    }

    #[test]
    fn the_recorded_run_gives_its_activity_final_message_and_session() -> TestResult {
        let replay = replay(&ClaudeAdapter, &fixture("claude", "done.jsonl")?, "")?;

        assert!(replay.errors.is_empty(), "{:?}", replay.errors);
        assert_eq!(replay.events.len(), 7, "{:?}", replay.events);
        assert_eq!(
            replay.events[0],
            ActivityEvent::Info("session started, model claude-opus-5-5".to_owned())
        );
        let commands: Vec<&str> = replay
            .events
            .iter()
            .filter_map(|event| match event {
                ActivityEvent::Command(command) => Some(command.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(commands.len(), 4);
        assert!(commands[0].starts_with(
            "cd /home/example/agents/worktrees/team-agents/20261102-172403-codex-1 && git rev-parse --abbrev-ref HEAD"
        ));
        assert!(commands[3].starts_with("cat > /home/example/agents/tasks/20261102-173008-inv100/result.md <<'EOF'"));
        assert!(
            matches!(&replay.events[5], ActivityEvent::Text(text) if text.starts_with("STATUS: DONE\nAnswer: I ran"))
        );
        assert_eq!(
            replay.events[6],
            ActivityEvent::Info("result: success, turns: 5".to_owned())
        );
        let final_message = replay.output.final_message.ok_or("no final message")?;
        assert!(
            final_message
                .starts_with("STATUS: DONE\nAnswer: I ran `git diff --name-status origin/main...HEAD -- tests`")
        );
        assert!(final_message.ends_with("The full list is in `result.md`.\nConfidence: high"));
        assert_eq!(replay.output.session_id, Some(RECORDED_SESSION.parse()?));
        assert_eq!(replay.output.stream, StreamOutcome::Succeeded);
        assert!(replay.output.error_tail.is_empty());
        Ok(())
    }

    #[test]
    fn the_error_tail_comes_from_stderr() -> TestResult {
        let replay = replay(
            &ClaudeAdapter,
            &fixture("claude", "done.jsonl")?,
            "warning: slow\n\nrate limit hit\n",
        )?;

        assert_eq!(replay.output.error_tail.lines(), ["warning: slow", "rate limit hit"]);
        Ok(())
    }

    #[test]
    fn a_run_that_hit_its_turn_limit_failed_and_has_no_final_message() -> TestResult {
        let replay = replay(&ClaudeAdapter, &fixture("claude", "max-turns.jsonl")?, "")?;

        assert!(replay.errors.is_empty(), "{:?}", replay.errors);
        assert_eq!(replay.output.stream, StreamOutcome::Failed);
        assert_eq!(replay.output.final_message, None);
        assert_eq!(
            replay.output.session_id,
            Some("6b1f0c1e-2d4a-4f7e-9c11-3a5d8e2b7f40".parse()?)
        );
        assert_eq!(
            replay.events.last(),
            Some(&ActivityEvent::Info("result: error_max_turns, turns: 300".to_owned()))
        );
        Ok(())
    }

    #[test]
    fn a_result_that_reports_an_error_failed_and_keeps_its_text() -> TestResult {
        let replay = replay(&ClaudeAdapter, &fixture("claude", "api-error.jsonl")?, "")?;

        assert_eq!(replay.output.stream, StreamOutcome::Failed);
        let message = replay.output.final_message.ok_or("no message")?;
        assert!(message.starts_with("API Error: 429 "), "{message}");
        Ok(())
    }

    #[test]
    fn a_killed_run_is_incomplete_but_knows_its_session() -> TestResult {
        let replay = replay(&ClaudeAdapter, &fixture("claude", "cut.jsonl")?, "")?;

        assert!(replay.errors.is_empty(), "{:?}", replay.errors);
        assert_eq!(replay.output.stream, StreamOutcome::Incomplete);
        assert_eq!(replay.output.final_message, None);
        assert_eq!(replay.output.session_id, Some(RECORDED_SESSION.parse()?));
        Ok(())
    }

    // Rule: an output cut anywhere is a typed error for the cut line and an incomplete run, never a panic.
    #[test]
    fn an_output_cut_at_any_byte_gives_a_typed_error_and_no_panic() -> TestResult {
        let text = fixture("claude", "done.jsonl")?;
        let complete = text.trim_end().len();

        for (cut, _) in text.char_indices() {
            let replay = replay(&ClaudeAdapter, &text[..cut], "")?;

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
    fn a_line_that_is_not_json_is_an_error_and_the_parser_goes_on() -> TestResult {
        let mut parser = ClaudeAdapter.parser();

        let error = parser.push_line("Warning: update available").err().ok_or("no error")?;
        assert_eq!(error.line(), "Warning: update available");
        assert!(matches!(error.cause(), OutputCause::Json(_)));
        let steps = parser.push_line(r#"{"type":"assistant","message":{"content":[{"type":"text","text":"hi"}]}}"#)?;
        assert_eq!(steps, [ActivityEvent::Text("hi".to_owned())]);
        Ok(())
    }

    #[test]
    fn a_json_line_without_a_type_is_unrecognized_and_an_unknown_type_is_not_an_error() -> TestResult {
        let mut parser = ClaudeAdapter.parser();

        let error = parser.push_line(r#"{"hello":"world"}"#).err().ok_or("no error")?;
        assert!(matches!(error.cause(), OutputCause::Unrecognized));
        assert_eq!(parser.push_line(r#"{"type":"stream_event","event":{}}"#)?, []);
        assert_eq!(parser.push_line("")?, []);
        assert_eq!(parser.push_line("   ")?, []);
        Ok(())
    }

    #[test]
    fn a_session_id_with_a_space_is_a_typed_error() -> TestResult {
        let mut parser = ClaudeAdapter.parser();

        let error = parser
            .push_line(r#"{"type":"system","subtype":"init","session_id":"a b","model":"m"}"#)
            .err()
            .ok_or("no error")?;

        assert!(matches!(error.cause(), OutputCause::SessionId(_)));
        Ok(())
    }

    #[test]
    fn tool_calls_become_commands_edits_bus_calls_and_other_tool_calls() -> TestResult {
        let mut parser = ClaudeAdapter.parser();
        let lines = [
            r#"{"type":"assistant","message":{"content":[{"type":"tool_use","name":"Edit","input":{"file_path":"/w/a.rs","old_string":"x"}}]}}"#,
            r#"{"type":"assistant","message":{"content":[{"type":"tool_use","name":"mcp__agentbus__board_add","input":{"title":"T"}}]}}"#,
            r#"{"type":"assistant","message":{"content":[{"type":"tool_use","name":"mcp__serena__find_symbol","input":{"name_path":"Foo"}}]}}"#,
            r#"{"type":"assistant","message":{"content":[{"type":"tool_use","name":"Read","input":{"file_path":"/w/b.rs"}}]}}"#,
        ];

        let mut events = Vec::new();
        for line in lines {
            events.extend(parser.push_line(line)?);
        }

        assert_eq!(
            events,
            [
                ActivityEvent::FileEdit {
                    path: "/w/a.rs".to_owned()
                },
                ActivityEvent::BusCall {
                    tool: "board_add".to_owned(),
                    arguments: r#"{"title":"T"}"#.to_owned()
                },
                ActivityEvent::ToolCall {
                    name: "mcp__serena__find_symbol".to_owned(),
                    summary: r#"{"name_path":"Foo"}"#.to_owned()
                },
                ActivityEvent::ToolCall {
                    name: "Read".to_owned(),
                    summary: "/w/b.rs".to_owned()
                },
            ]
        );
        Ok(())
    }

    #[test]
    fn a_failed_tool_result_is_an_error_event_whether_its_content_is_text_or_blocks() -> TestResult {
        let mut parser = ClaudeAdapter.parser();

        let text = parser.push_line(
            r#"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"t","content":"exit 1","is_error":true}]}}"#,
        )?;
        let blocks = parser.push_line(
            r#"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"t","content":[{"type":"text","text":"denied"}],"is_error":true}]}}"#,
        )?;
        let prompt_echo = parser.push_line(r#"{"type":"user","message":{"content":"the prompt"}}"#)?;

        assert_eq!(text, [ActivityEvent::Error("exit 1".to_owned())]);
        assert_eq!(blocks, [ActivityEvent::Error("denied".to_owned())]);
        assert_eq!(prompt_echo, []);
        Ok(())
    }
}
