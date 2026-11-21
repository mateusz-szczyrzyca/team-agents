//! A task list stored without the marker of the `Short desc:` header is read as every version before the line read it:
//! the line is brief text, whatever it says, and the permissions around it stay what they were.
use serde_json::json;

use super::TaskListText;
use crate::state::State;

/// The text read from a stored plain string.
fn stored(text: &str) -> TaskListText {
    serde_json::from_value(json!(text)).unwrap()
}

#[test]
fn a_stored_text_keeps_its_brief_and_its_permissions_as_the_version_before_read_them() {
    let cases = [
        ("Short desc: Original brief", None, "Short desc: Original brief", 0),
        (
            "Short desc: One\nShort desc: Two\nBrief.",
            None,
            "Short desc: One\nShort desc: Two\nBrief.",
            0,
        ),
        ("Short desc:\nOriginal brief.", None, "Short desc:\nOriginal brief.", 0),
        (
            "Allow: Cargo.toml\nShort desc:\nOriginal brief.",
            None,
            "Short desc:\nOriginal brief.",
            1,
        ),
        (
            "Allow: Cargo.toml\nShort desc: Fine\nAllow: docs/*.md\nBrief.",
            None,
            "Short desc: Fine\nAllow: docs/*.md\nBrief.",
            1,
        ),
        (
            "Coordinator: inv-opus\nAllow: Cargo.toml\nShort desc: Fine\n\nBrief.",
            None,
            "Short desc: Fine\n\nBrief.",
            1,
        ),
    ];

    for (text, short_desc, brief, allowances) in cases {
        let list = stored(text);

        assert_eq!(list.list().short_desc(), short_desc, "{text:?}");
        assert_eq!(list.list().brief(), brief, "{text:?}");
        assert_eq!(list.list().allowances().len(), allowances, "{text:?}");
        assert_eq!(list.text(), text);
    }
}

#[test]
fn a_stored_text_is_written_back_as_it_was_read() {
    for text in [
        "Short desc: Fine\nBrief.",
        "Allow: Cargo.toml\nShort desc:\nBrief.",
        "Brief only.\n",
    ] {
        let list = stored(text);

        assert_eq!(serde_json::to_value(&list).unwrap(), json!(text), "{text:?}");
    }
}

#[test]
fn a_new_text_with_a_header_description_is_written_marked_and_read_back_as_a_description() {
    let list = TaskListText::new("Allow: Cargo.toml\nShort desc: Fix it\nBrief.\n".to_owned()).unwrap();

    let written = serde_json::to_value(&list).unwrap();
    let read: TaskListText = serde_json::from_value(written.clone()).unwrap();

    assert_eq!(
        written,
        json!({
            "text": "Allow: Cargo.toml\nShort desc: Fix it\nBrief.\n",
            "allow_lines": "permissions",
            "short_desc_header": true
        })
    );
    assert_eq!(read, list);
    assert_eq!(read.list().short_desc(), Some("Fix it"));
    assert_eq!(read.list().allowances().len(), 1);
}

#[test]
fn a_new_text_without_a_header_description_is_written_as_a_plain_string() {
    let list = TaskListText::new("Fix it.\n\nShort desc: later in the brief\n".to_owned()).unwrap();

    assert_eq!(
        serde_json::to_value(&list).unwrap(),
        json!("Fix it.\n\nShort desc: later in the brief\n")
    );
}

#[test]
fn a_marked_text_that_the_header_rules_refuse_is_a_fault_of_the_state() {
    let long = "x".repeat(71);

    let result: Result<TaskListText, _> = serde_json::from_value(json!({
        "text": format!("Short desc: {long}\nBrief."),
        "allow_lines": "permissions",
        "short_desc_header": true
    }));

    assert!(matches!(result, Err(ref error) if error.is_data()), "{result:?}");
}

#[test]
fn a_state_written_before_the_line_loads_with_its_briefs_and_permissions_unchanged() {
    let state = State::from_json(include_str!("../../tests/fixtures/state-before-short-desc.json")).unwrap();
    let lists: Vec<_> = state.teams().iter().map(super::team::Team::task_list).collect();

    assert_eq!(lists.len(), 3);
    assert_eq!(lists[0].list().short_desc(), None);
    assert!(
        lists[0]
            .list()
            .brief()
            .starts_with("Short desc: Original brief\n\nBuild the thing.")
    );
    assert_eq!(
        lists[0]
            .list()
            .allowances()
            .iter()
            .map(|allowance| allowance.as_str())
            .collect::<Vec<_>>(),
        ["Cargo.toml"]
    );
    assert!(
        lists[1]
            .list()
            .brief()
            .starts_with("Short desc:\nFix the typo in the README.")
    );
    assert!(lists[2].list().brief().starts_with("Short desc: xxxxxxxx"));
    for list in &lists {
        assert_eq!(list.list().short_desc(), None);
    }
}

#[test]
fn such_a_state_has_a_derived_description_for_every_team_and_is_saved_as_it_was() {
    let json = include_str!("../../tests/fixtures/state-before-short-desc.json");
    let state = State::from_json(json).unwrap();

    let described: Vec<String> = state
        .teams()
        .iter()
        .map(|team| team.task_list().list().display_short_desc())
        .collect();
    let saved = state.to_json().unwrap();

    assert_eq!(described[0], "Short desc: Original brief");
    assert_eq!(described[1], "Short desc: Fix the typo in the README.");
    assert!(
        described[2].starts_with("Short desc: xxxx") && described[2].ends_with('…'),
        "{described:?}"
    );
    assert_eq!(State::from_json(&saved).unwrap(), state);
    assert!(!saved.contains("short_desc_header"));
}
