//! The line after the end of a run.

use agent_core::ids::RoundId;

use super::{Asked, wait_text};

/// The round of the lines.
fn round() -> RoundId {
    "round-20261121-132909-q1w2".parse().expect("a literal id is valid")
}

#[test]
fn a_round_that_waits_says_for_what() {
    assert_eq!(
        wait_text(
            &round(),
            &Asked::Waits("the queued run of its task to start".to_owned())
        ),
        "round round-20261121-132909-q1w2 waits for the queued run of its task to start"
    );
}

#[test]
fn a_round_that_waits_for_nothing_says_that_it_goes_on() {
    assert_eq!(
        wait_text(&round(), &Asked::Nothing),
        "round round-20261121-132909-q1w2 waits for nothing: a run of it goes on, or it is closed"
    );
}

#[test]
fn a_round_the_daemon_could_not_be_asked_about_says_why() {
    assert_eq!(
        wait_text(&round(), &Asked::Unknown("nothing listens at the socket".to_owned())),
        "round round-20261121-132909-q1w2: what it waits for is not known, the daemon was not asked: nothing listens at the socket"
    );
}

#[test]
fn a_github_retry_wait_is_formatted_in_the_client_zone() {
    use agent_core::local_time::Zone;

    let asked = Asked::Waits(
        "GitHub: refresh the description of the pull requests, tried 4 times, next try 2026-11-21T00:07:00Z".to_owned(),
    );
    let east = Zone::Fixed(time::UtcOffset::from_hms(1, 0, 0).expect("valid"));
    let west = Zone::Fixed(time::UtcOffset::from_hms(-5, 0, 0).expect("valid"));

    assert_eq!(
        super::wait_text_in(&round(), &asked, east),
        "round round-20261121-132909-q1w2 waits for GitHub: refresh the description of the pull requests, tried 4 times, next try 2026-11-21 01:07 +01:00"
    );
    assert_eq!(
        super::wait_text_in(&round(), &asked, west),
        "round round-20261121-132909-q1w2 waits for GitHub: refresh the description of the pull requests, tried 4 times, next try 2026-11-20 19:07 -05:00"
    );
}
