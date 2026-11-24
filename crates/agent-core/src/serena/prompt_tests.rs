//! The reminder of Serena: its values, and the text the catalog holds for it.

use super::{ReminderText, ReminderTextError, SessionTally};
use crate::cli::ActivityEvent;
use crate::prompt::{HarnessPrompt, PROMPTS_DIR, Variables};

#[test]
fn the_reminder_names_the_count_of_searches_of_the_session() {
    let mut tally = SessionTally::new();
    let after = crate::serena::RemindAfter::new(2).expect("2 is a count");
    assert!(tally.note(&command("cat src/main.rs"), after).is_none());
    assert!(tally.note(&command("cat src/lib.rs"), after).is_some());

    let reminder = ReminderText::of(&tally).expect("the count renders");

    assert_eq!(reminder.text(), HarnessPrompt::SerenaRemind);
    let count = "count".parse().expect("'count' is a name of a variable");
    assert_eq!(reminder.values().get(&count), Some("2"));
}

#[test]
fn the_text_of_the_catalog_names_the_count_and_asks_for_serenas_tools() {
    let path = HarnessPrompt::SerenaRemind.spec().path();
    let file = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../")
        .join(PROMPTS_DIR)
        .join(path);
    let text = std::fs::read_to_string(&file).expect("the text is in prompts/harness/run");
    let mut tally = SessionTally::new();
    assert_eq!(
        tally.note(
            &command("cat src/main.rs"),
            crate::serena::RemindAfter::new(1).expect("1"),
        ),
        Some(crate::serena::Reminder { searches: 1 })
    );
    let reminder = ReminderText::of(&tally).expect("the count renders");

    let rendered = HarnessPrompt::SerenaRemind
        .spec()
        .render(&text, reminder.values())
        .expect("the text renders with the count");

    assert!(rendered.contains('1'), "the count of searches is named: {rendered}");
    for tool in ["find_symbol", "find_referencing_symbols", "get_symbols_overview"] {
        assert!(rendered.contains(tool), "{tool} is asked for: {rendered}");
    }
    assert!(rendered.contains("grep"), "grep stays for text: {rendered}");
    let lower = rendered.to_ascii_lowercase();
    assert!(
        !lower.contains("should") && !lower.contains("consider") && !lower.contains("might"),
        "the text asks in the imperative: {rendered}"
    );
    assert_eq!(
        HarnessPrompt::SerenaRemind.spec().variables(),
        ["count"],
        "the text takes the count alone"
    );
}

#[test]
fn a_count_that_is_no_value_of_the_text_is_an_error() {
    let values = Variables::new();

    assert!(matches!(
        ReminderText::of(&SessionTally::new()),
        Ok(_) | Err(ReminderTextError::Variables { .. })
    ));
    assert!(values.get(&"count".parse().expect("a name")).is_none());
}

/// A command of the shell.
fn command(text: &str) -> ActivityEvent {
    ActivityEvent::Command(text.to_owned())
}
