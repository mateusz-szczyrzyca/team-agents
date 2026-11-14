//! A state file of schema version 1, `tests/fixtures/state-v1.json`: written by this format as it was fixed in item
//! 2.2. It must keep loading — a later build that cannot read it would lose the teams of an installation — and writing
//! it back must give the same text.

#[cfg(test)]
mod state_file {
    use agent_core::ids::{ItemId, TaskId, TeamId};
    use agent_core::kinds::Role;
    use agent_core::state::{
        AnsweredVia, GateOutcome, ItemPhase, MergeMode, Participant, QuestionState, ReviewOutcome, RunEnd, RunState,
        SessionId, State, StateError, UnsupportedSchema,
    };
    use agent_core::status::{ItemStatus, ReviewVerdict, RunOutcome, TaskStatus};

    /// The state, as shipped with the tests.
    const FIXTURE: &str = include_str!("fixtures/state-v1.json");

    /// The team ID that the fixture's first team has.
    fn team_a() -> TeamId {
        "team-20261111-193000-a1b2".parse().unwrap()
    }

    /// The task ID `n` of the fixture.
    fn task(n: u32) -> TaskId {
        format!("task-20261111-1930{n:02}-k3x9").parse().unwrap()
    }

    #[test]
    fn the_state_of_schema_version_1_loads_and_writes_back_to_the_same_text() {
        let state = State::from_json(FIXTURE).unwrap();

        assert_eq!(state.to_json().unwrap(), include_str!("fixtures/state-v1-as-v2.json"));
    }

    #[test]
    fn the_state_saved_and_loaded_back_is_equal() {
        let state = State::from_json(FIXTURE).unwrap();

        assert_eq!(State::from_json(&state.to_json().unwrap()).unwrap(), state);
    }

    #[test]
    fn the_teams_of_the_fixture_are_what_the_file_says() {
        let state = State::from_json(FIXTURE).unwrap();

        let ids: Vec<String> = state.teams().iter().map(|t| t.id().to_string()).collect();
        let team = state.team(&team_a()).unwrap();
        let toy = state.team(&"team-20261111-194500-c3d4".parse().unwrap()).unwrap();

        assert_eq!(ids, ["team-20261111-193000-a1b2", "team-20261111-194500-c3d4"]);
        assert_eq!(
            (team.repo().as_str(), team.merge()),
            ("team-agents", MergeMode::Autopilot)
        );
        assert_eq!((toy.repo().as_str(), toy.merge()), ("toy-repo", MergeMode::NoMerge));
        assert_eq!(team.task_list().list().items().count(), 3);
        assert_eq!(team.reviewers().len(), 2);
        assert_eq!(toy.reviewers(), []);
    }

    #[test]
    fn the_roles_and_participants_of_the_fixture_are_read() {
        let state = State::from_json(FIXTURE).unwrap();

        let roles: Vec<(Role, &str)> = state
            .team(&team_a())
            .unwrap()
            .crew()
            .roster()
            .into_iter()
            .map(|(role, member)| (role, member.agent().name()))
            .collect();

        assert_eq!(
            roles,
            [
                (Role::Coordinator, "inv-opus"),
                (Role::Member, "codex-1"),
                (Role::Worker, "worker-1")
            ]
        );
        let reviewer = state.find_task(&task(7)).unwrap();
        assert_eq!(
            (reviewer.role, reviewer.member.agent()),
            (Role::Reviewer, &Participant::Investigator("inv-codex".parse().unwrap()))
        );
        let solo = state.find_task(&task(11)).unwrap();
        assert_eq!((solo.role, solo.member.status()), (Role::Solo, TaskStatus::Questions));
    }

    #[test]
    fn the_progress_of_the_items_of_the_fixture_is_read() {
        let state = State::from_json(FIXTURE).unwrap();
        let team = state.team(&team_a()).unwrap();
        let item = |id: &str| team.item(id.parse::<ItemId>().unwrap()).unwrap();

        assert_eq!(item("1.1").status(), ItemStatus::Done);
        assert_eq!(item("2.1").status(), ItemStatus::Pending);
        let second = item("1.2");
        assert_eq!(second.status(), ItemStatus::InReview);
        assert_eq!(second.fix_passes().len(), 1);
        let ItemPhase::InReview { pr, reviews } = second.phase() else {
            panic!("not in review");
        };
        assert_eq!(pr.number.get().get(), 82);
        assert_eq!(
            reviews.iter().map(|r| r.outcome).collect::<Vec<_>>(),
            [
                ReviewOutcome::Submitted {
                    verdict: ReviewVerdict::Approve,
                    at: "2026-11-11T19:50:00Z".parse().unwrap()
                },
                ReviewOutcome::Pending
            ]
        );
        let gate = second.gate.as_ref().unwrap();
        assert_eq!(gate.outcome, GateOutcome::Passed);
        assert_eq!(gate.head.as_str(), "26b65de7e4076596768ca96b48a7f94799c93874");
    }

    #[test]
    fn the_runs_and_questions_of_the_fixture_are_read() {
        let state = State::from_json(FIXTURE).unwrap();
        let member = state.find_task(&task(2)).unwrap().member;

        assert_eq!(member.runs().len(), 3);
        assert_eq!(
            member.current_run().map(|run| run.id().to_string()),
            Some(String::from("run-20261111-193004-r7q2"))
        );
        assert_eq!(member.last_session().map(SessionId::as_str), Some("ses_3c1f9a2b0ffe"));
        let RunState::Ended { end, .. } = member.runs()[1].state() else {
            panic!("the second run goes");
        };
        assert_eq!(
            end,
            &RunEnd::QuotaExhausted {
                resets_at: Some("2026-11-11T21:00:00Z".parse().unwrap())
            }
        );
        let RunState::Ended { end, .. } = member.runs()[0].state() else {
            panic!("the first run goes");
        };
        assert_eq!(
            end,
            &RunEnd::Reported {
                outcome: RunOutcome::Questions
            }
        );
        assert_eq!(
            member.questions()[0].state(),
            &QuestionState::Answered {
                at: "2026-11-11T19:37:30Z".parse().unwrap(),
                via: AnsweredVia::SavedFile
            }
        );
        let asking = state.find_task(&task(11)).unwrap().member;
        assert_eq!(asking.open_question().map(|q| q.number().get()), Some(1));
    }

    #[test]
    fn the_same_state_of_another_schema_version_is_refused_with_that_version() {
        let text = FIXTURE.replacen("\"schema_version\": 1", "\"schema_version\": 3", 1);
        assert_ne!(text, FIXTURE);

        let err = State::from_json(&text).unwrap_err();

        assert!(matches!(
            err,
            StateError::UnsupportedSchema(UnsupportedSchema { found: 3 })
        ));
    }
}
