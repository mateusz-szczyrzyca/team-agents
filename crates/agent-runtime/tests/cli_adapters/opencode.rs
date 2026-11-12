//! The opencode adapter: what it starts for a new and for a resumed run, the configuration it hands over in the
//! environment, and what it makes of the recorded `run --format json` output.

use std::collections::BTreeMap;

use agent_core::cli::{ActivityEvent, CliAdapter, OutputCause, RunSpec, StreamOutcome};
use agent_core::kinds::CliKind;
use agent_runtime::cli::opencode::OpencodeAdapter;
use serde_json::{Value, json};

use crate::support::{EARLIER_SESSION, Servers, TestResult, fixture, json_of, new_run, replay, request, resumed_run};

fn strings(words: &[&str]) -> Vec<String> {
    words.iter().map(|word| (*word).to_owned()).collect()
}

/// The environment of `spec` as plain texts.
fn environment(spec: &RunSpec) -> BTreeMap<&str, &str> {
    spec.env
        .iter()
        .map(|(name, value)| (name.as_str(), value.as_str()))
        .collect()
}

/// The configuration `spec` hands over in `OPENCODE_CONFIG_CONTENT`.
fn configuration(spec: &RunSpec) -> TestResult<Value> {
    let text = environment(spec)
        .get("OPENCODE_CONFIG_CONTENT")
        .copied()
        .ok_or("no OPENCODE_CONFIG_CONTENT")?;
    Ok(serde_json::from_str(text)?)
}

#[test]
fn it_is_the_adapter_of_opencode() {
    assert_eq!(OpencodeAdapter.kind(), CliKind::Opencode);
}

#[test]
fn a_new_run_has_these_exact_arguments_and_environment() -> TestResult {
    let spec = OpencodeAdapter.build_run(&request(new_run()?, Servers::BusAndSerena)?)?;

    assert_eq!(spec.program.as_path().to_str(), Some("/opt/bin/cli"));
    assert_eq!(
        spec.args,
        strings(&[
            "run",
            "--standalone",
            "--format",
            "json",
            "--auto",
            "-m",
            "model-1",
            "--add-dir",
            "/extra",
            "--",
            "Do the item.\nQuote: \"it's\"",
        ])
    );
    let env = environment(&spec);
    assert_eq!(env.len(), 3, "{env:?}");
    assert_eq!(env.get("OPENCODE_DISABLE_AUTOUPDATE"), Some(&"1"));
    assert_eq!(
        env.get("OPENCODE_EXPERIMENTAL_BASH_DEFAULT_TIMEOUT_MS"),
        Some(&"600000")
    );
    assert_eq!(spec.working_dir.as_path().to_str(), Some("/work/tree"));
    assert_eq!(spec.worktree_excludes, strings(&[".serena/"]));
    Ok(())
}

#[test]
fn a_resumed_run_has_these_exact_arguments() -> TestResult {
    let spec = OpencodeAdapter.build_run(&request(resumed_run()?, Servers::Bus)?)?;

    assert_eq!(
        spec.args,
        strings(&[
            "run",
            "--standalone",
            "--format",
            "json",
            "--auto",
            "-m",
            "model-1",
            "--add-dir",
            "/extra",
            "--session",
            EARLIER_SESSION,
            "--",
            "Do the item.\nQuote: \"it's\"",
        ])
    );
    assert_eq!(spec.worktree_excludes, Vec::<String>::new());
    Ok(())
}

#[test]
fn a_run_without_a_model_gets_no_model_flag() -> TestResult {
    let mut request = request(new_run()?, Servers::Bus)?;
    request.model = None;

    let spec = OpencodeAdapter.build_run(&request)?;

    assert!(!spec.args.contains(&"-m".to_owned()), "{:?}", spec.args);
    assert_eq!(
        configuration(&spec)?["provider"]
            .get("openrouter")
            .and_then(|p| p.get("models")),
        None
    );
    Ok(())
}

#[test]
fn the_configuration_names_agentbus_serve_and_serena_with_its_context() -> TestResult {
    let mut request = request(new_run()?, Servers::BusAndSerena)?;
    request.model = Some("openrouter/z-ai/glm-4.6".parse()?);

    let spec = OpencodeAdapter.build_run(&request)?;

    assert_eq!(
        configuration(&spec)?,
        json!({
            "$schema": "https://opencode.ai/config.json",
            "autoupdate": false,
            "mcp": {
                "agentbus": {
                    "type": "local",
                    "command": ["/opt/bin/agentbus", "serve", "--agent", "claude-1", "--team", "team-7"],
                    "enabled": true
                },
                "serena": {
                    "type": "local",
                    "command": [
                        "/opt/bin/serena", "start-mcp-server", "--project", "/work/tree",
                        "--open-web-dashboard", "false", "--context", "ide"
                    ],
                    "enabled": true
                }
            },
            "provider": {
                "openrouter": {
                    "options": { "apiKey": "{env:OPENROUTER_API_KEY}" },
                    "models": { "z-ai/glm-4.6": { "name": "z-ai/glm-4.6" } }
                },
                "deepinfra": {
                    "npm": "@ai-sdk/openai-compatible",
                    "name": "DeepInfra",
                    "options": {
                        "baseURL": "https://api.deepinfra.com/v1/openai",
                        "apiKey": "{env:DEEPINFRA_API_KEY}"
                    }
                }
            }
        })
    );
    Ok(())
}

#[test]
fn the_same_configuration_is_kept_in_a_file_of_the_run() -> TestResult {
    let spec = OpencodeAdapter.build_run(&request(new_run()?, Servers::Bus)?)?;

    assert_eq!(spec.files.len(), 1);
    assert_eq!(
        spec.files[0].path.as_path().to_str(),
        Some("/work/run/opencode-config.json")
    );
    assert_eq!(json_of(&spec.files[0])?, configuration(&spec)?);
    Ok(())
}

#[test]
fn a_model_of_a_known_provider_is_registered_with_it_and_other_models_are_not() -> TestResult {
    let cases = [
        ("deepinfra/Qwen/Qwen3-Coder", Some(("deepinfra", "Qwen/Qwen3-Coder"))),
        (
            "openrouter/anthropic/claude-4",
            Some(("openrouter", "anthropic/claude-4")),
        ),
        ("anthropic/claude-4", None),
        ("openrouter", None),
        ("plain-model", None),
    ];

    for (model, registered) in cases {
        let mut request = request(new_run()?, Servers::Bus)?;
        request.model = Some(model.parse()?);

        let spec = OpencodeAdapter.build_run(&request)?;

        let config = configuration(&spec)?;
        let models: Vec<(String, String)> = ["openrouter", "deepinfra"]
            .iter()
            .filter_map(|provider| {
                config["provider"][*provider]["models"].as_object().map(|models| {
                    models
                        .keys()
                        .map(|key| ((*provider).to_owned(), key.clone()))
                        .collect::<Vec<_>>()
                })
            })
            .flatten()
            .collect();
        let expected: Vec<(String, String)> = registered
            .iter()
            .map(|(provider, name)| ((*provider).to_owned(), (*name).to_owned()))
            .collect();
        assert_eq!(models, expected, "{model}");
        assert!(spec.args.contains(&model.to_owned()), "{model}: {:?}", spec.args);
    }
    Ok(())
}

#[test]
fn a_run_without_agentbus_has_none_in_its_configuration() -> TestResult {
    let spec = OpencodeAdapter.build_run(&request(new_run()?, Servers::None)?)?;

    assert_eq!(configuration(&spec)?["mcp"], json!({}));
    Ok(())
}

#[test]
fn the_recorded_run_gives_its_activity_final_message_and_session() -> TestResult {
    let replay = replay(&OpencodeAdapter, &fixture("opencode", "done.jsonl")?, "")?;

    assert!(replay.errors.is_empty(), "{:?}", replay.errors);
    assert_eq!(replay.events.len(), 1, "{:?}", replay.events);
    assert!(
        matches!(&replay.events[0], ActivityEvent::Text(text) if text.starts_with("STATUS: FAILED\nResult: UNCLEAR\nHow: N/A"))
    );
    let final_message = replay.output.final_message.ok_or("no final message")?;
    assert!(final_message.starts_with("STATUS: FAILED\nResult: UNCLEAR\nHow: N/A\nNotes: The request asks to"));
    assert_eq!(
        replay.output.session_id,
        Some("ses_e48294288ffeXb3Wk1EFOhtd7L".parse()?)
    );
    assert_eq!(replay.output.stream, StreamOutcome::Succeeded);
    assert!(replay.output.error_tail.is_empty());
    Ok(())
}

#[test]
fn shell_calls_are_commands_and_code_calls_are_tool_calls_and_the_last_text_is_the_final_message() -> TestResult {
    let replay = replay(&OpencodeAdapter, &fixture("opencode", "tools.jsonl")?, "")?;

    assert!(replay.errors.is_empty(), "{:?}", replay.errors);
    assert_eq!(replay.events.len(), 7, "{:?}", replay.events);
    assert!(
        matches!(&replay.events[0], ActivityEvent::Command(command) if command.starts_with("cd /home/example/agents/worktrees/team-agents/20261107-214333-codex-1 && make gate"))
    );
    assert!(matches!(&replay.events[1], ActivityEvent::Command(command) if command.starts_with("grep -n -E")));
    assert!(matches!(&replay.events[2], ActivityEvent::Command(command) if command.starts_with("sed -n 1,14p")));
    for event in &replay.events[3..6] {
        let ActivityEvent::ToolCall { name, summary } = event else {
            return Err(format!("not a tool call: {event:?}").into());
        };
        assert_eq!(name, "execute");
        assert!(
            summary.starts_with(r#"{"code":"await tools.agentbus.send_message("#)
                || summary.starts_with(r#"{"code":"const r = await tools.agentbus.send_message("#),
            "{summary}"
        );
        assert!(summary.chars().count() <= 161, "{summary}");
    }
    assert!(
        matches!(&replay.events[6], ActivityEvent::Text(text) if text.starts_with("STATUS: DONE\nResult: `make gate`"))
    );
    let final_message = replay.output.final_message.ok_or("no final message")?;
    assert!(final_message.starts_with("STATUS: DONE\nResult: `make gate`"));
    assert_eq!(replay.output.stream, StreamOutcome::Succeeded);
    Ok(())
}

#[test]
fn edits_are_file_edits_and_a_failed_tool_adds_an_error() -> TestResult {
    let replay = replay(&OpencodeAdapter, &fixture("opencode", "mcp-tools.jsonl")?, "")?;

    assert!(replay.errors.is_empty(), "{:?}", replay.errors);
    assert_eq!(replay.events.len(), 4, "{:?}", replay.events);
    assert!(matches!(&replay.events[0], ActivityEvent::ToolCall { name, .. } if name == "execute"));
    assert_eq!(
            replay.events[1],
            ActivityEvent::FileEdit {
                path: "/home/example/agents/worktrees/team-agents/20261108-024241-openrouter-luna-h1032/.agents/reports/20261108-020325-gemini-1.md".to_owned()
            }
        );
    assert!(matches!(&replay.events[2], ActivityEvent::ToolCall { name, .. } if name == "shell"));
    assert!(
        matches!(&replay.events[3], ActivityEvent::Error(text) if text.starts_with("Invalid arguments for tool \"shell\""))
    );
    Ok(())
}

#[test]
fn a_provider_error_fails_the_run_and_its_line_is_in_the_error_tail() -> TestResult {
    let replay = replay(&OpencodeAdapter, &fixture("opencode", "rate-limit.jsonl")?, "")?;

    assert!(replay.errors.is_empty(), "{:?}", replay.errors);
    assert_eq!(
        replay.events,
        [ActivityEvent::Error("Model busy, retry later".to_owned())]
    );
    assert_eq!(replay.output.stream, StreamOutcome::Failed);
    assert_eq!(replay.output.final_message, None);
    assert_eq!(
        replay.output.session_id,
        Some("ses_e56202c48ffeKRD9SRm5jpflqp".parse()?)
    );
    let tail = replay.output.error_tail.lines();
    assert_eq!(tail.len(), 1, "{tail:?}");
    assert!(
        tail[0].contains(r#""status":429"#) && tail[0].contains("provider.rate-limit"),
        "{}",
        tail[0]
    );
    Ok(())
}

#[test]
fn the_error_tail_has_stderr_first_and_the_error_lines_of_the_stream_after_it() -> TestResult {
    let replay = replay(&OpencodeAdapter, &fixture("opencode", "rate-limit.jsonl")?, "boom\n")?;

    let tail = replay.output.error_tail.lines();
    assert_eq!(tail.len(), 2, "{tail:?}");
    assert_eq!(tail[0], "boom");
    assert!(tail[1].contains(r#""status":429"#));
    Ok(())
}

// Rule: an output cut anywhere is a typed error for the cut line and an incomplete run, never a panic.
#[test]
fn an_output_cut_at_any_byte_gives_a_typed_error_and_no_panic() -> TestResult {
    let text = fixture("opencode", "done.jsonl")?;

    for (cut, _) in text.char_indices() {
        let replay = replay(&OpencodeAdapter, text.get(..cut).ok_or("a cut inside a character")?, "")?;

        assert!(replay.errors.len() <= 1, "cut {cut}: {:?}", replay.errors);
        assert!(
            replay
                .errors
                .iter()
                .all(|error| matches!(error.cause(), OutputCause::Json(_))),
            "cut {cut}: {:?}",
            replay.errors
        );
        if replay.output.final_message.is_none() {
            assert_eq!(replay.output.stream, StreamOutcome::Incomplete, "cut {cut}");
        }
    }
    Ok(())
}

#[test]
fn unreadable_lines_are_errors_and_unknown_events_are_not() -> TestResult {
    let mut parser = OpencodeAdapter.parser();

    let not_json = parser.push_line("INFO starting").err().ok_or("no error")?;
    assert!(matches!(not_json.cause(), OutputCause::Json(_)));
    let no_type = parser.push_line(r#"{"sessionID":"ses_1"}"#).err().ok_or("no error")?;
    assert!(matches!(no_type.cause(), OutputCause::Unrecognized));
    let bad_session = parser
        .push_line(r#"{"type":"step_start","sessionID":"a b","part":{}}"#)
        .err()
        .ok_or("no error")?;
    assert!(matches!(bad_session.cause(), OutputCause::SessionId(_)));
    assert_eq!(parser.push_line(r#"{"type":"reasoning","part":{}}"#)?, []);
    assert_eq!(parser.push_line("")?, []);
    Ok(())
}

#[test]
fn an_error_names_its_message_whatever_shape_the_provider_gave_it() -> TestResult {
    let mut parser = OpencodeAdapter.parser();

    let nested =
        parser.push_line(r#"{"type":"error","error":{"name":"APIError","data":{"message":"quota exceeded"}}}"#)?;
    let named = parser.push_line(r#"{"type":"error","error":{"name":"UnknownError"}}"#)?;
    let bare = parser.push_line(r#"{"type":"error","error":{"code":7}}"#)?;

    assert_eq!(nested, [ActivityEvent::Error("quota exceeded".to_owned())]);
    assert_eq!(named, [ActivityEvent::Error("UnknownError".to_owned())]);
    assert_eq!(bare, [ActivityEvent::Error(r#"{"code":7}"#.to_owned())]);
    Ok(())
}

#[test]
fn a_tool_event_of_the_older_name_and_a_tool_without_a_state_are_calls_too() -> TestResult {
    let mut parser = OpencodeAdapter.parser();

    let old = parser.push_line(
            r#"{"type":"tool","sessionID":"ses_1","part":{"tool":"bash","state":{"status":"completed","input":{"command":"ls"}}}}"#,
        )?;
    let bare = parser.push_line(r#"{"type":"tool_use","sessionID":"ses_1","part":{"tool":"glob"}}"#)?;

    assert_eq!(old, [ActivityEvent::Command("ls".to_owned())]);
    assert_eq!(
        bare,
        [ActivityEvent::ToolCall {
            name: "glob".to_owned(),
            summary: "null".to_owned()
        }]
    );
    Ok(())
}

#[test]
fn a_blank_text_is_no_step_and_does_not_replace_the_final_message() -> TestResult {
    let mut parser = OpencodeAdapter.parser();

    let first =
        parser.push_line(r#"{"type":"text","sessionID":"ses_1","part":{"type":"text","text":"STATUS: DONE"}}"#)?;
    let blank = parser.push_line(r#"{"type":"text","sessionID":"ses_1","part":{"type":"text","text":"  "}}"#)?;
    let none = parser.push_line(r#"{"type":"text","sessionID":"ses_1","part":{"type":"text"}}"#)?;

    assert_eq!(first, [ActivityEvent::Text("STATUS: DONE".to_owned())]);
    assert_eq!(blank, []);
    assert_eq!(none, []);
    let output = parser.finish("", std::num::NonZeroU32::MIN);
    assert_eq!(output.final_message.as_deref(), Some("STATUS: DONE"));
    Ok(())
}

#[test]
fn only_a_stop_ends_the_run_successfully() -> TestResult {
    let finish = |reason: &str| {
        format!(r#"{{"type":"step_finish","sessionID":"ses_1","part":{{"type":"step-finish","reason":"{reason}"}}}}"#)
    };
    let outcome = |lines: &[String]| -> TestResult<StreamOutcome> {
        let mut parser = OpencodeAdapter.parser();
        for line in lines {
            parser.push_line(line)?;
        }
        Ok(parser.finish("", std::num::NonZeroU32::MIN).stream)
    };

    assert_eq!(outcome(&[finish("tool-calls")])?, StreamOutcome::Incomplete);
    assert_eq!(outcome(&[finish("stop")])?, StreamOutcome::Succeeded);
    assert_eq!(outcome(&[finish("error")])?, StreamOutcome::Failed);
    assert_eq!(
        outcome(&[finish("stop"), finish("tool-calls")])?,
        StreamOutcome::Succeeded
    );
    Ok(())
}

// Rule: opencode ends a run that answered with its text: recorded runs that the old harness reported as done end on
// the `text` line, with no `step_finish`. A text that a tool call, a finished step, a new step or an error follows is
// not the end.
#[test]
fn a_run_that_ends_on_its_answer_succeeds_without_a_stop() -> TestResult {
    let start = r#"{"type":"step_start","sessionID":"ses_1","part":{"type":"step-start"}}"#;
    let text =
        |text: &str| format!(r#"{{"type":"text","sessionID":"ses_1","part":{{"type":"text","text":"{text}"}}}}"#);
    let tool = r#"{"type":"tool_use","sessionID":"ses_1","part":{"type":"tool","tool":"shell","state":{"status":"completed","input":{"command":"ls"}}}}"#;
    let finish = r#"{"type":"step_finish","sessionID":"ses_1","part":{"type":"step-finish","reason":"tool-calls"}}"#;
    let error = r#"{"type":"error","sessionID":"ses_1","error":{"name":"APIError","data":{"message":"boom"}}}"#;
    let answer = text("STATUS: DONE");
    let table = [
        (vec![start, answer.as_str()], StreamOutcome::Succeeded),
        (vec![answer.as_str()], StreamOutcome::Succeeded),
        (vec![start, answer.as_str(), tool], StreamOutcome::Incomplete),
        (vec![start, answer.as_str(), tool, finish], StreamOutcome::Incomplete),
        (
            vec![start, answer.as_str(), tool, finish, start],
            StreamOutcome::Incomplete,
        ),
        (vec![start, answer.as_str(), start], StreamOutcome::Incomplete),
        (vec![start, answer.as_str(), error], StreamOutcome::Failed),
        (vec![start, tool, finish], StreamOutcome::Incomplete),
    ];

    for (lines, expected) in table {
        let mut parser = OpencodeAdapter.parser();
        for line in &lines {
            parser.push_line(line)?;
        }
        let output = parser.finish("", std::num::NonZeroU32::MIN);

        assert_eq!(output.stream, expected, "{lines:?}");
    }
    let blank = text("  ");
    let mut parser = OpencodeAdapter.parser();
    parser.push_line(start)?;
    parser.push_line(&blank)?;
    assert_eq!(
        parser.finish("", std::num::NonZeroU32::MIN).stream,
        StreamOutcome::Incomplete
    );
    Ok(())
}

// Rule: an event that reports an error with an empty `sessionID` (a recorded run's refusal of an id it does not know)
// is an error of the run, not a bad session ID that hides it.
#[test]
fn an_error_event_with_an_empty_session_id_is_still_the_error_of_the_run() -> TestResult {
    let line = r#"{"type":"error","timestamp":1,"sessionID":"","error":{"type":"unknown","message":"Expected a string starting with \"ses\""}}"#;
    let mut parser = OpencodeAdapter.parser();

    let events = parser.push_line(line)?;

    assert_eq!(
        events,
        [ActivityEvent::Error(
            "Expected a string starting with \"ses\"".to_owned()
        )]
    );
    let output = parser.finish("", std::num::NonZeroU32::MIN);
    assert_eq!(output.stream, StreamOutcome::Failed);
    assert_eq!(output.session_id, None);
    assert_eq!(output.error_tail.lines(), [line]);
    Ok(())
}
