//! A task list in a state written before the `Short desc:` line existed is read as it was, whatever its first lines say.
use serde_json::json;

use super::TaskListText;
use crate::task_list::TaskListError;

/// The text of a task list whose first line is a `Short desc:` line that this version would refuse.
fn too_long_first_line() -> String {
    format!("Short desc: {}\nFix the typo.\n", "x".repeat(71))
}

#[test]
fn a_stored_text_with_a_valid_line_gives_its_description() {
    let list: TaskListText = serde_json::from_value(json!("Short desc: Fix it\nFix the typo.\n")).unwrap();

    assert_eq!(list.list().short_desc(), Some("Fix it"));
    assert_eq!(list.list().brief(), "Fix the typo.");
}

#[test]
fn a_stored_text_with_a_refused_line_is_read_as_the_brief_it_was() {
    for text in [
        too_long_first_line(),
        "Short desc:\nFix the typo.\n".to_owned(),
        "Short desc: One\nShort desc: Two\nFix the typo.\n".to_owned(),
    ] {
        let list: TaskListText = serde_json::from_value(json!(text)).unwrap();

        assert_eq!(list.list().short_desc(), None, "{text:?}");
        assert!(list.list().brief().starts_with("Short desc:"), "{text:?}");
        assert_eq!(list.text(), text);
    }
}

#[test]
fn a_text_read_that_way_is_written_back_marked_and_read_the_same_again() {
    let text = too_long_first_line();
    let list: TaskListText = serde_json::from_value(json!(text)).unwrap();

    let written = serde_json::to_value(&list).unwrap();
    let again: TaskListText = serde_json::from_value(written.clone()).unwrap();

    assert_eq!(written, json!({ "text": text, "allow_lines": "brief" }));
    assert_eq!(again, list);
}

#[test]
fn a_new_text_with_a_refused_line_is_still_refused() {
    let error = TaskListText::new(too_long_first_line()).unwrap_err();

    assert!(matches!(error, TaskListError::ShortDescTooLong { .. }), "{error:?}");
}

#[test]
fn another_fault_of_a_stored_text_is_still_a_fault() {
    let result: Result<TaskListText, _> =
        serde_json::from_value(json!("Short desc: Fine\n1. Feature\n[ ] 1.x Broken\n"));

    assert!(result.is_err());
}
