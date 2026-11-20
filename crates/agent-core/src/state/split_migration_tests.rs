//! Compatibility with schema 12, before approved split plans existed.

use super::*;
use crate::state::split::{PartDefinition, SplitPlan};

#[test]
fn schema_twelve_keeps_all_history_and_defaults_to_no_splits_before_writing_thirteen() {
    let old = include_str!("../../tests/fixtures/state-before-splits.json");
    let mut state = State::from_json(old).expect("schema 12 loads");
    assert!(state.teams().iter().all(|team| team.splits().is_empty()));
    let mut written: serde_json::Value = serde_json::from_str(&state.to_json().expect("save")).expect("json");
    assert_eq!(written["schema_version"], 14);
    written["schema_version"] = serde_json::json!(12);
    assert_eq!(
        written,
        serde_json::from_str::<serde_json::Value>(old).expect("old json")
    );
    let team_id = state.teams()[0].id().clone();
    let plan = SplitPlan::new(
        "2.1".parse().expect("item"),
        ["Draft", "Publish"]
            .into_iter()
            .map(|title| PartDefinition {
                title: title.into(),
                delivers: "Document".into(),
                checks: "Review examples".into(),
                depends_on: vec![],
            })
            .collect(),
    )
    .expect("plan");
    state
        .team_mut(&team_id)
        .expect("team")
        .record_split(plan.clone())
        .expect("record");
    let restored = State::from_json(&state.to_json().expect("save split state")).expect("restart");
    assert_eq!(restored, state);
    assert_eq!(restored.team(&team_id).expect("team").splits(), &[plan]);
}
