//! State written before conflict accounting loads cleanly and preserves spent passes and conflict state.

use agent_core::state::{ItemPhase, MergeConflict, State};

#[test]
fn state_before_conflict_accounting_loads_with_exact_passes_and_conflict() {
    let text = include_str!("fixtures/state-before-conflict-accounting.json");
    let state = State::from_json(text).expect("state before conflict accounting should load cleanly");
    let item = state.teams()[0].item("1.1".parse().unwrap()).unwrap();

    assert!(matches!(item.phase(), ItemPhase::Approved { .. }));
    assert_eq!(item.merge_passes(), 2);
    assert!(matches!(item.merge_conflict(), Some(MergeConflict::MergingBase { .. })));
}
