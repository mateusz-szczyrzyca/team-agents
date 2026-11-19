//! The state of the evidence — two items of one pull request, a pending explanation in one of them only — is repaired
//! once, and a state whose items agree is left as it is.
use super::*;
use agent_core::state::{ExplanationStatus, FindingStatus};
use std::num::NonZeroU32;

/// The state of the evidence: items 1.1 and 2.1 on one pull request, finding 28 explained in 1.1 only.
fn evidence() -> State {
    State::from_json(include_str!(
        "../../../../../agent-core/tests/fixtures/state-split-explanations.json"
    ))
    .unwrap()
}

/// The explanation of finding `number` in item `item` of the evidence's team.
fn explanation_of(state: &State, item: &str, number: u32) -> Option<ExplanationStatus> {
    state.teams()[0]
        .item(item.parse().unwrap())
        .unwrap()
        .findings()
        .unwrap()
        .disputes()
        .unwrap()
        .explanations
        .get(&NonZeroU32::new(number).unwrap())
        .map(|explanation| explanation.status)
}

#[test]
fn the_explanation_of_the_evidence_is_pending_in_both_items_after_the_repair() {
    let mut state = evidence();
    let team = state.teams()[0].id().clone();

    let repaired = reconcile_findings(&mut state);

    assert_eq!(
        repaired,
        vec![RepairedFinding {
            team,
            pr: PrNumber::new(NonZeroU32::new(132).unwrap()),
            items: vec!["1.1".parse().unwrap(), "2.1".parse().unwrap()],
            finding: Reconciled {
                number: NonZeroU32::new(28).unwrap(),
                explanation: Some(ExplanationStatus::Pending),
                status: FindingStatus::Open,
            },
        }]
    );
    assert_eq!(explanation_of(&state, "2.1", 28), Some(ExplanationStatus::Pending));
    assert_eq!(explanation_of(&state, "1.1", 28), Some(ExplanationStatus::Pending));
    assert_eq!(State::from_json(&state.to_json().unwrap()).unwrap(), state);
}

#[test]
fn repairing_twice_changes_nothing_the_second_time() {
    let mut state = evidence();
    assert_eq!(reconcile_findings(&mut state).len(), 1);
    let repaired = state.clone();

    assert_eq!(reconcile_findings(&mut state), vec![]);
    assert_eq!(state, repaired);
}
