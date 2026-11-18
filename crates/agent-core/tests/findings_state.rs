//! Schema migration keeps old rounds on their original protocol; new ledgers survive a restart.
use agent_core::review_findings::{Finding, Severity};
use agent_core::state::{FindingStatus, Findings, RecordedFinding, SCHEMA_VERSION, State, StateError};
use std::num::NonZeroU32;

/// A state written by the schema immediately before findings were introduced.
const OLD: &str = include_str!("fixtures/state-before-findings.json");
#[test]
fn reads_the_previous_schema_without_enabling_new_rules_for_open_rounds() {
    let state = State::from_json(OLD).unwrap();
    assert_eq!(SCHEMA_VERSION, 3);
    assert!(
        state
            .teams()
            .iter()
            .flat_map(agent_core::state::Team::items)
            .all(|item| item.findings().is_none())
    );
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&state.to_json().unwrap()).unwrap()["schema_version"],
        3
    );
}
#[test]
fn a_new_findings_ledger_roundtrips_with_its_item() {
    let mut state = State::from_json(OLD).unwrap();
    let team = state.teams()[0].id().clone();
    let id = state.teams()[0].items()[0].id();
    state.team_mut(&team).unwrap().item_mut(id).unwrap().start_findings();
    assert!(state.team(&team).unwrap().item(id).unwrap().findings().is_some());
    assert_eq!(State::from_json(&state.to_json().unwrap()).unwrap(), state);
}

/// A ledger whose entries carry the given permanent numbers, in the order given.
fn ledger_numbered(numbers: &[u32]) -> Findings {
    let mut ledger = Findings::default();
    for number in numbers {
        ledger.entries.push(RecordedFinding {
            number: NonZeroU32::new(*number).unwrap(),
            finding: Finding {
                severity: Severity::Minor,
                rule: "API-1".into(),
                place: "src/lib.rs:1".into(),
                text: "takes &String → &str".into(),
                check: "grep -n '&String' src/lib.rs".into(),
            },
            reporter: "inv-codex".parse().unwrap(),
            pass: NonZeroU32::MIN,
            status: FindingStatus::Open,
        });
    }
    ledger
}

#[test]
fn a_ledger_with_duplicate_gapped_or_unordered_numbers_is_refused_when_read() {
    for numbers in [&[1, 1][..], &[2, 1], &[1, 3], &[2]] {
        let text = serde_json::to_string(&ledger_numbered(numbers)).unwrap();
        assert!(
            serde_json::from_str::<Findings>(&text).is_err(),
            "numbers {numbers:?} must not be read"
        );
    }
}

#[test]
fn a_ledger_numbered_one_to_n_in_order_is_read_back() {
    for numbers in [&[][..], &[1], &[1, 2, 3]] {
        let ledger = ledger_numbered(numbers);
        let text = serde_json::to_string(&ledger).unwrap();
        assert_eq!(serde_json::from_str::<Findings>(&text).unwrap(), ledger);
    }
}

#[test]
fn a_state_with_two_open_findings_numbered_one_is_malformed() {
    let mut state = State::from_json(OLD).unwrap();
    let team = state.teams()[0].id().clone();
    let id = state.teams()[0].items()[0].id();
    let mut found = state.team_mut(&team).unwrap();
    let mut item = found.item_mut(id).unwrap();
    item.start_findings();
    *item.findings_mut().unwrap() = ledger_numbered(&[1, 1]);

    let err = State::from_json(&state.to_json().unwrap()).unwrap_err();

    assert!(matches!(err, StateError::Malformed { .. }), "{err:?}");
}
