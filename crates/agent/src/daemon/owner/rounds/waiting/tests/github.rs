use std::num::NonZeroU32;

use agent_core::ids::TeamId;
use agent_core::state::Timestamp;
use agent_runtime::github::Outlook;

use crate::daemon::owner::rounds::waiting::github::{MAX_WAITS, Step, Waits, clock_time};

fn team() -> TeamId {
    "team-20261120-070000-wt01".parse().unwrap()
}

fn at(text: &str) -> Timestamp {
    text.parse().unwrap()
}

fn round() -> Step {
    Step::Round {
        round: "round-20261120-070000-wt01".parse().unwrap(),
    }
}

const GET: &str = "get pull request";

#[test]
fn a_first_failure_waits_thirty_seconds_and_each_further_one_doubles_the_wait() {
    let mut waits = Waits::default();
    let start = at("2026-11-20T00:00:55Z");

    let first = waits
        .failed(
            &round(),
            &team(),
            GET,
            None,
            "timed out".to_owned(),
            Outlook::MayPass,
            start,
        )
        .unwrap()
        .cloned()
        .unwrap();
    assert_eq!(first.tries, NonZeroU32::MIN);
    assert_eq!(first.next_at, Some(at("2026-11-20T00:01:25Z")));

    let second = waits
        .failed(
            &round(),
            &team(),
            GET,
            None,
            "timed out".to_owned(),
            Outlook::MayPass,
            at("2026-11-20T00:01:25Z"),
        )
        .unwrap()
        .cloned()
        .unwrap();
    assert_eq!(second.tries, NonZeroU32::new(2).unwrap());
    assert_eq!(second.next_at, Some(at("2026-11-20T00:02:25Z")));

    let third = waits
        .failed(
            &round(),
            &team(),
            GET,
            None,
            "timed out".to_owned(),
            Outlook::MayPass,
            at("2026-11-20T00:02:25Z"),
        )
        .unwrap()
        .cloned()
        .unwrap();
    assert_eq!(third.tries, NonZeroU32::new(3).unwrap());
    assert_eq!(third.next_at, Some(at("2026-11-20T00:04:25Z")));
}

#[test]
fn a_wait_runs_out_only_when_its_time_comes() {
    let mut waits = Waits::default();
    waits
        .failed(
            &round(),
            &team(),
            GET,
            None,
            "timed out".to_owned(),
            Outlook::MayPass,
            at("2026-11-20T01:00:00Z"),
        )
        .unwrap();

    assert!(!waits.due(&round(), GET, at("2026-11-20T01:00:29Z")));
    assert!(waits.due(&round(), GET, at("2026-11-20T01:00:30Z")));
    assert!(waits.due(&super::Step::Team { team: team() }, GET, at("2026-11-20T01:00:00Z")));
}

#[test]
fn a_refusal_waits_for_the_human_and_is_forgotten_by_resuming_its_team() {
    let mut waits = Waits::default();
    waits
        .failed(
            &round(),
            &team(),
            GET,
            None,
            "404 Not Found".to_owned(),
            Outlook::WillNotPass,
            at("2026-11-20T01:00:00Z"),
        )
        .unwrap();

    assert!(!waits.due(&round(), GET, at("2027-11-20T01:00:00Z")));

    waits.forget_team(&team());

    assert!(waits.due(&round(), GET, at("2026-11-20T01:00:00Z")));
}

#[test]
fn the_line_of_a_wait_names_the_step_the_tries_and_the_next_try_in_local_time() {
    let mut waits = Waits::default();
    let wait = waits
        .failed(
            &round(),
            &team(),
            GET,
            Some(agent_core::state::PrNumber::new(NonZeroU32::new(138).unwrap())),
            "timed out".to_owned(),
            Outlook::MayPass,
            at("2026-11-20T00:04:25Z"),
        )
        .unwrap()
        .cloned()
        .unwrap();
    let two_hours = time::UtcOffset::from_hms(2, 0, 0).unwrap();

    assert_eq!(
        wait.line(two_hours),
        "get pull request #138, tried 1 time, next try 02:05"
    );
    assert_eq!(clock_time(at("2026-11-20T00:07:00Z"), two_hours), "02:07");
}

#[test]
fn never_more_than_the_limit_of_waits_is_kept() {
    let mut waits = Waits::default();
    for index in 0..=MAX_WAITS {
        let step = Step::Round {
            round: format!("round-20261120-070000-{index:04}").parse().unwrap(),
        };
        waits
            .failed(
                &step,
                &team(),
                GET,
                None,
                "timed out".to_owned(),
                Outlook::MayPass,
                at("2026-11-20T00:00:00Z"),
            )
            .unwrap();
    }

    assert_eq!(waits.steps.len(), MAX_WAITS);
    assert!(
        waits
            .for_round(&"round-20261120-070000-0000".parse().unwrap(), GET)
            .is_none()
    );
}

#[test]
fn a_step_that_succeeds_leaves_no_wait_behind() {
    let mut waits = Waits::default();
    waits
        .failed(
            &round(),
            &team(),
            GET,
            None,
            "timed out".to_owned(),
            Outlook::MayPass,
            at("2026-11-20T00:00:00Z"),
        )
        .unwrap();

    assert!(waits.done(&round(), GET));

    assert!(!waits.done(&round(), GET));
    assert!(waits.due(&round(), GET, at("2026-11-20T00:00:00Z")));
}
