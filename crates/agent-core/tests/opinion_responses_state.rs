//! Compatibility with the daemon before durable opinion responses.
use agent_core::state::{SCHEMA_VERSION, State};
use std::num::NonZeroU32;
#[test]
fn schema_seven_active_fix_pass_migrates_without_inventing_answers() {
    let old = include_str!("fixtures/state-before-opinion-responses.json");
    let state = State::from_json(old).unwrap();
    let ledger = state.teams()[0].items()[1].findings().unwrap();
    assert_eq!(
        ledger.fixed_answers(NonZeroU32::MIN),
        std::collections::BTreeSet::default()
    );
    assert_eq!(ledger.opinions().len(), 0);
    assert_eq!(SCHEMA_VERSION, 9);
    assert_eq!(
        state.to_json().unwrap(),
        old.replacen("\"schema_version\": 7", "\"schema_version\": 9", 1)
    );
    assert_eq!(State::from_json(&state.to_json().unwrap()).unwrap(), state);
}

#[test]
fn schema_seven_unanswered_fix_pass_accepts_the_new_response_protocol() {
    use agent_core::review_response::opinion::Permission;
    use agent_core::state::{ItemPhase, OpinionStage};
    let old = include_str!("fixtures/state-seven-unanswered-fix.json");
    assert_eq!(old.matches("\"phase\": \"fixing\"").count(), 1);
    for (message, fixed) in [
        ("FINDING 1: FIXED", true),
        ("FINDING 1: SECOND OPINION — café path is unreachable", false),
    ] {
        let mut state = State::from_json(old).unwrap();
        let team_id = state.teams()[0].id().clone();
        let mut team = state.team_mut(&team_id).unwrap();
        let mut item = team.item_mut("1.2".parse().unwrap()).unwrap();
        assert!(matches!(item.phase(), ItemPhase::Fixing { .. }));
        let ledger = item.findings_mut().unwrap();
        assert_eq!(ledger.disputes().unwrap().answered_pass, 0);
        assert_eq!(ledger.opinions().len(), 0);
        assert_eq!(
            ledger.fixed_answers(NonZeroU32::MIN),
            std::collections::BTreeSet::default()
        );
        let head = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".parse().unwrap();
        ledger
            .answer_opinions(message, NonZeroU32::MIN, &head, Permission::Enabled)
            .unwrap();
        assert_eq!(ledger.disputes().unwrap().answered_pass, 1);
        if fixed {
            assert_eq!(ledger.fixed_answers(NonZeroU32::MIN), [NonZeroU32::MIN].into());
        } else {
            assert_eq!(ledger.opinions()[&NonZeroU32::MIN].stage, OpinionStage::Waiting);
        }
        let saved = state.to_json().unwrap();
        assert_eq!(State::from_json(&saved).unwrap(), state);
    }
}
