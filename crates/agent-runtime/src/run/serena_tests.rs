//! The count of a session's searches of code without Serena: when the reminder of the session is due.

use agent_core::cli::ActivityEvent;
use agent_core::serena::RemindAfter;

use super::{SessionWatch, Watched};

/// A command of the shell, as a CLI's stream names it.
fn command(text: &str) -> ActivityEvent {
    ActivityEvent::Command(text.to_owned())
}

/// The bound of the tests: two searches of code.
fn two() -> RemindAfter {
    RemindAfter::new(2).expect("2 is a bound")
}

#[test]
fn a_run_without_serena_counts_no_step() {
    let watch = SessionWatch::new(None);

    assert!(watch.is_none(), "a run without Serena has no watch");
}

#[test]
fn the_watch_of_a_session_that_goes_on_is_not_reminded_again() {
    let mut watch = SessionWatch::after_reminder(2);

    let told: Vec<Watched> = (0..4).map(|_| watch.note(&command("cat src/main.rs"), two())).collect();

    assert_eq!(told, vec![Watched::Quiet; 4]);
}

#[test]
fn the_reminder_is_due_at_the_bound_once() {
    let mut watch = SessionWatch::new(Some(two())).expect("the run has Serena");

    let told: Vec<Watched> = (0..4).map(|_| watch.note(&command("cat src/main.rs"), two())).collect();

    assert_eq!(
        told,
        [
            Watched::Quiet,
            Watched::Due { searches: 2 },
            Watched::Quiet,
            Watched::Quiet
        ]
    );
}

#[test]
fn a_step_of_text_is_no_search_of_code() {
    let mut watch = SessionWatch::new(Some(two())).expect("the run has Serena");

    for _ in 0..5 {
        assert_eq!(watch.note(&command("cat README.md"), two()), Watched::Quiet);
        assert_eq!(
            watch.note(
                &ActivityEvent::ToolCall {
                    name: "Read".to_owned(),
                    summary: "docs/configuration.md".to_owned(),
                },
                two()
            ),
            Watched::Quiet
        );
    }
}

#[test]
fn the_bound_of_zero_reminds_at_the_first_step_that_counts() {
    let after = RemindAfter::new(1).expect("1 is a bound");
    let mut watch = SessionWatch::new(Some(after)).expect("the run has Serena");

    assert_eq!(
        watch.note(&command("grep -n foo src/main.rs"), after),
        Watched::Due { searches: 1 }
    );
    assert_eq!(watch.note(&command("grep -n foo src/main.rs"), after), Watched::Quiet);
}
