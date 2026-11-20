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
