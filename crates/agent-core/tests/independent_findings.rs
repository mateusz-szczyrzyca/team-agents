//! Persisted independence survives a restart while legacy ledgers retain their protocol.
use agent_core::config::ReviewCount;
use agent_core::state::Findings;

#[test]
fn independent_ledgers_keep_the_count_and_legacy_ledgers_do_not_opt_in() {
    let count = ReviewCount::try_from(3).unwrap();
    let ledger = Findings::independent(count);
    assert_eq!(ledger.review_count(), Some(count));
    let saved = serde_json::to_string(&ledger).unwrap();
    let loaded: Findings = serde_json::from_str(&saved).unwrap();
    assert_eq!(loaded.review_count(), Some(count));
    let legacy: Findings = serde_json::from_str(r#"{"entries":[],"pending":{},"comment":null}"#).unwrap();
    assert_eq!(legacy.review_count(), None);
}
