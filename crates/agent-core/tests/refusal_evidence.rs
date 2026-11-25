//! Provider evidence keeps its structured fields and falls back to text reset times at invalid boundaries.

use agent_core::cli::{ErrorTail, RunOutput, StreamOutcome};
use agent_core::config::Config;
use agent_core::decide::outcome::{ExitStatus, LimitContext, RunFinish, StoppedBy, classify_outcome};
use agent_core::state::Timestamp;
use time::UtcOffset;

#[test]
fn long_provider_messages_keep_the_status_and_code() {
    let message = "界".repeat(220);
    let line = serde_json::json!({"type":"error","error":{
        "type":"provider.rate-limit", "status":429, "message":message,
        "response":{"body":"{\"error\":{\"code\":\"engine_overloaded\"}}"}
    }})
    .to_string();
    let pause = classify(&line).unwrap();
    assert!(pause.evidence.starts_with("an overload: 429 · "), "{pause:?}");
    assert!(pause.evidence.ends_with(" · engine_overloaded"), "{pause:?}");
    assert!(pause.evidence.chars().count() < message.chars().count(), "{pause:?}");
}

#[test]
fn invalid_or_elapsed_structured_resets_fall_back_to_the_result_text() {
    for (name, reset) in [
        ("past", 1_i64),
        ("exactly now", 1_794_477_600),
        ("out of range", i64::MAX),
    ] {
        let line = format!(
            "{{\"type\":\"rate_limit_event\",\"rate_limit_info\":{{\"status\":\"rejected\",\"rateLimitType\":\"five_hour\",\"resetsAt\":{reset}}}}}\nAPI Error: 429 rate limit exceeded · resets 3pm"
        );
        let pause = classify(&line).unwrap();
        assert_eq!(
            pause.until,
            "2026-11-12T15:01:00Z".parse::<Timestamp>().unwrap(),
            "{name}"
        );
    }
}

fn classify(lines: &str) -> Result<agent_core::decide::outcome::Pause, Box<dyn std::error::Error>> {
    let config = Config::default();
    let output = RunOutput {
        final_message: None,
        session_id: None,
        stream: StreamOutcome::Failed,
        error_tail: ErrorTail::from_text(lines, config.limits().tail_lines),
    };
    classify_outcome(
        &output,
        RunFinish {
            exit: ExitStatus::Code(1),
            stopped_by: StoppedBy::Nobody,
        },
        &LimitContext {
            limits: config.limits(),
            now: "2026-11-12T10:00:00Z".parse()?,
            local_offset: UtcOffset::UTC,
        },
    )?
    .pause
    .ok_or_else(|| std::io::Error::other("the provider refusal must pause the run").into())
}
