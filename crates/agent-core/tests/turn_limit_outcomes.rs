//! Turn exhaustion is resumable; provider quota and explicit stops still win.
use agent_core::cli::{ErrorTail, RunOutput, StreamOutcome};
use agent_core::config::Limits;
use agent_core::decide::outcome::{ExitStatus, LimitContext, RunFinish, StoppedBy, classify_outcome};
use serde_json::json;
use time::UtcOffset;

#[test]
fn turn_limit_is_distinct_and_quota_takes_precedence() {
    let limits = Limits::default();
    for (message, errors, stopped_by, expected) in [
        (None, "", StoppedBy::Nobody, json!({"kind":"turn_limit"})),
        (
            Some("STATUS: DONE"),
            "",
            StoppedBy::Nobody,
            json!({"kind":"turn_limit"}),
        ),
        (
            None,
            "quota exhausted",
            StoppedBy::Nobody,
            json!({"kind":"quota_exhausted"}),
        ),
        (
            Some("STATUS: DONE"),
            "quota exhausted",
            StoppedBy::Nobody,
            json!({"kind":"quota_exhausted"}),
        ),
        (None, "", StoppedBy::Human, json!({"kind":"stopped"})),
    ] {
        let output = RunOutput {
            final_message: message.map(str::to_owned),
            session_id: None,
            stream: StreamOutcome::TurnLimit,
            error_tail: ErrorTail::from_text(errors, limits.tail_lines),
        };
        let classified = classify_outcome(
            &output,
            RunFinish {
                exit: ExitStatus::Code(0),
                stopped_by,
            },
            &LimitContext {
                limits: &limits,
                now: "2026-11-16T00:00:00Z".parse().unwrap(),
                local_offset: UtcOffset::UTC,
            },
        )
        .unwrap();
        assert_eq!(
            serde_json::to_value(classified.end).unwrap(),
            expected,
            "message={message:?}, errors={errors}"
        );
        assert_eq!(classified.pause.is_some(), !errors.is_empty());
    }
}
