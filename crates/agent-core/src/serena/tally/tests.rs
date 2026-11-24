//! The table of the reminder: when the one reminder of a session is due, and when none is.

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

/// The steps of one session: `searches` searches of code, then the steps of `extra`.
fn tally_of(searches: u64, extra: &[ActivityEvent], after: RemindAfter) -> SessionTally {
    let mut tally = SessionTally::new();
    for _ in 0..searches {
        assert_eq!(tally.note(&command("cat src/main.rs"), after), None);
    }
    for event in extra {
        tally.note(event, after);
    }
    tally
}

#[test]
fn a_session_below_the_bound_is_not_reminded_and_one_at_it_is() {
    let after = RemindAfter::new(40).unwrap();

    let below = tally_of(39, &[], after);

    assert_eq!(below.searches(), 39);
    let mut at = tally_of(39, &[], after);
    assert_eq!(
        at.note(&command("grep -n foo src/lib.rs"), after),
        Some(Reminder { searches: 40 })
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
    assert_eq!(tally.note(&command("mcp__serena__find_symbol"), after), None);

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
    first.note(&command("cat src/main.rs"), after);
    first.note(&command("cat src/main.rs"), after);

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
    let mut resumed = tally_of(2, &[], after);

    assert_eq!(resumed.searches(), 2);
    assert_eq!(resumed.note(&command("cat src/main.rs"), after), None);
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
