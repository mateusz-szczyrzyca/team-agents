//! The state of schema version 2: a fixture written by this format and read back as it is, the migration of version 1
//! (the fixture `state-v1.json` loads as exactly `state-v1-as-v2.json`, every time), and the versions that are refused.

#[cfg(test)]
mod state_v2 {
    use agent_core::ids::RoundId;
    use std::fmt::Write as _;

    use agent_core::state::{
        Asked, ChecksRecord, InvalidState, RoundEnd, RoundLife, State, StateError, Team, Timestamp, UnsupportedSchema,
    };
    use serde_json::{Value, json};

    /// A state of version 2 with every kind of round.
    const V2: &str = include_str!("fixtures/state-v2.json");
    /// A state of version 1, as item 2.2 fixed it.
    const V1: &str = include_str!("fixtures/state-v1.json");
    /// What `V1` stands for as version 2.
    const V1_AS_V2: &str = include_str!("fixtures/state-v1-as-v2.json");

    fn at(text: &str) -> Timestamp {
        text.parse().unwrap()
    }

    fn round(text: &str) -> RoundId {
        text.parse().unwrap()
    }

    /// `V1` as a JSON value.
    fn v1() -> Value {
        serde_json::from_str(V1).unwrap()
    }

    /// The state `value` of version 1 as it is read, as JSON.
    fn migrated(value: &Value) -> Value {
        let state = State::from_json(&value.to_string()).unwrap();
        serde_json::from_str(&state.to_json().unwrap()).unwrap()
    }

    // ---- version 2 ----------------------------------------------------------------------------------------------

    #[test]
    fn a_state_of_schema_version_2_is_written_back_as_it_was_read() {
        let state = State::from_json(V2).unwrap();

        assert_eq!(state.to_json().unwrap(), V2);
    }

    #[test]
    fn the_fixture_of_version_2_has_a_round_of_every_kind_and_the_runs_that_name_them() {
        let state = State::from_json(V2).unwrap();

        let ends: Vec<(String, Option<RoundEnd>, bool)> = state
            .teams()
            .iter()
            .flat_map(Team::rounds)
            .map(|round| (round.id().to_string(), round.ended(), round.item().is_some()))
            .collect();

        assert_eq!(
            ends,
            [
                ("round-20261111-193005-m4k2".to_owned(), Some(RoundEnd::Merged), true),
                ("round-20261111-193900-p7w3".to_owned(), None, true),
                ("round-20261111-194500-c3d5".to_owned(), Some(RoundEnd::Approved), true),
                (
                    "round-20261111-200000-q9z1".to_owned(),
                    Some(RoundEnd::Abandoned),
                    false
                ),
            ]
        );
        let (_, reviewer_codex) = {
            let team = &state.teams()[0];
            team.find(&team.reviewers()[0].task().clone()).unwrap()
        };
        assert_eq!(reviewer_codex.runs()[0].rounds(), [round("round-20261111-193900-p7w3")]);
        let coordinator = state.teams()[0].crew().coordinator().unwrap();
        assert_eq!(
            coordinator.runs()[0].rounds(),
            [round("round-20261111-193005-m4k2"), round("round-20261111-193900-p7w3")]
        );
    }

    #[test]
    fn a_state_of_schema_version_3_is_refused_as_an_unsupported_version_whatever_else_it_holds() {
        for text in [
            V2.replacen("\"schema_version\": 2", "\"schema_version\": 3", 1),
            "{\"schema_version\": 3, \"teams\": {\"by_id\": 5}}".to_owned(),
            "{\"teams\": [], \"schema_version\": 3, \"future\": true}".to_owned(),
        ] {
            let err = State::from_json(&text).unwrap_err();

            assert!(
                matches!(err, StateError::UnsupportedSchema(UnsupportedSchema { found: 3 })),
                "{err:?}"
            );
        }
    }

    #[test]
    fn a_state_of_version_2_with_a_field_it_does_not_have_is_refused() {
        let mut team_with = serde_json::from_str::<Value>(V2).unwrap();
        team_with["teams"][0]["extra"] = json!(1);
        let mut round_with = serde_json::from_str::<Value>(V2).unwrap();
        round_with["teams"][0]["rounds"][0]["extra"] = json!(1);
        let mut question_with = serde_json::from_str::<Value>(V2).unwrap();
        question_with["teams"][0]["crew"]["members"][0]["questions"][0]["round"] = json!(1);

        for value in [team_with, round_with, question_with] {
            let err = State::from_json(&value.to_string()).unwrap_err();

            assert!(matches!(err, StateError::Malformed { .. }), "{err:?}");
        }
    }

    // ---- the migration of version 1 -----------------------------------------------------------------------------

    #[test]
    fn a_state_of_version_1_loads_as_exactly_the_state_of_version_2_it_stands_for() {
        let state = State::from_json(V1).unwrap();

        assert_eq!(state.to_json().unwrap(), V1_AS_V2);
        assert_eq!(State::from_json(V1_AS_V2).unwrap(), state);
    }

    #[test]
    fn reading_the_same_state_of_version_1_twice_gives_the_same_state() {
        let first = State::from_json(V1).unwrap();
        let second = State::from_json(V1).unwrap();

        assert_eq!(first, second);
        assert_eq!(first.to_json().unwrap(), second.to_json().unwrap());
    }

    #[test]
    fn a_migrated_state_is_saved_as_version_2_and_loads_again() {
        let state = State::from_json(V1).unwrap();

        let saved = state.to_json().unwrap();

        assert!(saved.starts_with("{\n  \"schema_version\": 2,\n"));
        assert_eq!(State::from_json(&saved).unwrap(), state);
    }

    #[test]
    fn each_item_past_pending_gets_a_round_whose_id_is_derived_from_its_team_and_its_place() {
        let state = State::from_json(V1).unwrap();

        let ids: Vec<(String, String)> = state
            .teams()
            .iter()
            .flat_map(|team| {
                team.rounds()
                    .iter()
                    .map(|round| (round.id().to_string(), round.item().unwrap().to_string()))
            })
            .collect();

        // Team a1b2: item 1.1 is done as listed, 1.2 (place 1) is in review, 2.1 is pending. Team c3d4: 1.1 (place 0).
        assert_eq!(
            ids,
            [
                ("round-20261111-193000-a1b4".to_owned(), "1.2".to_owned()),
                ("round-20261111-194500-c3d5".to_owned(), "1.1".to_owned()),
            ]
        );
        assert_eq!(state.teams()[0].round_of("1.1".parse().unwrap()), None);
        assert_eq!(state.teams()[0].round_of("2.1".parse().unwrap()), None);
    }

    #[test]
    fn a_migrated_round_is_opened_when_its_team_was_started_and_is_open_while_its_item_is_not_done() {
        let state = State::from_json(V1).unwrap();

        for team in state.teams() {
            for round in team.rounds() {
                assert_eq!((round.opened_at(), round.life()), (team.created_at(), &RoundLife::Open));
            }
        }
    }

    #[test]
    fn the_runs_of_version_1_name_no_round_and_its_questions_are_numbered() {
        let state = State::from_json(V1).unwrap();

        for team in state.teams() {
            for (_, member) in team.crew().roster() {
                assert!(member.runs().iter().all(|run| run.rounds().is_empty()));
            }
        }
        let asking = state
            .find_task(&"task-20261111-193011-k3x9".parse().unwrap())
            .unwrap()
            .member;
        assert_eq!(asking.open_question().map(|question| question.number().get()), Some(1));
    }

    #[test]
    fn an_item_that_was_merged_gets_a_round_closed_as_merged_at_the_time_of_the_merge() {
        let mut value = v1();
        value["teams"][0]["items"][1]["phase"] = json!({
            "phase": "done",
            "completion": {
                "how": "merged",
                "pr": {"number": 82, "branch": "agents/codex-1/item-1-2", "opened_at": "2026-11-11T19:40:00Z"},
                "at": "2026-11-11T19:55:00Z"
            }
        });
        value["teams"][0]["items"][1]
            .as_object_mut()
            .unwrap()
            .remove("fix_passes");

        let state = State::from_json(&value.to_string()).unwrap();

        let round = &state.teams()[0].rounds()[0];
        assert_eq!(
            round.life(),
            &RoundLife::Closed {
                at: at("2026-11-11T19:55:00Z"),
                how: RoundEnd::Merged
            }
        );
    }

    #[test]
    fn a_merge_dated_before_the_team_was_started_closes_its_round_when_it_was_opened() {
        let mut value = v1();
        value["teams"][0]["items"][1]["phase"] = json!({
            "phase": "done",
            "completion": {
                "how": "merged",
                "pr": {"number": 82, "branch": "agents/codex-1/item-1-2", "opened_at": "2026-11-11T19:40:00Z"},
                "at": "2026-11-11T18:00:00Z"
            }
        });
        value["teams"][0]["items"][1]
            .as_object_mut()
            .unwrap()
            .remove("fix_passes");

        let state = State::from_json(&value.to_string()).unwrap();

        let round = &state.teams()[0].rounds()[0];
        assert_eq!(
            round.life(),
            &RoundLife::Closed {
                at: round.opened_at(),
                how: RoundEnd::Merged
            }
        );
    }

    /// The phase `approved` for an item, with approvals at the given times.
    fn approved(times: [&str; 2]) -> Value {
        let review = |name: &str, task: &str, at: &str| {
            json!({
                "reviewer": name,
                "task": task,
                "requested_at": "2026-11-11T19:41:00Z",
                "outcome": {"status": "submitted", "verdict": "APPROVE", "at": at}
            })
        };
        json!({
            "phase": "approved",
            "pr": {"number": 82, "branch": "agents/codex-1/item-1-2", "opened_at": "2026-11-11T19:40:00Z"},
            "reviews": [
                review("inv-codex", "task-20261111-193007-k3x9", times[0]),
                review("inv-gemini", "task-20261111-193008-k3x9", times[1])
            ]
        })
    }

    #[test]
    fn an_approved_item_of_a_team_that_does_not_merge_by_itself_gets_a_round_closed_as_approved_at_its_last_review() {
        let mut value = v1();
        value["teams"][0]["merge"] = json!("no_merge");
        value["teams"][0]["items"][1]["phase"] = approved(["2026-11-11T19:50:00Z", "2026-11-11T19:52:00Z"]);
        value["teams"][0]["items"][1]
            .as_object_mut()
            .unwrap()
            .remove("fix_passes");

        let state = State::from_json(&value.to_string()).unwrap();

        assert_eq!(
            state.teams()[0].rounds()[0].life(),
            &RoundLife::Closed {
                at: at("2026-11-11T19:52:00Z"),
                how: RoundEnd::Approved
            }
        );
    }

    #[test]
    fn an_approved_item_of_a_team_that_merges_by_itself_keeps_its_round_open_until_the_merge() {
        let mut value = v1();
        value["teams"][0]["items"][1]["phase"] = approved(["2026-11-11T19:50:00Z", "2026-11-11T19:52:00Z"]);
        value["teams"][0]["items"][1]
            .as_object_mut()
            .unwrap()
            .remove("fix_passes");

        let state = State::from_json(&value.to_string()).unwrap();

        assert_eq!(state.teams()[0].rounds()[0].life(), &RoundLife::Open);
    }

    #[test]
    fn a_list_without_items_of_version_1_gets_no_round() {
        let mut value = v1();
        value["teams"][1]["task_list"] = json!("Fix the typo in the README.\n");
        value["teams"][1]["items"] = json!([]);

        let state = State::from_json(&value.to_string()).unwrap();

        assert_eq!(state.teams()[1].rounds(), []);
    }

    #[test]
    fn the_rounds_of_two_teams_whose_derived_ids_meet_get_different_ids_in_the_order_of_the_file() {
        // The second team is made to start at the same second as the first with the next suffix: the round of its first
        // item would be the first team's round of its second.
        let mut value = v1();
        value["teams"][1]["id"] = json!("team-20261111-193000-a1b3");

        let migrated = migrated(&value);

        let ids: Vec<&str> = migrated["teams"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|team| team["rounds"].as_array().unwrap())
            .map(|round| round["id"].as_str().unwrap())
            .collect();
        assert_eq!(ids, ["round-20261111-193000-a1b4", "round-20261111-193000-a1b5"]);
        assert_eq!(migrated, migrated_again(&value));
    }

    /// The same as [`migrated`], to show that the same text always gives the same state.
    fn migrated_again(value: &Value) -> Value {
        migrated(value)
    }

    #[test]
    fn a_state_in_which_no_free_id_is_left_for_a_round_is_refused_naming_the_team_and_the_item() {
        // Team a1b2 has eight items in progress at the places 1 to 8: the IDs that team a1b3 would try for its only item,
        // one after the other, are all taken.
        let mut value = v1();
        let first = value["teams"][0].clone();
        let mut items = vec![json!({"id": "1.1", "phase": {"phase": "pending"}})];
        let mut list = String::from("Build.\n\n1. First\n[ ] 1.1 Zero\n    Do.\n");
        for n in 2..=9 {
            items.push(json!({"id": format!("1.{n}"), "phase": {"phase": "in_progress", "assignee": "codex-1", "branch": format!("agents/codex-1/item-1-{n}")}}));
            writeln!(list, "[ ] 1.{n} Item {n}\n    Do.").unwrap();
        }
        let mut crowded = first;
        crowded["items"] = Value::Array(items);
        crowded["task_list"] = json!(list);
        crowded["reviewers"] = json!([]);
        value["teams"][0] = crowded;
        value["teams"][1]["id"] = json!("team-20261111-193000-a1b3");

        let err = State::from_json(&value.to_string()).unwrap_err();

        assert!(
            matches!(&err, StateError::Invalid(InvalidState::NoFreeRoundId { team, item })
                if team.as_str() == "team-20261111-193000-a1b3" && item.to_string() == "1.1"),
            "{err:?}"
        );
    }

    #[test]
    fn a_state_of_version_1_with_a_field_it_did_not_have_is_refused() {
        let mut with_rounds = v1();
        with_rounds["teams"][0]["rounds"] = json!([]);
        let mut with_run_rounds = v1();
        with_run_rounds["teams"][0]["crew"]["members"][0]["runs"][0]["rounds"] = json!([]);
        let mut with_number = v1();
        with_number["teams"][0]["crew"]["members"][0]["questions"][0]["number"] = json!(1);
        let mut with_extra = v1();
        with_extra["teams"][0]["extra"] = json!(true);
        let mut without_teams = v1();
        without_teams.as_object_mut().unwrap().remove("teams");

        for value in [with_rounds, with_run_rounds, with_number, with_extra, without_teams] {
            let err = State::from_json(&value.to_string()).unwrap_err();

            assert!(matches!(err, StateError::Malformed { .. }), "{err:?}");
        }
    }

    #[test]
    fn a_state_of_version_1_that_breaks_a_rule_of_version_1_is_still_refused() {
        let mut value = v1();
        value["teams"][0]["items"] = json!([]);

        let err = State::from_json(&value.to_string()).unwrap_err();

        assert!(matches!(err, StateError::Malformed { .. }), "{err:?}");
    }

    // ---- the record of the checks -------------------------------------------------------------------------------

    #[test]
    fn a_checks_record_written_with_the_key_round_is_read_and_written_with_the_key_number() {
        let old =
            json!({"asked": {"round": 2, "at": "2026-11-11T19:41:00Z", "items": ["1.2"], "subjects": [], "relays": 0}});

        let record: ChecksRecord = serde_json::from_value(old).unwrap();

        let asked: &Asked = record.asked.as_ref().unwrap();
        assert_eq!(asked.number.get(), 2);
        let written = serde_json::to_value(&record).unwrap();
        assert_eq!(written["asked"]["number"], json!(2));
        assert_eq!(written["asked"].get("round"), None);
    }
}
