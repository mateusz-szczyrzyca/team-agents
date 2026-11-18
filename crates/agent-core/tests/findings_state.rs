//! Schema migration keeps old rounds on their original protocol; new ledgers survive a restart.
use agent_core::review_findings::{Finding, Severity};
use agent_core::state::{FindingStatus, Findings, RecordedFinding, StateError};
use agent_core::state::{SCHEMA_VERSION, State};
use std::num::NonZeroU32;

/// A state written by the schema immediately before findings were introduced.
const OLD: &str = include_str!("fixtures/state-before-findings.json");
#[test]
fn reads_the_previous_schema_without_enabling_new_rules_for_open_rounds() {
    let state = State::from_json(OLD).unwrap();
    assert_eq!(SCHEMA_VERSION, 5);
    assert!(
        state
            .teams()
            .iter()
            .flat_map(agent_core::state::Team::items)
            .all(|item| item.findings().is_none())
    );
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&state.to_json().unwrap()).unwrap()["schema_version"],
        4
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

/// A ledger of `count` findings numbered 1 to `count`, as the harness builds one.
fn ledger_of(count: u32) -> Result<Findings, Box<dyn std::error::Error>> {
    let mut entries = Vec::new();
    for number in 1..=count {
        entries.push(RecordedFinding {
            number: NonZeroU32::new(number).ok_or("a finding number is never 0")?,
            finding: Finding {
                severity: Severity::Minor,
                rule: "API-1".into(),
                place: "src/lib.rs:1".into(),
                text: "takes &String → &str".into(),
                check: "grep -n '&String' src/lib.rs".into(),
            },
            reporter: "inv-codex".parse()?,
            pass: NonZeroU32::MIN,
            status: FindingStatus::Open,
        });
    }
    Ok(Findings::new(entries)?)
}

/// The JSON of [`ledger_of`] with the permanent numbers replaced by `numbers`, as a damaged file would carry them.
fn ledger_numbered(numbers: &[u32]) -> Result<serde_json::Value, Box<dyn std::error::Error>> {
    let mut value = serde_json::to_value(ledger_of(u32::try_from(numbers.len())?)?)?;
    let entries = value
        .get_mut("entries")
        .and_then(serde_json::Value::as_array_mut)
        .ok_or("entries is a list")?;
    for (entry, number) in entries.iter_mut().zip(numbers) {
        entry
            .as_object_mut()
            .ok_or("an entry is an object")?
            .insert("number".into(), (*number).into());
    }
    Ok(value)
}

#[test]
fn a_ledger_with_duplicate_gapped_or_unordered_numbers_is_refused_when_read() {
    for numbers in [&[1, 1][..], &[2, 1], &[1, 3], &[2]] {
        let value = ledger_numbered(numbers).unwrap();
        assert!(
            serde_json::from_value::<Findings>(value).is_err(),
            "numbers {numbers:?} must not be read"
        );
    }
}

#[test]
fn a_ledger_numbered_one_to_n_in_order_is_read_back() {
    for count in [0, 1, 3] {
        let ledger = ledger_of(count).unwrap();
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
    *item.findings_mut().unwrap() = ledger_of(2).unwrap();
    let mut value: serde_json::Value = serde_json::from_str(&state.to_json().unwrap()).unwrap();
    *value.pointer_mut("/teams/0/items/0/findings/entries/1/number").unwrap() = 1.into();

    let err = State::from_json(&value.to_string()).unwrap_err();

    assert!(matches!(err, StateError::Malformed { .. }), "{err:?}");
}
