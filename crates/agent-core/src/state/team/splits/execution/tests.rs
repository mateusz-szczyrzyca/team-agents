use super::*;
use crate::state::fixtures::{at, drive, pr, round_id, team};
use crate::state::split::PartDefinition;
use crate::state::{ItemPhase, PhaseKind, RoundEnd};

fn plan(dependencies: &[&[&str]]) -> SplitPlan {
    SplitPlan::new(
        "1.2".parse().unwrap(),
        dependencies
            .iter()
            .enumerate()
            .map(|(index, depends)| PartDefinition {
                title: format!("Deliverable {index}"),
                delivers: "An independently useful document — résumé".into(),
                checks: "A reviewer accepts the document".into(),
                depends_on: depends
                    .iter()
                    .map(|key| (*key).to_owned().try_into().unwrap())
                    .collect(),
            })
            .collect(),
    )
    .unwrap()
}

fn working() -> Team {
    let mut found = team(1);
    found
        .open_round(round_id(1), Some("1.2".parse().unwrap()), at(1))
        .unwrap();
    drive(&mut found, "1.2", PhaseKind::InProgress);
    found
}

fn round_trip(found: &Team) -> Team {
    let text = serde_json::to_string(found).unwrap();
    let read: Team = serde_json::from_str(&text).unwrap();
    assert_eq!(&read, found);
    read
}

#[test]
fn approved_split_binds_first_round_and_retains_each_merged_part_until_last() {
    let parent = "1.2".parse().unwrap();
    let mut found = working();
    let approved = plan(&[&[], &["a"], &["a"]]);
    assert_eq!(found.activate_split(approved.clone()), Ok(()));
    assert_eq!(found.splits(), &[approved]);
    assert_eq!(found.round_of(parent).unwrap().part().unwrap().to_string(), "a");
    found = round_trip(&found);
    for (number, key) in [(1, "a"), (2, "b"), (3, "c")] {
        if number > 1 {
            found.open_round(round_id(number), Some(parent), at(60)).unwrap();
            assert_eq!(found.round_of(parent).unwrap().part().unwrap().to_string(), key);
            drive(&mut found, "1.2", PhaseKind::Done);
        } else {
            // Continue the already-working first part through the public state operations.
            let mut item = found.item_mut(parent).unwrap();
            item.advance(ItemPhase::InReview {
                pr: pr(7),
                reviews: vec![],
            })
            .unwrap();
            let reviewer = crate::state::fixtures::review("inv-codex", 21, crate::state::ReviewOutcome::Pending);
            item.ask_reviews(vec![reviewer]).unwrap();
            item.submit_review(
                &crate::state::fixtures::task(21),
                crate::status::ReviewVerdict::Approve,
                at(40),
            )
            .unwrap();
            let ItemPhase::InReview { reviews, .. } = item.phase().clone() else {
                panic!("in review")
            };
            item.advance(ItemPhase::Approved { pr: pr(7), reviews }).unwrap();
            item.advance(ItemPhase::Done {
                completion: crate::state::Completion::Merged { pr: pr(7), at: at(50) },
            })
            .unwrap();
        }
        assert_eq!(found.close_round(&round_id(number), at(70), RoundEnd::Merged), Ok(()));
        assert_eq!(
            found.round(&round_id(number)).unwrap().part_pull_request(),
            Some(&pr(7))
        );
        assert_eq!(
            found.item(parent).unwrap().phase().kind(),
            if number == 3 {
                PhaseKind::Done
            } else {
                PhaseKind::Pending
            }
        );
        found = round_trip(&found);
    }
    assert_eq!(found.rounds().len(), 3);
}

#[test]
fn activation_refuses_nonworking_duplicate_unknown_and_blocked_first_atomically() {
    let mut idle = team(1);
    let snapshot = idle.clone();
    assert_eq!(
        idle.activate_split(plan(&[&[], &["a"]])),
        Err(ActivateSplitError::NotWorking {
            item: "1.2".parse().unwrap()
        })
    );
    assert_eq!(idle, snapshot);
    let mut found = working();
    let snapshot = found.clone();
    assert_eq!(
        found.activate_split(plan(&[&["b"], &[]])),
        Err(ActivateSplitError::FirstBlocked)
    );
    assert_eq!(found, snapshot);
    found.activate_split(plan(&[&[], &["a"]])).unwrap();
    let snapshot = found.clone();
    assert_eq!(
        found.activate_split(plan(&[&[], &["a"]])),
        Err(ActivateSplitError::Record(RecordSplitError::AlreadySplit {
            item: "1.2".parse().unwrap()
        }))
    );
    assert_eq!(found, snapshot);
    let unknown = SplitPlan::new("9.9".parse().unwrap(), plan(&[&[], &[]]).parts().to_vec()).unwrap();
    assert_eq!(
        found.activate_split(unknown),
        Err(ActivateSplitError::Record(RecordSplitError::UnknownItem {
            item: "9.9".parse().unwrap()
        }))
    );
    assert_eq!(found, snapshot);
}

#[test]
fn persisted_part_rounds_require_a_plan_known_unique_keys_and_merged_dependencies() {
    let mut found = working();
    found.activate_split(plan(&[&[], &["a"]])).unwrap();
    let original = serde_json::to_value(&found).unwrap();
    let mut cases = Vec::new();
    let mut no_plan = original.clone();
    no_plan["splits"] = serde_json::json!([]);
    cases.push(no_plan);
    let mut unknown = original.clone();
    unknown["rounds"][0]["part"]["key"] = serde_json::json!("z");
    cases.push(unknown);
    let mut blocked = original.clone();
    blocked["rounds"][0]["part"]["key"] = serde_json::json!("b");
    cases.push(blocked);
    let mut premature = original.clone();
    premature["rounds"][0]["part"] = serde_json::json!({"status":"merged", "key":"a", "pr":pr(7)});
    cases.push(premature);
    let mut duplicate = original;
    let mut second = duplicate["rounds"][0].clone();
    second["id"] = serde_json::to_value(round_id(2)).unwrap();
    duplicate["rounds"].as_array_mut().unwrap().push(second);
    cases.push(duplicate);
    for value in cases {
        let read = serde_json::from_value::<Team>(value);
        assert!(matches!(read, Err(error) if error.is_data()));
    }
}

#[test]
fn persisted_active_split_cannot_lose_a_later_rounds_part_identity() {
    let mut found = working();
    found.activate_split(plan(&[&[], &["a"], &["b"]])).unwrap();
    let mut wire = serde_json::to_value(found).unwrap();
    let mut first = wire["rounds"][0].clone();
    first["state"] = serde_json::json!({"status":"closed", "at":at(2), "how":"merged"});
    first["part"] = serde_json::json!({"status":"merged", "key":"a", "pr":pr(1)});
    let mut second = first.clone();
    second["id"] = serde_json::to_value(round_id(2)).unwrap();
    second["part"] = serde_json::json!({"status":"merged", "key":"b", "pr":pr(2)});
    let mut third = wire["rounds"][0].clone();
    third["id"] = serde_json::to_value(round_id(3)).unwrap();
    third.as_object_mut().unwrap().remove("part");
    wire["rounds"] = serde_json::json!([first, second, third]);
    let read = serde_json::from_value::<Team>(wire);
    assert!(matches!(read, Err(error) if error.is_data()));
}

#[test]
fn persisted_split_cannot_have_two_open_parts_of_one_item() {
    let mut found = working();
    found.activate_split(plan(&[&[], &[]])).unwrap();
    let mut wire = serde_json::to_value(found).unwrap();
    let mut second = wire["rounds"][0].clone();
    second["id"] = serde_json::to_value(round_id(2)).unwrap();
    second["part"]["key"] = serde_json::json!("b");
    wire["rounds"].as_array_mut().unwrap().push(second);
    let read = serde_json::from_value::<Team>(wire);
    assert!(matches!(read, Err(error) if error.is_data()));
}

#[test]
fn part_assignment_survives_review_and_reload_and_tracks_a_reassigned_executor() {
    use crate::state::fixtures::{agent, run, task};
    use crate::state::{NewRun, Round};
    let mut found = working();
    found.activate_split(plan(&[&[], &["a"]])).unwrap();
    assert_eq!(found.part_assignments().get(&round_id(1)), Some(&agent("codex-1")));
    found
        .item_mut("1.2".parse().unwrap())
        .unwrap()
        .reassign(agent("inv-opus"), "agents/inv-opus/part-a".parse().unwrap())
        .unwrap();
    found
        .begin_run(
            &task(1),
            NewRun {
                id: run(1),
                at: at(5),
                session: None,
                rounds: vec![round_id(1)],
            },
        )
        .unwrap();
    found
        .item_mut("1.2".parse().unwrap())
        .unwrap()
        .advance(ItemPhase::InReview {
            pr: pr(7),
            reviews: vec![],
        })
        .unwrap();
    let found = round_trip(&found);
    assert_eq!(found.part_assignments().get(&round_id(1)), Some(&agent("inv-opus")));
    let legacy: Round = serde_json::from_str(include_str!("before-assignment.json")).unwrap();
    assert_eq!(serde_json::to_value(&legacy).unwrap().get("assigned_to"), None);
    assert_eq!(
        serde_json::to_value(legacy).unwrap(),
        serde_json::from_str::<serde_json::Value>(include_str!("before-assignment.json")).unwrap()
    );
}

#[test]
fn executor_is_stored_at_team_level_and_round_format_is_unchanged() {
    let mut found = working();
    found.activate_split(plan(&[&[], &["a"]])).unwrap();
    let wire = serde_json::to_value(&found).unwrap();
    assert_eq!(wire["part_assignments"][round_id(1).as_str()], "codex-1");
    assert_eq!(wire["rounds"][0].get("assigned_to"), None);
}
