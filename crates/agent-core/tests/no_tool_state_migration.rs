//! Schema fourteen keeps every saved value when promoted to schema fifteen.

use agent_core::state::State;

#[test]
fn schema_fourteen_preserves_the_complete_document_when_promoted_to_fifteen() {
    let fixture = include_str!("fixtures/state-before-no-tool-call.json");
    let mut expected: serde_json::Value = serde_json::from_str(fixture).unwrap();
    assert_eq!(expected["schema_version"], 14);
    expected["schema_version"] = 15.into();
    // The serializer already omits an empty activity list; its absence carries the same empty history.
    assert_eq!(expected["teams"][0]["activities"], serde_json::json!([]));
    expected["teams"][0].as_object_mut().unwrap().remove("activities");

    let migrated = State::from_json(fixture).unwrap();
    let saved = migrated.to_json().unwrap();

    assert_eq!(serde_json::from_str::<serde_json::Value>(&saved).unwrap(), expected);
    assert_eq!(State::from_json(&saved).unwrap(), migrated);
    assert!(
        migrated
            .teams()
            .iter()
            .flat_map(|team| team.crew().roster())
            .all(|(_, member)| member.runs().iter().all(|run| !run.used_tool()))
    );
}
