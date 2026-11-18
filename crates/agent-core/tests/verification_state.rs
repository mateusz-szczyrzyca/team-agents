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
