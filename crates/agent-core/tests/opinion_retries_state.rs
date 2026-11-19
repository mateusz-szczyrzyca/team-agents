//! Migration preserves an open fix pass before bounded opinion assignments existed.
use agent_core::state::{SCHEMA_VERSION, State};
use std::num::NonZeroU32;

#[test]
fn schema_eight_active_fix_pass_migrates_without_inventing_opinion_attempts() {
    let old = include_str!("fixtures/state-before-opinion-retries.json");
    let state = State::from_json(old).unwrap();
    assert_eq!(SCHEMA_VERSION, 9);
    let ledger = state.teams()[0].items()[1].findings().unwrap();
    assert_eq!(ledger.opinion_attempts(NonZeroU32::MIN), []);
    assert_eq!(ledger.opinions().len(), 0);
    assert_eq!(ledger.fixed_answers(NonZeroU32::MIN), Default::default());
    assert_eq!(
        state.to_json().unwrap(),
        old.replacen("\"schema_version\": 8", "\"schema_version\": 9", 1)
    );
    assert_eq!(State::from_json(&state.to_json().unwrap()).unwrap(), state);
    let mut state = state;
    let team_id = state.teams()[0].id().clone();
    let mut team = state.team_mut(&team_id).unwrap();
    let mut item = team.item_mut("1.2".parse().unwrap()).unwrap();
    let ledger = item.findings_mut().unwrap();
    ledger
        .answer_opinions(
            "FINDING 1: SECOND OPINION — café boundary",
            NonZeroU32::MIN,
            &"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".parse().unwrap(),
            agent_core::review_response::opinion::Permission::Enabled,
        )
        .unwrap();
    ledger
        .assign_opinion_at(
            NonZeroU32::MIN,
            "inv-independent".parse().unwrap(),
            "task-20261111-193007-k3x9".parse().unwrap(),
            "2026-11-11T19:50:00Z".parse().unwrap(),
        )
        .unwrap();
    assert_eq!(ledger.opinion_attempts(NonZeroU32::MIN).len(), 1);
    assert_eq!(ledger.disputes().unwrap().answered_pass, 1);
    assert_eq!(State::from_json(&state.to_json().unwrap()).unwrap(), state);
}
