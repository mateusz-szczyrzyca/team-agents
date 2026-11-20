//! The preceding daemon's state keeps active rounds on the preceding protocol.
use agent_core::state::{SCHEMA_VERSION, State};

#[test]
fn schema_three_migrates_without_opting_existing_items_into_independent_reviews() {
    let old = include_str!("fixtures/state-before-independent-reviews.json");
    let state = State::from_json(old).unwrap();
    assert_eq!(SCHEMA_VERSION, 13);
    assert!(
        state
            .teams()
            .iter()
            .flat_map(agent_core::state::Team::items)
            .all(|item| item.findings().is_none_or(|ledger| ledger.review_count().is_none()))
    );
    let saved = state.to_json().unwrap();
    assert_eq!(
        saved,
        include_str!("fixtures/state-before-independent-reviews-activities.json")
            .replace("\"schema_version\": 11", "\"schema_version\": 13")
    );
    assert_eq!(State::from_json(&saved).unwrap(), state);
}
