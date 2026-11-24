//! The table of the reminder of a session: when the one reminder is due, and when none is.

use crate::cli::ActivityEvent;

use super::{DEFAULT_REMIND_AFTER, RemindAfter, RemindAfterError, Reminder, SessionTally};

/// A command of the shell, as a CLI's stream names it.
fn command(text: &str) -> ActivityEvent {
    ActivityEvent::Command(text.to_owned())
}

/// A call of the tool `name` of a CLI.
fn call(name: &str, summary: &str) -> ActivityEvent {
    ActivityEvent::ToolCall {
        name: name.to_owned(),
        summary: summary.to_owned(),
    }
}

/// A session after `searches` searches of code, none of them past `after`, and what the searches told: the count has one
/// per search, told by its own number.
fn one_reminder(searches: u64, after: RemindAfter) -> Option<Reminder> {
    let mut tally = SessionTally::new();
    let told: Vec<Option<Reminder>> = (0..searches)
        .map(|_| tally.note(&command("cat src/main.rs"), after))
        .collect();
    assert_eq!(tally.searches(), searches);
    let reminders = told.iter().flatten().copied().collect::<Vec<_>>();
    assert_eq!(reminders.len(), usize::from(searches >= after.get()));
    reminders.first().copied()
}

#[test]
fn a_session_below_the_bound_is_not_reminded_and_one_at_it_is() {
    let after = RemindAfter::new(40).unwrap();

    assert_eq!(one_reminder(39, after), None, "39 searches are below 40");
    assert_eq!(
        one_reminder(40, after),
        Some(Reminder { searches: 40 }),
        "40 searches are at the bound"
    );
}

#[test]
fn a_reminder_is_told_once_in_a_session() {
    let after = RemindAfter::new(2).unwrap();
    let mut tally = SessionTally::new();
    assert_eq!(tally.note(&command("cat src/main.rs"), after), None);
    assert_eq!(
        tally.note(&command("cat src/main.rs"), after),
        Some(Reminder { searches: 2 })
    );

    assert_eq!(tally.note(&command("cat src/main.rs"), after), None);
    assert_eq!(tally.note(&command("cat src/lib.rs"), after), None);
    assert_eq!(tally.searches(), 4);
}

#[test]
fn a_session_that_called_serena_is_never_reminded() {
    let after = RemindAfter::new(2).unwrap();
    let mut tally = SessionTally::new();
    assert_eq!(tally.note(&command("cat src/main.rs"), after), None);
    assert_eq!(tally.note(&call("mcp__serena__find_symbol", "{}"), after), None);

    assert_eq!(tally.note(&command("cat src/main.rs"), after), None);
    assert_eq!(tally.note(&command("cat src/lib.rs"), after), None);

    let mut called_at_once = SessionTally::new();
    assert_eq!(
        called_at_once.note(&call("serena.get_symbols_overview", "{}"), after),
        None
    );
    assert_eq!(called_at_once.note(&command("cat src/main.rs"), after), None);
    assert_eq!(called_at_once.note(&command("cat src/main.rs"), after), None);
}

#[test]
fn a_session_without_serena_is_never_reminded() {
    let mut tally = SessionTally::new();

    assert_eq!(tally.note(&command("cat src/main.rs"), RemindAfter::default()), None);
}

#[test]
fn a_new_session_counts_its_searches_anew() {
    let after = RemindAfter::new(2).unwrap();
    let mut first = SessionTally::new();
    assert_eq!(first.note(&command("cat src/main.rs"), after), None);
    assert_eq!(
        first.note(&command("cat src/main.rs"), after),
        Some(Reminder { searches: 2 })
    );

    let mut second = SessionTally::new();

    assert_eq!(second.searches(), 0);
    assert_eq!(second.note(&command("cat src/main.rs"), after), None);
    assert_eq!(
        second.note(&command("cat src/main.rs"), after),
        Some(Reminder { searches: 2 })
    );
}

#[test]
fn a_session_that_goes_on_keeps_its_count_and_its_reminder() {
    let after = RemindAfter::new(2).unwrap();
    let mut tally = SessionTally::new();
    assert_eq!(tally.note(&command("cat src/main.rs"), after), None);
    assert_eq!(
        tally.note(&command("cat src/main.rs"), after),
        Some(Reminder { searches: 2 })
    );
    let count_when_reminded = tally.searches();

    assert_eq!(tally.note(&command("cat src/lib.rs"), after), None);
    assert_eq!(tally.searches(), 3);
    assert_eq!(count_when_reminded, 2);
}

#[test]
fn searches_of_text_are_not_counted() {
    let after = RemindAfter::new(2).unwrap();
    let mut tally = SessionTally::new();

    for _ in 0..5 {
        assert_eq!(tally.note(&command("cat README.md"), after), None);
        assert_eq!(tally.note(&call("Read", "docs/configuration.md"), after), None);
    }
    assert_eq!(tally.searches(), 0);
}

#[test]
fn zero_searches_is_no_point_at_which_to_remind() {
    assert_eq!(RemindAfter::new(0), Err(RemindAfterError::Zero));
    assert_eq!(RemindAfter::default().get(), DEFAULT_REMIND_AFTER);
}
