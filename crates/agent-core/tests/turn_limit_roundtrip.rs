//! Seeded persistence, protocol and consecutive-suffix invariants for turn-limit endings.

use agent_core::config::Limits;
use agent_core::decide::outcome::{AfterTurnLimit, after_turn_limit, consecutive_turn_limits};
use agent_core::protocol::Frame;
use agent_core::state::{Run, RunEnd, State};
use rand::{RngExt, SeedableRng, rngs::StdRng};
use serde_json::json;

#[test]
fn seeded_turn_limit_runs_round_trip_and_count_only_the_consecutive_suffix() {
    for seed in 0..100 {
        let mut rng = StdRng::seed_from_u64(seed);
        let mut expected = 0;
        let mut runs = Vec::new();
        for index in 0..rng.random_range(1..100) {
            let limited = rng.random_bool(0.7);
            expected = if limited { expected + 1 } else { 0 };
            let end = if limited {
                json!({"kind":"turn_limit"})
            } else {
                json!({"kind":"stopped"})
            };
            let value = json!({"id":format!("run-20261116-000000-{index:04}"), "session":"session-🦀", "started_at":"2026-11-16T00:00:00Z",
                "state":{"status":"ended", "at":"2026-11-16T00:00:00Z", "end":end}});
            let run: Run = serde_json::from_value(value).unwrap_or_else(|e| panic!("seed={seed}: {e}"));
            let encoded = serde_json::to_string(&run).unwrap();
            assert_eq!(serde_json::from_str::<Run>(&encoded).unwrap(), run, "seed={seed}");
            runs.push(run);
            assert_eq!(consecutive_turn_limits(&runs), expected, "seed={seed}, index={index}");
            let limit = rng.random_range(0..8);
            let config = Limits {
                turn_limit_resumes: limit,
                ..Limits::default()
            };
            let session = "session".parse().unwrap();
            assert_eq!(
                after_turn_limit(expected, Some(&session), &config),
                if expected <= u64::from(limit) {
                    AfterTurnLimit::Resume
                } else {
                    AfterTurnLimit::Exhausted
                },
                "seed={seed}"
            );
            let event = json!({"type":"event", "protocol":3, "at":"2026-11-16T00:00:00Z", "event":{
                "kind":"run_ended", "team":"team-20261116-000000-0000", "task":"task-20261116-000000-0000",
                "run":format!("run-20261116-000000-{index:04}"), "end":end}});
            let frame = Frame::from_line(&event.to_string()).unwrap_or_else(|e| panic!("seed={seed}: {e}"));
            assert_eq!(
                Frame::from_line(&frame.to_line().unwrap()).unwrap(),
                frame,
                "seed={seed}"
            );
        }
    }
}

#[test]
fn turn_limit_rejects_fields_of_other_endings_and_old_states_remain_readable() {
    assert_eq!(
        serde_json::from_value::<RunEnd>(json!({"kind":"turn_limit"})).unwrap(),
        RunEnd::TurnLimit
    );
    for extra in [
        json!({"outcome":"DONE"}),
        json!({"exit_code":0}),
        json!({"resets_at":"2026-11-16T00:00:00Z"}),
    ] {
        let mut value = json!({"kind":"turn_limit"});
        value
            .as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        let error = serde_json::from_value::<RunEnd>(value).unwrap_err();
        assert!(error.is_data(), "fields of another variant must be rejected");
    }
    for text in [
        include_str!("fixtures/state-v1.json"),
        include_str!("fixtures/state-v2.json"),
    ] {
        let state = State::from_json(text).unwrap();
        assert_eq!(State::from_json(&state.to_json().unwrap()).unwrap(), state);
    }
}
