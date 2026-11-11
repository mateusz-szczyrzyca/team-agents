//! The tagged enums of the state and of the protocol refuse a field they do not know, like their structs do: a
//! state or a line from a newer build must not load with part of it silently dropped.

#[cfg(test)]
mod strict_wire {
    use std::fmt::Debug;

    use agent_core::state::{
        Completion, GateOutcome, ItemPhase, Participant, QuestionState, ReviewOutcome, RunEnd, RunState,
    };
    use serde::de::DeserializeOwned;
    use serde_json::error::Category;
    use serde_json::{Value, json};

    /// A timestamp of these tests, as written.
    const AT: &str = "2026-11-11T19:15:30Z";

    /// Asserts that `plain` reads as a `T` and that the same object with one more field is refused as data.
    fn assert_unknown_field_refused<T: DeserializeOwned + Debug>(plain: &Value) {
        serde_json::from_value::<T>(plain.clone()).unwrap_or_else(|err| panic!("{plain} should be valid: {err}"));
        let mut with_extra = plain.clone();
        with_extra
            .as_object_mut()
            .unwrap_or_else(|| panic!("{plain} is not an object"))
            .insert("extra".to_owned(), json!(1));

        let err =
            serde_json::from_value::<T>(with_extra.clone()).expect_err(&format!("{with_extra} should be refused"));

        assert_eq!(err.classify(), Category::Data, "{with_extra}: {err}");
    }

    /// A pull request, as written.
    fn pr() -> Value {
        json!({"number": 7, "branch": "agents/codex-1/item-1-2", "opened_at": AT})
    }

    /// An approving review, as written.
    fn approval() -> Value {
        json!({
            "reviewer": "inv-opus",
            "task": "task-20261111-153005-k3x9",
            "requested_at": AT,
            "outcome": {"status": "submitted", "verdict": "APPROVE", "at": AT}
        })
    }

    #[test]
    fn an_item_phase_refuses_an_unknown_field_in_every_variant() {
        for plain in [
            json!({"phase": "pending"}),
            json!({"phase": "in_progress", "assignee": "codex-1", "branch": "agents/codex-1/item-1-2"}),
            json!({"phase": "in_review", "pr": pr(), "reviews": []}),
            json!({"phase": "fixing", "assignee": "codex-1", "branch": "agents/codex-1/item-1-2", "pr": pr()}),
            json!({"phase": "approved", "pr": pr(), "reviews": [approval()]}),
            json!({"phase": "done", "completion": {"how": "listed"}}),
        ] {
            assert_unknown_field_refused::<ItemPhase>(&plain);
        }
    }

    #[test]
    fn a_review_outcome_and_a_completion_refuse_an_unknown_field() {
        for plain in [
            json!({"status": "pending"}),
            json!({"status": "submitted", "verdict": "APPROVE", "at": AT}),
        ] {
            assert_unknown_field_refused::<ReviewOutcome>(&plain);
        }
        for plain in [json!({"how": "listed"}), json!({"how": "merged", "pr": pr(), "at": AT})] {
            assert_unknown_field_refused::<Completion>(&plain);
        }
    }

    #[test]
    fn a_question_state_a_run_state_and_a_run_end_refuse_an_unknown_field() {
        for plain in [
            json!({"status": "open"}),
            json!({"status": "answered", "at": AT, "via": "dashboard"}),
        ] {
            assert_unknown_field_refused::<QuestionState>(&plain);
        }
        for plain in [
            json!({"status": "running"}),
            json!({"status": "ended", "at": AT, "end": {"kind": "hung"}}),
        ] {
            assert_unknown_field_refused::<RunState>(&plain);
        }
        for plain in [
            json!({"kind": "hung"}),
            json!({"kind": "rate_limited"}),
            json!({"kind": "stopped"}),
            json!({"kind": "no_status", "exit_code": 2}),
            json!({"kind": "quota_exhausted", "resets_at": AT}),
            json!({"kind": "reported", "outcome": "DONE"}),
        ] {
            assert_unknown_field_refused::<RunEnd>(&plain);
        }
    }

    #[test]
    fn a_gate_outcome_and_a_participant_refuse_an_unknown_field() {
        for plain in [
            json!({"kind": "passed"}),
            json!({"kind": "failed", "exit_code": 2}),
            json!({"kind": "timed_out"}),
        ] {
            assert_unknown_field_refused::<GateOutcome>(&plain);
        }
        for plain in [
            json!({"tier": "normal", "name": "codex-1"}),
            json!({"tier": "worker", "name": "worker-1"}),
            json!({"tier": "investigator", "name": "inv-opus"}),
        ] {
            assert_unknown_field_refused::<Participant>(&plain);
        }
    }
}
