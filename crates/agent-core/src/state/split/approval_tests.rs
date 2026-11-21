//! Approval metadata remains optional and survives stored-plan serialization.

use super::*;
use crate::kinds::Tier;
use serde_json::{Value, json};

/// A split extracted without alteration from a stored pre-provenance team.
const LEGACY: &str = include_str!("../../../tests/fixtures/split-plan-before-approval.json");

#[test]
fn stored_plan_without_provenance_loads_and_retains_its_representation() {
    let plan: SplitPlan = serde_json::from_str(LEGACY).expect("legacy plan");
    assert_eq!(plan.parent(), "1.2".parse().expect("id"));
    assert_eq!(plan.approved_by(), None);
    assert_eq!(plan.entries().count(), 2);
    for (id, _) in plan.entries() {
        assert_eq!(plan.requirements(id.key()).expect("requirements").tier, Tier::Normal);
    }
    assert_eq!(
        serde_json::to_value(plan).expect("serialize"),
        serde_json::from_str::<Value>(LEGACY).expect("json")
    );
}

#[test]
fn approval_authority_round_trips_without_changing_parts_or_requirements() {
    for (approval, keyword) in [(SplitApproval::Harness, "harness"), (SplitApproval::Human, "human")] {
        let mut value: Value = serde_json::from_str(LEGACY).expect("fixture");
        value["parts"][0]["tier"] = json!("investigator");
        value["parts"][0]["estimate"] = json!({"amount":12.5,"unit":"pages"});
        let plan: SplitPlan = serde_json::from_value(value.clone()).expect("plan");
        let approved = plan.with_approval(approval);
        assert_eq!(approved.approved_by(), Some(approval));
        value["approved_by"] = json!(keyword);
        assert_eq!(serde_json::to_value(&approved).expect("serialize"), value);
        assert_eq!(serde_json::from_value::<SplitPlan>(value).expect("reload"), approved);
    }
}

#[test]
fn unknown_approval_authority_is_refused() {
    let mut value: Value = serde_json::from_str(LEGACY).expect("fixture");
    value["approved_by"] = json!("automatic-ish");
    let error = serde_json::from_value::<SplitPlan>(value).expect_err("unknown authority");
    assert_eq!(error.classify(), serde_json::error::Category::Data);
}
