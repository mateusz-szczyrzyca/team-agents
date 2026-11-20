use super::*;
use crate::state::{
    fixtures::team,
    split::{PartDefinition, SplitPlan, keeping::ProposalStage},
};

fn plan() -> SplitPlan {
    SplitPlan::new(
        "1.2".parse().expect("item"),
        ["Draft", "Publish"]
            .into_iter()
            .map(|title| PartDefinition {
                title: title.into(),
                delivers: "Document".into(),
                checks: "Read it".into(),
                depends_on: vec![],
            })
            .collect(),
    )
    .expect("plan")
}

#[test]
fn team_proposal_is_durable_replaceable_and_removed_by_whole_approval() {
    let mut team = team(1);
    let original_items = team.items().to_vec();
    let original_rounds = team.rounds().to_vec();
    let item = "1.2".parse().expect("item");
    assert_eq!(team.split_proposal(item), None);
    let initial = ProposalKeeping::new("raw malformed proposal".into()).expect("input");
    team.keep_split_proposal(item, initial.clone())
        .expect("save before decision");
    let mut restarted: Team = serde_json::from_value(serde_json::to_value(&team).expect("write")).expect("restart");
    assert_eq!(restarted.split_proposal(item), Some(&initial));
    let mut correction = initial;
    correction.send_back(crate::decide::split::approval::ProposalFault::Unreadable { part: None });
    restarted
        .keep_split_proposal(item, correction.clone())
        .expect("save correction");
    let mut corrected: Team =
        serde_json::from_value(serde_json::to_value(&restarted).expect("write")).expect("restart after correction");
    assert_eq!(corrected.split_proposal(item), Some(&correction));
    assert!(matches!(
        corrected.split_proposal(item).expect("saved").stage(),
        ProposalStage::Correction { .. }
    ));
    assert_eq!(corrected.items(), original_items);
    assert_eq!(corrected.rounds(), original_rounds);
    corrected.record_split(plan()).expect("whole approval");
    assert_eq!(corrected.split_proposal(item), None);
    assert_eq!(corrected.splits(), &[plan()]);
}

#[test]
fn proposal_membership_and_already_approved_items_are_checked_atomically_on_write_and_read() {
    let mut team = team(1);
    let input = ProposalKeeping::new("input".into()).expect("bounded");
    let unknown = "9.9".parse().expect("item");
    let snapshot = team.clone();
    assert_eq!(
        team.keep_split_proposal(unknown, input.clone()),
        Err(RecordSplitError::UnknownItem { item: unknown })
    );
    assert_eq!(team, snapshot);
    let mut value = serde_json::to_value(&team).expect("write");
    value["split_proposals"] = serde_json::json!({"9.9":input});
    assert_eq!(
        serde_json::from_value::<Team>(value)
            .expect_err("unknown parent")
            .classify(),
        serde_json::error::Category::Data
    );
    team.record_split(plan()).expect("plan");
    let item = "1.2".parse().expect("item");
    let snapshot = team.clone();
    assert_eq!(
        team.keep_split_proposal(item, input.clone()),
        Err(RecordSplitError::AlreadySplit { item })
    );
    assert_eq!(team, snapshot);
    let mut value = serde_json::to_value(&team).expect("write");
    value["split_proposals"] = serde_json::json!({"1.2":input});
    assert_eq!(
        serde_json::from_value::<Team>(value)
            .expect_err("approved parent")
            .classify(),
        serde_json::error::Category::Data
    );
}

#[test]
fn stored_plan_before_proposals_loads_with_no_estimate_and_normal_tier() {
    let text = include_str!("../../../../../tests/fixtures/state-v0-36-before-proposals.json");
    let restored = crate::state::State::from_json(text).expect("old state");
    let saved: serde_json::Value = serde_json::from_str(&restored.to_json().expect("save")).expect("json");
    let original: serde_json::Value = serde_json::from_str(text).expect("fixture");
    assert_eq!(saved, original);
    let old_plan: SplitPlan = serde_json::from_value(saved["teams"][0]["splits"][0].clone()).expect("legacy split");
    for (id, _) in old_plan.entries() {
        assert_eq!(
            old_plan.requirements(id.key()),
            Some(crate::state::split::proposal::Requirements::default())
        );
    }
}

#[test]
fn activating_a_whole_plan_clears_the_saved_proposal_atomically() {
    use crate::state::fixtures::{at, drive, round_id};
    let mut found = team(1);
    let item = "1.2".parse().expect("item");
    let input = ProposalKeeping::new("proposal".into()).expect("bounded");
    found.keep_split_proposal(item, input.clone()).expect("save");
    assert_eq!(found.split_proposal(item), Some(&input));
    let snapshot = found.clone();
    assert_eq!(
        found.activate_split(plan()),
        Err(crate::state::ActivateSplitError::NotWorking { item })
    );
    assert_eq!(found, snapshot);
    found.open_round(round_id(1), Some(item), at(1)).expect("round");
    drive(&mut found, "1.2", crate::state::PhaseKind::InProgress);
    found.activate_split(plan()).expect("activate");
    assert_eq!(found.split_proposal(item), None);
    assert_eq!(
        serde_json::from_value::<Team>(serde_json::to_value(&found).expect("save")).expect("restart"),
        found
    );
}
