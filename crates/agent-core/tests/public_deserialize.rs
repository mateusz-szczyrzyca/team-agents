//! Reading through the public API cannot skip a check. `serde_json::from_value::<T>` and a direct `T::deserialize`
//! must both refuse what the state refuses: a `T::deserialize` that skipped the checks would let a caller build a
//! value that no state file could hold again.

#[cfg(test)]
mod public_deserialize {
    use agent_core::state::{
        Completion, GateOutcome, ItemPhase, ItemProgress, Member, Question, QuestionState, ReviewOutcome, Run, RunEnd,
        RunState, Team,
    };
    use serde::Deserialize;
    use serde_json::{Value, json};

    /// The state, as shipped with the tests.
    const FIXTURE: &str = include_str!("fixtures/state-v1.json");

    /// The fixture, as a JSON value.
    fn fixture() -> Value {
        serde_json::from_str(FIXTURE).unwrap()
    }

    /// The fixture's first team: valid.
    fn team() -> Value {
        fixture()["teams"][0].clone()
    }

    /// The fixture's member `codex-1`: valid, with three runs and a question.
    fn member() -> Value {
        team()["crew"]["members"][0].clone()
    }

    /// `value` with `field` set to `to`.
    fn with(mut value: Value, field: &str, to: Value) -> Value {
        value[field] = to;
        value
    }

    /// `value` with one field `extra` more.
    fn with_extra(value: &Value) -> Value {
        with(value.clone(), "extra", json!(1))
    }

    /// Asserts that `T::deserialize(value)`, the call a caller can write, fails with a data error.
    macro_rules! assert_direct_refuses {
        ($t:ty, $value:expr) => {{
            let value: Value = $value;
            let err = <$t>::deserialize(value.clone()).expect_err(&format!("{value} should be refused"));
            assert_eq!(err.classify(), serde_json::error::Category::Data, "{value}: {err}");
        }};
    }

    /// Asserts that `T::deserialize(value)` reads a value that is valid.
    macro_rules! assert_direct_reads {
        ($t:ty, $value:expr) => {{
            let value: Value = $value;
            <$t>::deserialize(value.clone()).unwrap_or_else(|err| panic!("{value} should be valid: {err}"));
        }};
    }

    #[test]
    fn a_direct_deserialize_of_a_team_checks_that_the_progress_fits_the_task_list() {
        assert_direct_reads!(Team, team());

        assert_direct_refuses!(Team, with(team(), "items", json!([])));
    }

    #[test]
    fn a_direct_deserialize_of_a_member_checks_that_only_the_last_run_goes() {
        let mut runs = member()["runs"].clone();
        let going = runs[2].clone();
        runs.as_array_mut().unwrap().push(going);
        assert_direct_reads!(Member, member());

        assert_direct_refuses!(Member, with(member(), "runs", runs));
    }

    #[test]
    fn a_direct_deserialize_of_a_run_checks_that_it_does_not_end_before_it_started() {
        let run = json!({
            "id": "run-20261111-193002-r7q2",
            "started_at": "2026-11-11T19:31:00Z",
            "state": {"status": "ended", "at": "2026-11-11T19:30:00Z", "end": {"kind": "hung"}}
        });

        assert_direct_reads!(Run, member()["runs"][0].clone());
        assert_direct_refuses!(Run, run);
    }

    #[test]
    fn a_direct_deserialize_of_a_question_checks_that_it_is_not_answered_before_it_was_asked() {
        let question = json!({
            "round": 1,
            "file": "/home/dev/agents-test/tasks/task-20261111-193002-k3x9/questions-1.md",
            "asked_at": "2026-11-11T19:36:00Z",
            "state": {"status": "answered", "at": "2026-11-11T19:30:00Z", "via": "dashboard"}
        });

        assert_direct_reads!(Question, member()["questions"][0].clone());
        assert_direct_refuses!(Question, question);
    }

    #[test]
    fn a_direct_deserialize_of_an_item_progress_checks_that_a_fixing_item_has_a_fix_pass() {
        let fixing = json!({
            "id": "1.2",
            "phase": {
                "phase": "fixing",
                "assignee": "codex-1",
                "branch": "agents/codex-1/item-1-2",
                "pr": {"number": 82, "branch": "agents/codex-1/item-1-2", "opened_at": "2026-11-11T19:40:00Z"}
            }
        });

        assert_direct_reads!(ItemProgress, team()["items"][1].clone());
        assert_direct_refuses!(ItemProgress, fixing);
    }

    #[test]
    fn a_direct_deserialize_of_an_item_phase_checks_the_reviews_it_carries() {
        let approved = json!({
            "phase": "approved",
            "pr": {"number": 82, "branch": "agents/codex-1/item-1-2", "opened_at": "2026-11-11T19:40:00Z"},
            "reviews": []
        });

        assert_direct_reads!(ItemPhase, json!({"phase": "pending"}));
        assert_direct_refuses!(ItemPhase, approved);
    }

    #[test]
    fn a_direct_deserialize_of_a_tagged_enum_refuses_a_field_the_variant_does_not_have() {
        let pr = json!({"number": 7, "branch": "agents/codex-1/item-1-2", "opened_at": "2026-11-11T19:15:30Z"});
        let at = "2026-11-11T19:15:30Z";

        assert_direct_refuses!(ItemPhase, with_extra(&json!({"phase": "pending"})));
        assert_direct_refuses!(ReviewOutcome, with_extra(&json!({"status": "pending"})));
        assert_direct_refuses!(Completion, with_extra(&json!({"how": "merged", "pr": pr, "at": at})));
        assert_direct_refuses!(GateOutcome, with_extra(&json!({"kind": "passed"})));
        assert_direct_refuses!(QuestionState, with_extra(&json!({"status": "open"})));
        assert_direct_refuses!(RunState, with_extra(&json!({"status": "running"})));
        assert_direct_refuses!(RunEnd, with_extra(&json!({"kind": "hung"})));
    }

    #[test]
    fn a_direct_deserialize_of_a_tagged_enum_reads_what_is_valid() {
        assert_direct_reads!(ReviewOutcome, json!({"status": "pending"}));
        assert_direct_reads!(GateOutcome, json!({"kind": "failed", "exit_code": 2}));
        assert_direct_reads!(RunEnd, json!({"kind": "quota_exhausted"}));
    }
}
