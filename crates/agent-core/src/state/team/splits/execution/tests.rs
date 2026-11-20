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
