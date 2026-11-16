//! Consecutive turn exhaustion is bounded and its count comes from persisted runs.
use agent_core::config::Limits;
use agent_core::decide::outcome::{AfterTurnLimit, after_turn_limit, consecutive_turn_limits};
use agent_core::state::Run;
use serde_json::json;

#[test]
fn first_last_allowed_and_first_refused_resumes_follow_configuration() {
    let session = "session-1".parse().unwrap();
    for (limit, count, expected) in [
        (3, 1, AfterTurnLimit::Resume),
        (3, 3, AfterTurnLimit::Resume),
        (3, 4, AfterTurnLimit::Exhausted),
        (1, 1, AfterTurnLimit::Resume),
        (1, 2, AfterTurnLimit::Exhausted),
        (0, 1, AfterTurnLimit::Exhausted),
        (u32::MAX, u64::from(u32::MAX), AfterTurnLimit::Resume),
        (u32::MAX, u64::from(u32::MAX) + 1, AfterTurnLimit::Exhausted),
        (3, u64::MAX, AfterTurnLimit::Exhausted),
    ] {
        let limits = Limits {
            turn_limit_resumes: limit,
            ..Limits::default()
        };
        assert_eq!(
            after_turn_limit(count, Some(&session), &limits),
            expected,
            "limit={limit}, count={count}"
        );
    }
    assert_eq!(after_turn_limit(1, None, &Limits::default()), AfterTurnLimit::NoSession);
}

#[test]
fn persisted_runs_reset_the_count_after_every_other_end() {
    let other = [
        json!({"kind":"reported", "outcome":"DONE"}),
        json!({"kind":"no_status", "exit_code":1}),
        json!({"kind":"quota_exhausted"}),
        json!({"kind":"rate_limited"}),
        json!({"kind":"hung"}),
        json!({"kind":"stopped"}),
    ];
    for end in other {
        let mut runs = Vec::new();
        for (index, end) in [
            json!({"kind":"turn_limit"}),
            json!({"kind":"turn_limit"}),
            end,
            json!({"kind":"turn_limit"}),
        ]
        .into_iter()
        .enumerate()
        {
            runs.push(serde_json::from_value::<Run>(json!({"id":format!("run-20261116-000000-{index:04}"),
                "started_at":"2026-11-16T00:00:00Z", "state":{"status":"ended", "at":"2026-11-16T00:00:00Z", "end":end}})).unwrap());
            let expected = [1, 2, 0, 1][index];
            assert_eq!(consecutive_turn_limits(&runs), expected, "end={end}");
        }
    }
    assert_eq!(consecutive_turn_limits(&[]), 0);
}
