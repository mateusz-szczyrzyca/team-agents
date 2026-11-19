//! Rotating review context survives state serialization and leaves old PRs alone.
use agent_core::{config::ReviewCount, state::Findings};

#[test]
fn rotating_verification_context_round_trips_and_legacy_context_stays_absent() {
    let mut ledger = Findings::verifying(ReviewCount::TWO);
    assert_ne!(ledger.verification(), None, "new PR uses rotating verification");
    let context = ledger.verification_mut().expect("new PR uses rotating verification");
    context.reviewed_head = Some("a".repeat(40).parse().unwrap());
    context.lead_message = Some("Fixed the café boundary.\nSTATUS: DONE".into());
    context.extra_fix_passes = 5;
    let json = serde_json::to_string(&ledger).unwrap();
    assert_eq!(serde_json::from_str::<Findings>(&json).unwrap(), ledger);
    assert_eq!(Findings::independent(ReviewCount::TWO).verification(), None);
    assert_eq!(Findings::default().verification(), None);
}

#[test]
fn schema_four_active_pr_keeps_its_reviewers_and_previous_protocol() {
    use agent_core::state::{SCHEMA_VERSION, State};
    let old = include_str!("fixtures/state-before-rotating-verification.json");
    let state = State::from_json(old).unwrap();
    assert_eq!(SCHEMA_VERSION, 7);
    let item = state.teams()[0].item("1.2".parse().unwrap()).unwrap();
    assert_eq!(item.findings().unwrap().review_count(), Some(ReviewCount::TWO));
    assert_eq!(item.findings().unwrap().verification(), None);
    assert_eq!(
        state.to_json().unwrap(),
        old.replacen("\"schema_version\": 4", "\"schema_version\": 7", 1)
    );
    assert_eq!(State::from_json(&state.to_json().unwrap()).unwrap(), state);
}
