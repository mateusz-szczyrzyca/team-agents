//! Rounds through the public API of the state: opening them for the items of a team, closing them, naming them in a
//! run, and what is refused — each rule by the operations and again when the state is read.

#[cfg(test)]
#[path = "support/world.rs"]
mod world;

#[cfg(test)]
mod rounds {
    use agent_core::ids::RoundId;
    use agent_core::state::{
        AdvanceItemError, BeginRunError, CloseRoundError, Completion, InvalidState, ItemPhase, NewRun, OpenRoundError,
        PhaseKind, ReviewOutcome, Round, RoundEnd, RoundLife, RunError, State, StateError, StateOpenRoundError, Team,
    };
    use agent_core::status::{ReviewVerdict, TaskStatus};
    use serde_json::{Value, json};

    use super::world::{at, item, pr, review, round, run, state, task, team_id};

    /// Moves item `item` of team `team`, which has a round already, along the phases to `to`, with the pull request 7 and
    /// the reviewers of team 1. `to` is `in_progress`, `in_review`, `approved` or `done` (merged).
    fn drive(state: &mut State, team: u32, item_id: &str, to: PhaseKind) {
        let mut found = state.team_mut(&team_id(team)).unwrap();
        let mut handle = found.item_mut(item(item_id)).unwrap();
        handle
            .advance(ItemPhase::InProgress {
                assignee: "codex-1".parse().unwrap(),
                branch: "agents/codex-1/item-1-2".parse().unwrap(),
            })
            .unwrap();
        if to == PhaseKind::InProgress {
            return;
        }
        handle
            .advance(ItemPhase::InReview {
                pr: pr(7),
                reviews: Vec::new(),
            })
            .unwrap();
        if to == PhaseKind::InReview {
            return;
        }
        handle
            .ask_reviews(vec![
                review("inv-codex", 7, ReviewOutcome::Pending),
                review("inv-gemini", 8, ReviewOutcome::Pending),
            ])
            .unwrap();
        for reviewer in [7, 8] {
            handle
                .submit_review(&task(reviewer), ReviewVerdict::Approve, at(40))
                .unwrap();
        }
        let ItemPhase::InReview { pr: opened, reviews } = handle.phase().clone() else {
            panic!("the item is in review");
        };
        handle
            .advance(ItemPhase::Approved {
                pr: opened.clone(),
                reviews,
            })
            .unwrap();
        if to == PhaseKind::Approved {
            return;
        }
        handle
            .advance(ItemPhase::Done {
                completion: Completion::Merged { pr: opened, at: at(50) },
            })
            .unwrap();
    }

    /// `state` as JSON, to be changed into a state that no operation could have made.
    fn as_json(state: &State) -> Value {
        serde_json::from_str(&state.to_json().unwrap()).unwrap()
    }

    /// The error of reading `value` as a state.
    fn read(value: &Value) -> StateError {
        State::from_json(&value.to_string()).expect_err("the state was expected to be refused")
    }

    /// The team `n` of `state`.
    fn team(state: &State, n: u32) -> &Team {
        state.team(&team_id(n)).unwrap()
    }

    /// Stops every member of team `n`, leaving nobody to work on its rounds.
    fn stop_crew(state: &mut State, n: u32) {
        let tasks: Vec<_> = team(state, n)
            .crew()
            .roster()
            .into_iter()
            .map(|(_, member)| member.task().clone())
            .collect();
        for task in tasks {
            state
                .team_mut(&team_id(n))
                .unwrap()
                .crew_mut()
                .find_mut(&task)
                .unwrap()
                .1
                .transition(TaskStatus::Stopped)
                .unwrap();
        }
    }

    fn new_run(n: u32, rounds: &[u32]) -> NewRun {
        NewRun {
            id: run(n),
            at: at(5),
            session: None,
            rounds: rounds.iter().map(|&n| round(n)).collect(),
        }
    }

    // ---- opening ------------------------------------------------------------------------------------------------

    #[test]
    fn a_round_is_opened_for_an_item_of_a_team_and_found_in_the_state_with_its_team() {
        let mut state = state();

        state
            .open_round(&team_id(1), round(1), Some(item("1.2")), at(1))
            .unwrap();

        let (found_team, found) = state.find_round(&round(1)).unwrap();
        assert_eq!(
            (found_team.id(), found.item(), found.is_open()),
            (&team_id(1), Some(item("1.2")), true)
        );
        assert_eq!(team(&state, 1).round_of(item("1.2")), Some(found));
        assert_eq!(state.find_round(&round(2)), None);
    }

    #[test]
    fn a_team_that_the_state_does_not_have_gets_no_round() {
        let mut state = state();

        let err = state.open_round(&team_id(9), round(1), None, at(1)).unwrap_err();

        assert_eq!(err, StateOpenRoundError::UnknownTeam { team: team_id(9) });
    }

    #[test]
    fn a_round_id_is_used_once_in_the_whole_state_not_only_in_one_team() {
        let mut state = state();
        state
            .open_round(&team_id(1), round(1), Some(item("1.2")), at(1))
            .unwrap();
        let before = state.clone();

        let err = state
            .open_round(&team_id(2), round(1), Some(item("1.1")), at(2))
            .unwrap_err();

        assert_eq!(
            err,
            StateOpenRoundError::DuplicateRound {
                round: round(1),
                team: team_id(1)
            }
        );
        assert_eq!(state, before);
    }

    #[test]
    fn the_rules_of_a_team_for_its_rounds_are_the_rules_of_the_state() {
        let mut state = state();
        state
            .open_round(&team_id(1), round(1), Some(item("1.2")), at(1))
            .unwrap();
        let cases = [
            (
                team_id(1),
                round(2),
                Some(item("1.2")),
                OpenRoundError::ItemHasRound {
                    item: item("1.2"),
                    round: round(1),
                },
            ),
            (
                team_id(1),
                round(2),
                Some(item("1.1")),
                OpenRoundError::ItemDone { item: item("1.1") },
            ),
            (
                team_id(1),
                round(2),
                Some(item("7.7")),
                OpenRoundError::UnknownItem { item: item("7.7") },
            ),
            (team_id(1), round(2), None, OpenRoundError::NoItemInTeamWithItems),
            (
                team_id(3),
                round(2),
                Some(item("1.1")),
                OpenRoundError::UnknownItem { item: item("1.1") },
            ),
        ];
        for (team, id, for_item, expected) in cases {
            let before = state.clone();

            let err = state.open_round(&team, id, for_item, at(2)).unwrap_err();

            assert_eq!(err, StateOpenRoundError::Team(expected));
            assert_eq!(state, before);
        }
    }

    #[test]
    fn a_list_without_items_has_one_round_for_its_whole_work() {
        let mut state = state();

        state.open_round(&team_id(3), round(1), None, at(1)).unwrap();
        let err = state.open_round(&team_id(3), round(2), None, at(2)).unwrap_err();

        assert_eq!(
            err,
            StateOpenRoundError::Team(OpenRoundError::WholeWorkTwice { round: round(1) })
        );
        assert_eq!(team(&state, 3).rounds().len(), 1);
    }

    #[test]
    fn a_team_inserted_with_a_round_of_a_round_id_the_state_has_is_refused() {
        let mut state = state();
        state
            .open_round(&team_id(1), round(1), Some(item("1.2")), at(1))
            .unwrap();
        let mut other = team(&state, 2).clone();
        // The team of the same ID is not insertable twice, so the clash is made with another team's ID.
        let mut renamed = serde_json::to_value(&other).unwrap();
        renamed["id"] = json!(team_id(5).as_str());
        renamed["crew"]["member"]["task"] = json!(task(50).as_str());
        renamed["rounds"] = json!([{
            "id": round(1).as_str(),
            "item": "1.1",
            "opened_at": "2026-11-11T19:30:01Z",
            "state": {"status": "open"}
        }]);
        other = serde_json::from_value(renamed).unwrap();

        let err = state.insert_team(other).unwrap_err();

        assert_eq!(
            err,
            InvalidState::DuplicateRound {
                round: round(1),
                team: team_id(5)
            }
        );
    }

    // ---- closing ------------------------------------------------------------------------------------------------

    #[test]
    fn a_round_is_closed_in_the_state_by_its_id() {
        let mut state = state();
        state
            .open_round(&team_id(1), round(1), Some(item("1.2")), at(1))
            .unwrap();

        stop_crew(&mut state, 1);
        state.close_round(&round(1), at(9), RoundEnd::Abandoned).unwrap();

        let (_, closed) = state.find_round(&round(1)).unwrap();
        assert_eq!(
            closed.life(),
            &RoundLife::Closed {
                at: at(9),
                how: RoundEnd::Abandoned
            }
        );
    }

    #[test]
    fn a_round_the_state_does_not_have_is_not_closed() {
        let mut state = state();

        let err = state.close_round(&round(4), at(9), RoundEnd::Abandoned).unwrap_err();

        assert_eq!(err, CloseRoundError::NoSuchRound { round: round(4) });
    }

    #[test]
    fn the_ways_a_round_may_end_follow_where_its_item_is() {
        for (phase, fitting) in [
            (PhaseKind::InProgress, vec![RoundEnd::Abandoned]),
            (PhaseKind::InReview, vec![RoundEnd::Abandoned]),
            (PhaseKind::Approved, vec![RoundEnd::Abandoned, RoundEnd::Approved]),
            (
                PhaseKind::Done,
                vec![RoundEnd::Abandoned, RoundEnd::Approved, RoundEnd::Merged],
            ),
        ] {
            for how in RoundEnd::ALL {
                let mut state = state();
                state
                    .open_round(&team_id(1), round(1), Some(item("1.2")), at(1))
                    .unwrap();
                drive(&mut state, 1, "1.2", phase);
                if matches!(phase, PhaseKind::InProgress | PhaseKind::InReview) {
                    stop_crew(&mut state, 1);
                }

                let closed = state.close_round(&round(1), at(60), *how);

                assert_eq!(
                    closed.is_ok(),
                    fitting.contains(how),
                    "{how} for an item that is {phase}: {closed:?}"
                );
            }
        }
    }

    // ---- the items of a round -----------------------------------------------------------------------------------

    #[test]
    fn an_item_is_taken_out_of_pending_by_the_team_only_when_it_has_a_round() {
        let mut state = state();
        let start = || ItemPhase::InProgress {
            assignee: "codex-1".parse().unwrap(),
            branch: "agents/codex-1/item-1-2".parse().unwrap(),
        };

        let refused = state
            .team_mut(&team_id(1))
            .unwrap()
            .advance_item(item("1.2"), start())
            .unwrap_err();
        state
            .open_round(&team_id(1), round(1), Some(item("1.2")), at(1))
            .unwrap();
        state
            .team_mut(&team_id(1))
            .unwrap()
            .advance_item(item("1.2"), start())
            .unwrap();

        assert_eq!(refused, AdvanceItemError::NoRound { item: item("1.2") });
        assert_eq!(
            team(&state, 1).item(item("1.2")).unwrap().phase().kind(),
            PhaseKind::InProgress
        );
    }

    #[test]
    fn a_round_is_opened_and_closed_through_the_handle_of_a_team_too() {
        let mut state = state();
        let mut handle = state.team_mut(&team_id(1)).unwrap();

        handle.open_round(round(1), Some(item("1.2")), at(1)).unwrap();
        let tasks: Vec<_> = handle
            .crew()
            .roster()
            .into_iter()
            .map(|(_, member)| member.task().clone())
            .collect();
        for task in tasks {
            handle
                .crew_mut()
                .find_mut(&task)
                .unwrap()
                .1
                .transition(TaskStatus::Stopped)
                .unwrap();
        }
        handle.close_round(&round(1), at(2), RoundEnd::Abandoned).unwrap();

        assert_eq!(
            team(&state, 1).round(&round(1)).and_then(Round::ended),
            Some(RoundEnd::Abandoned)
        );
    }

    // ---- the rounds of a run ------------------------------------------------------------------------------------

    #[test]
    fn a_run_names_rounds_of_its_own_team_only() {
        let mut state = state();
        state
            .open_round(&team_id(1), round(1), Some(item("1.2")), at(1))
            .unwrap();
        state
            .open_round(&team_id(2), round(2), Some(item("1.1")), at(1))
            .unwrap();
        let before = state.clone();

        let other_team = state.begin_run(&task(2), new_run(1, &[2])).unwrap_err();
        let nobody = state.begin_run(&task(50), new_run(1, &[1])).unwrap_err();
        state.begin_run(&task(2), new_run(1, &[1])).unwrap();

        assert_eq!(other_team, BeginRunError::UnknownRound { round: round(2) });
        assert_eq!(nobody, BeginRunError::UnknownTask { task: task(50) });
        assert_ne!(state, before);
        let run = state
            .find_task(&task(2))
            .unwrap()
            .member
            .runs()
            .last()
            .cloned()
            .unwrap();
        assert_eq!(run.rounds(), [round(1)]);
    }

    #[test]
    fn a_run_names_no_round_twice_and_nothing_is_recorded_when_it_does() {
        let mut state = state();
        state
            .open_round(&team_id(1), round(1), Some(item("1.2")), at(1))
            .unwrap();
        let before = state.clone();

        let err = state.begin_run(&task(2), new_run(1, &[1, 1])).unwrap_err();

        assert_eq!(
            err,
            BeginRunError::Run(RunError::RoundTwice {
                run: run(1),
                round: round(1)
            })
        );
        assert_eq!(state, before);
    }

    // ---- reading ------------------------------------------------------------------------------------------------

    /// A change that makes a state one no operation could have made.
    type Mutation = fn(&mut Value);

    /// A state with rounds in every way a round can be, as JSON: team 1 has the items 1.2 (merged) and 1.3 (in
    /// review), team 2 has 1.1 approved, and team 3 has the round of its whole work, abandoned.
    fn full() -> Value {
        let mut state = state();
        state
            .open_round(&team_id(1), round(1), Some(item("1.2")), at(1))
            .unwrap();
        state
            .open_round(&team_id(1), round(2), Some(item("1.3")), at(2))
            .unwrap();
        state
            .open_round(&team_id(2), round(3), Some(item("1.1")), at(1))
            .unwrap();
        state.open_round(&team_id(3), round(4), None, at(1)).unwrap();
        drive(&mut state, 1, "1.2", PhaseKind::Done);
        drive(&mut state, 1, "1.3", PhaseKind::InReview);
        drive(&mut state, 2, "1.1", PhaseKind::Approved);
        state.close_round(&round(1), at(60), RoundEnd::Merged).unwrap();
        state.close_round(&round(3), at(60), RoundEnd::Approved).unwrap();
        stop_crew(&mut state, 3);
        state.close_round(&round(4), at(60), RoundEnd::Abandoned).unwrap();
        state.begin_run(&task(2), new_run(1, &[1, 2])).unwrap();
        State::from_json(&state.to_json().unwrap()).unwrap();
        as_json(&state)
    }

    #[test]
    fn a_state_with_every_kind_of_round_is_read_back_as_it_was_written() {
        let value = full();

        let state = State::from_json(&value.to_string()).unwrap();

        assert_eq!(as_json(&state), value);
        assert_eq!(team(&state, 1).rounds().len(), 2);
    }

    #[test]
    fn a_state_whose_rounds_break_a_rule_of_the_operations_is_refused_when_read() {
        let mutations: Vec<(&str, Mutation)> = vec![
            ("a round id twice in a team", |v| {
                v["teams"][0]["rounds"][1]["id"] = v["teams"][0]["rounds"][0]["id"].clone();
            }),
            ("a round of an item the team does not have", |v| {
                v["teams"][0]["rounds"][1]["item"] = json!("9.9");
            }),
            ("an item with two rounds", |v| {
                v["teams"][0]["rounds"][1]["item"] = json!("1.2");
            }),
            ("a round of an item the list marks done", |v| {
                v["teams"][0]["rounds"][1]["item"] = json!("1.1");
            }),
            ("a round without an item in a list with items", |v| {
                v["teams"][0]["rounds"][1].as_object_mut().unwrap().remove("item");
            }),
            ("two rounds of the whole work", |v| {
                let copy =
                    json!({"id": round(40).as_str(), "opened_at": "2026-11-11T19:30:01Z", "state": {"status": "open"}});
                v["teams"][2]["rounds"].as_array_mut().unwrap().push(copy);
            }),
            ("an item in review without a round", |v| {
                v["teams"][0]["rounds"].as_array_mut().unwrap().remove(1);
            }),
            ("an item approved without a round", |v| {
                v["teams"][1]["rounds"] = json!([]);
            }),
            ("a round closed as merged for an item in review", |v| {
                v["teams"][0]["rounds"][1]["state"] =
                    json!({"status": "closed", "at": "2026-11-11T19:31:00Z", "how": "merged"});
            }),
            ("a round closed as approved for an item in review", |v| {
                v["teams"][0]["rounds"][1]["state"] =
                    json!({"status": "closed", "at": "2026-11-11T19:31:00Z", "how": "approved"});
            }),
            ("the whole work closed as merged", |v| {
                v["teams"][2]["rounds"][0]["state"]["how"] = json!("merged");
            }),
            ("a round closed before it was opened", |v| {
                v["teams"][0]["rounds"][0]["state"]["at"] = json!("2026-11-11T19:29:00Z");
            }),
            ("a run that names a round the team does not have", |v| {
                v["teams"][0]["crew"]["members"][0]["runs"][0]["rounds"] = json!([round(3).as_str()]);
            }),
            ("a run that names a round twice", |v| {
                let first = v["teams"][0]["rounds"][0]["id"].clone();
                v["teams"][0]["crew"]["members"][0]["runs"][0]["rounds"] = json!([first, first]);
            }),
        ];
        for (what, mutate) in mutations {
            let mut value = full();
            mutate(&mut value);

            let err = read(&value);

            assert!(matches!(err, StateError::Malformed { .. }), "{what}: {err:?}");
        }
    }

    #[test]
    fn a_round_id_in_two_teams_is_refused_when_read_with_the_team_that_repeats_it() {
        let mut value = full();
        value["teams"][1]["rounds"][0]["id"] = value["teams"][0]["rounds"][0]["id"].clone();

        let err = read(&value);

        assert!(
            matches!(
                &err,
                StateError::Invalid(InvalidState::DuplicateRound { round: found, team })
                    if *found == round(1) && *team == team_id(2)
            ),
            "{err:?}"
        );
    }

    #[test]
    fn a_round_is_a_round_id_not_another_id() {
        let mut value = full();
        value["teams"][0]["rounds"][0]["id"] = json!(team_id(1).as_str());

        let err = read(&value);

        assert!(matches!(err, StateError::Malformed { .. }), "{err:?}");
        assert_eq!(
            "round-20261111-193001-q1w2".parse::<RoundId>().unwrap().as_str(),
            "round-20261111-193001-q1w2"
        );
    }
}
