//! The only ways to read a state type from outside the crate are the validating ones: the `Deserialize` trait
//! function that serde generates for a derive must not leak out as a public inherent `Type::deserialize` that skips
//! the checks, which `State::insert_team` and `to_json` would then accept and a reload would refuse.

#[cfg(test)]
mod public_api {
    use agent_core::state::{
        Completion, GateOutcome, ItemPhase, ItemProgress, Member, Question, QuestionState, ReviewOutcome, Run, RunEnd,
        RunState, Team,
    };
    use serde::Deserialize;
    use serde_json::{Value, json};

    /// The state, as shipped with the tests.
    const FIXTURE: &str = include_str!("fixtures/state-v1.json");

    /// The first team of the fixture, as JSON.
    fn team() -> Value {
        let mut state: Value = serde_json::from_str(FIXTURE).unwrap();
        state["teams"][0].take()
    }

    /// Asserts that reading `value` through `T::deserialize` is refused.
    fn assert_refused<'de, T: Deserialize<'de>>(
        read: impl FnOnce(Value) -> Result<T, serde_json::Error>,
        value: &Value,
    ) {
        assert!(read(value.clone()).is_err(), "{value} should be refused");
    }

    #[test]
    fn team_deserialize_checks_the_progress_against_the_task_list() {
        let mut team = team();
        team["items"] = json!([]);

        assert_refused(Team::deserialize, &team);
    }

    #[test]
    fn member_deserialize_refuses_two_runs_going() {
        let running =
            |id: &str| json!({"id": id, "started_at": "2026-11-11T19:38:00Z", "state": {"status": "running"}});
        let mut member = team()["crew"]["members"][0].take();
        member["runs"] = json!([running("run-20261111-193001-r7q2"), running("run-20261111-193002-r7q2")]);

        assert_refused(Member::deserialize, &member);
    }

    #[test]
    fn run_deserialize_refuses_an_end_before_the_start() {
        let run = json!({
            "id": "run-20261111-193001-r7q2",
            "started_at": "2026-11-11T19:38:00Z",
            "state": {"status": "ended", "at": "2026-11-11T19:00:00Z", "end": {"kind": "hung"}}
        });

        assert_refused(Run::deserialize, &run);
    }

    #[test]
    fn question_deserialize_refuses_an_answer_before_the_question() {
        let question = json!({
            "round": 1,
            "file": "/tmp/tasks/questions-1.md",
            "asked_at": "2026-11-11T19:36:00Z",
            "state": {"status": "answered", "at": "2026-11-11T19:00:00Z", "via": "saved_file"}
        });

        assert_refused(Question::deserialize, &question);
    }

    #[test]
    fn item_progress_deserialize_refuses_a_fixing_item_without_a_fix_pass() {
        let item = json!({
            "id": "1.2",
            "phase": {
                "phase": "fixing",
                "assignee": "codex-1",
                "branch": "agents/codex-1/item-1-2",
                "pr": {"number": 7, "branch": "agents/codex-1/item-1-2", "opened_at": "2026-11-11T19:40:00Z"}
            }
        });

        assert_refused(ItemProgress::deserialize, &item);
    }

    #[test]
    fn the_tagged_enums_deserialize_refuses_a_field_they_do_not_have() {
        assert_refused(GateOutcome::deserialize, &json!({"kind": "passed", "extra": 1}));
        assert_refused(Completion::deserialize, &json!({"how": "listed", "extra": 1}));
        assert_refused(ReviewOutcome::deserialize, &json!({"status": "pending", "extra": 1}));
        assert_refused(ItemPhase::deserialize, &json!({"phase": "pending", "extra": 1}));
        assert_refused(RunState::deserialize, &json!({"status": "running", "extra": 1}));
        assert_refused(RunEnd::deserialize, &json!({"kind": "hung", "extra": 1}));
        assert_refused(QuestionState::deserialize, &json!({"status": "open", "extra": 1}));
    }
}
