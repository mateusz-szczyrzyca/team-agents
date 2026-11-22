use super::*;
use crate::state::{fixtures::team, split::PartDefinition};

fn plan(parent: &str) -> SplitPlan {
    SplitPlan::new(
        parent.parse().expect("item"),
        ["Draft", "Publish"]
            .into_iter()
            .map(|title| PartDefinition {
                title: title.into(),
                delivers: "A document".into(),
                checks: "A reviewer approves it".into(),
                depends_on: vec![],
            })
            .collect(),
    )
    .expect("plan")
}

#[test]
fn team_records_whole_plans_in_order_without_changing_items_or_rounds() {
    let mut team = team(1);
    let original_items = team.items().to_vec();
    let original_rounds = team.rounds().to_vec();
    let first = plan("1.2");
    let second = plan("2.1");
    team.record_split(first.clone()).expect("first split");
    team.record_split(second.clone()).expect("second split");
    assert_eq!(team.splits(), &[first, second]);
    assert_eq!(team.items(), original_items);
    assert_eq!(team.rounds(), original_rounds);
    assert_eq!(
        serde_json::from_str::<Team>(&serde_json::to_string(&team).expect("serialize")).expect("deserialize"),
        team
    );
}

#[test]
fn team_refuses_unknown_and_duplicate_plans_atomically() {
    let mut team = team(1);
    assert_eq!(
        team.record_split(plan("9.9")),
        Err(RecordSplitError::UnknownItem {
            item: "9.9".parse().expect("item")
        })
    );
    assert_eq!(team.splits(), &[]);
    team.record_split(plan("1.2")).expect("first plan");
    let snapshot = team.clone();
    assert_eq!(
        team.record_split(plan("1.2")),
        Err(RecordSplitError::AlreadySplit {
            item: "1.2".parse().expect("item")
        })
    );
    assert_eq!(team, snapshot);
}

#[test]
fn deserializing_team_validates_split_membership_uniqueness_and_plan_invariants() {
    let value = serde_json::to_value(team(1)).expect("serialize");
    for plans in [
        serde_json::json!([plan("9.9")]),
        serde_json::json!([plan("1.2"), plan("1.2")]),
        serde_json::json!([{"parent":"1.2","parts":[]}]),
    ] {
        let mut invalid = value.clone();
        invalid["splits"] = plans;
        let error = serde_json::from_value::<Team>(invalid).expect_err("invalid persisted plan");
        assert_eq!(error.classify(), serde_json::error::Category::Data);
    }
    assert_eq!(
        serde_json::from_value::<Team>(value)
            .expect("old team without plans")
            .splits(),
        &[]
    );
}

#[test]
fn old_state_from_20261121_case_3_stopped_member_on_split_part_loads_and_continues() {
    let raw = serde_json::json!({
        "schema_version": 14,
        "teams": [
            {
                "id": "team-20261121-120000-abcd",
                "repo": "example",
                "created_at": "2026-11-21T12:00:00Z",
                "merge": "autopilot",
                "task_list": "Build the feature.\n\n1. First\n[ ] 1.1 The first item\n",
                "items": [
                    {
                        "id": "1.1",
                        "phase": {
                            "phase": "pending"
                        }
                    }
                ],
                "rounds": [
                    {
                        "id": "round-20261121-120001-aaaa",
                        "item": "1.1",
                        "opened_at": "2026-11-21T12:00:01Z",
                        "state": {
                            "status": "closed",
                            "at": "2026-11-21T12:05:00Z",
                            "how": "merged"
                        },
                        "part": {
                            "status": "merged",
                            "key": "a",
                            "pr": {
                                "number": 1,
                                "branch": "agents/claude-1/task-20261121-120002-m111-1.1a",
                                "opened_at": "2026-11-21T12:04:00Z"
                            }
                        }
                    }
                ],
                "crew": {
                    "kind": "team",
                    "coordinator": {
                        "task": "task-20261121-120002-lead",
                        "agent": {
                            "tier": "normal",
                            "name": "lead"
                        },
                        "status": "waiting",
                        "runs": []
                    },
                    "members": [
                        {
                            "task": "task-20261121-120002-m111",
                            "agent": {
                                "tier": "normal",
                                "name": "claude-1"
                            },
                            "status": "stopped",
                            "runs": []
                        }
                    ],
                    "workers": []
                },
                "splits": [
                    {
                        "parent": "1.1",
                        "parts": [
                            {
                                "title": "Part A",
                                "delivers": "Part A delivered",
                                "checks": "Checks",
                                "depends_on": []
                            },
                            {
                                "title": "Part B",
                                "delivers": "Part B delivered",
                                "checks": "Checks",
                                "depends_on": ["a"]
                            }
                        ]
                    }
                ]
            }
        ]
    });
    let state = crate::state::State::from_json(&serde_json::to_string(&raw).unwrap()).expect("loads legacy state");
    let team = &state.teams()[0];
    assert_eq!(team.next_part("1.1".parse().unwrap()).unwrap().0.to_string(), "1.1b");
    let dispatched = crate::decide::assign::dispatch::dispatch(team);
    assert_eq!(dispatched.assigned, vec![("1.1".parse().unwrap(), crate::state::fixtures::normal("lead"))]);
}
