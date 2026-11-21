//! `agent run --short-desc`: the header line it gives to a task given as text.
use agent_core::task_list::{TaskList, TaskListError};

use super::*;

/// `items` as the words of the command line.
fn words(items: &[&str]) -> Vec<String> {
    items.iter().map(|word| (*word).to_owned()).collect()
}

#[test]
fn the_line_goes_before_the_brief_of_the_last_word() {
    let result = words_with_short_desc(words(&["claude-1", "Fix the typo."]), None, Some("Fix a typo")).unwrap();

    assert_eq!(result, words(&["claude-1", "Short desc: Fix a typo\nFix the typo."]));
}

#[test]
fn only_the_last_word_changes_in_a_team() {
    let result = words_with_short_desc(words(&["claude-1", "codex-1", "Fix it."]), None, Some("Fix")).unwrap();

    assert_eq!(result, words(&["claude-1", "codex-1", "Short desc: Fix\nFix it."]));
}

#[test]
fn the_line_goes_after_a_coordinator_line_which_stays_first() {
    let result = words_with_short_desc(
        words(&["claude-1", "codex-1", "Coordinator: claude-1\nFix it."]),
        None,
        Some("Fix"),
    )
    .unwrap();
    let list = TaskList::parse(result.last().unwrap()).unwrap();

    assert_eq!(
        list.coordinator().map(agent_core::ids::AgentName::as_str),
        Some("claude-1")
    );
    assert_eq!(list.short_desc(), Some("Fix"));
    assert_eq!(list.brief(), "Fix it.");
}

#[test]
fn without_the_flag_the_words_are_as_they_were() {
    let given = words(&["claude-1", "Fix the typo."]);

    assert_eq!(words_with_short_desc(given.clone(), None, None).unwrap(), given);
}

#[test]
fn a_command_line_with_no_task_is_left_for_the_words_check_to_refuse() {
    let given = words(&["claude-1"]);

    assert_eq!(words_with_short_desc(given.clone(), None, Some("Fix")).unwrap(), given);
}

#[test]
fn the_flag_with_a_file_is_refused_and_names_the_file() {
    let file = PathBuf::from("tasks.md");

    let error = words_with_short_desc(words(&["claude-1"]), Some(&file), Some("Fix")).unwrap_err();

    assert!(
        matches!(&error, RunError::ShortDescWithFile { path } if *path == file),
        "{error:?}"
    );
}

#[test]
fn the_text_a_wrong_flag_makes_is_refused_by_the_header_rules() {
    let long = "x".repeat(71);
    let result = words_with_short_desc(words(&["claude-1", "Fix it."]), None, Some(&long)).unwrap();

    let error = TaskList::parse(result.last().unwrap()).unwrap_err();

    assert!(matches!(error, TaskListError::ShortDescTooLong { .. }), "{error:?}");
}

#[test]
fn a_task_with_its_own_line_and_the_flag_is_refused_as_two_lines() {
    let result = words_with_short_desc(words(&["claude-1", "Short desc: Mine\nFix it."]), None, Some("Other")).unwrap();

    let error = TaskList::parse(result.last().unwrap()).unwrap_err();

    assert!(matches!(error, TaskListError::ShortDescTwice { .. }), "{error:?}");
}
