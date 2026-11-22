//! Pre-grouping publication question identities remain readable without invented task or question identities.

use agent_core::state::State;

#[test]
fn flat_publication_question_fixture_preserves_complete_identity_and_discards_incomplete_identity() {
    let text = include_str!("fixtures/state-before-grouped-publish-question.json");
    let state = State::from_json(text).expect("flat publication state loads");
    let items = state.teams().first().unwrap().items();
    let complete = items.first().unwrap();
    assert_eq!(complete.publish_tries(), 4);
    assert_eq!(
        complete.publish_question(),
        Some("2026-11-21T12:00:00Z".parse().unwrap())
    );
    assert_eq!(
        complete.publish_question_task(),
        Some(&"task-20261111-193100-a1b2".parse().unwrap())
    );
    assert_eq!(complete.publish_question_number(), std::num::NonZeroU32::new(1));
    assert_eq!(complete.publish_question_branch(), None);
    let incomplete = items.get(1).unwrap();
    assert_eq!(incomplete.publish_tries(), 3);
    assert_eq!(incomplete.publish_question(), None);
    assert_eq!(incomplete.publish_question_task(), None);
    assert_eq!(incomplete.publish_question_number(), None);
    assert_eq!(incomplete.publish_question_branch(), None);
    let restored = State::from_json(&serde_json::to_string(&state).unwrap()).unwrap();
    assert_eq!(restored.teams().first().unwrap().items(), items);
}
