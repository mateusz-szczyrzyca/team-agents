//! The steps of the owner that call GitHub or git, and what a failure of theirs does: it waits, and ends nothing. The
//! evidence of 20.11.2026 is the second opinion of a pull request whose read of GitHub timed out three times and
//! stopped the whole daemon for the night, and the schema below is what the places of the owner must keep doing.
use std::num::NonZeroU32;

use agent_core::ids::TeamId;
use agent_core::state::{PrNumber, Timestamp};
use agent_runtime::git::GitError;
use agent_runtime::github::{GitHubError, GitHubOperation, Outlook, Rejection};

use crate::daemon::owner::rounds::waiting::github::{
    Failure, MAX_WAITS, Step, WaitingStep, Waits, clock_time, outlook_of,
};

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

/// The step of a second opinion: reading the pull request before the investigator's worktree is made.
const OPINION: WaitingStep = WaitingStep::AskOpinion;

#[test]
fn a_first_failure_waits_thirty_seconds_and_each_further_one_doubles_the_wait() {
    let mut waits = Waits::default();
    let (first_failure, first) = waits
        .failed(
            &round(),
            &team(),
            OPINION,
            Some(PrNumber::new(NonZeroU32::new(138).unwrap())),
            "timed out".to_owned(),
            Outlook::MayPass,
            at("2026-11-20T00:00:55Z"),
        )
        .unwrap();
    assert_eq!(first_failure, Failure::First);
    assert_eq!(first.tries, NonZeroU32::MIN);
    assert_eq!(first.next_at, Some(at("2026-11-20T00:01:25Z")));

    let (second_failure, second) = waits
        .failed(
            &round(),
            &team(),
            OPINION,
            Some(PrNumber::new(NonZeroU32::new(138).unwrap())),
            "timed out".to_owned(),
            Outlook::MayPass,
            at("2026-11-20T00:01:25Z"),
        )
        .unwrap();
    assert_eq!(second_failure, Failure::Again);
    assert_eq!(second.tries, NonZeroU32::new(2).unwrap());
    assert_eq!(second.next_at, Some(at("2026-11-20T00:02:25Z")));
}

#[test]
fn a_wait_runs_out_only_when_its_time_comes() {
    let mut waits = Waits::default();
    waits
        .failed(
            &round(),
            &team(),
            OPINION,
            None,
            "timed out".to_owned(),
            Outlook::MayPass,
            at("2026-11-20T01:00:00Z"),
        )
        .unwrap();

    assert!(!waits.due(&round(), OPINION, at("2026-11-20T01:00:29Z")));
    assert!(waits.due(&round(), OPINION, at("2026-11-20T01:00:30Z")));
    assert!(waits.due(&Step::Team { team: team() }, OPINION, at("2026-11-20T01:00:00Z")));
}

#[test]
fn a_refusal_waits_for_the_human_and_is_forgotten_by_resuming_its_team() {
    let mut waits = Waits::default();
    waits
        .failed(
            &round(),
            &team(),
            OPINION,
            Some(PrNumber::new(NonZeroU32::new(138).unwrap())),
            "get pull request was refused with status 404".to_owned(),
            Outlook::WillNotPass,
            at("2026-11-20T01:00:00Z"),
        )
        .unwrap();

    assert!(!waits.due(&round(), OPINION, at("2027-11-20T01:00:00Z")));

    waits.forget_team(&team());

    assert!(waits.due(&round(), OPINION, at("2026-11-20T01:00:00Z")));
}

#[test]
fn the_line_of_a_wait_names_the_step_the_tries_and_the_next_try_in_local_time() {
    let mut waits = Waits::default();
    let (_, wait) = waits
        .failed(
            &round(),
            &team(),
            WaitingStep::AskOpinion,
            Some(PrNumber::new(NonZeroU32::new(138).unwrap())),
            "timed out".to_owned(),
            Outlook::MayPass,
            at("2026-11-20T00:04:25Z"),
        )
        .unwrap();
    let two_hours = time::UtcOffset::from_hms(2, 0, 0).unwrap();

    assert_eq!(
        wait.line(two_hours),
        "prepare a second opinion #138, tried 1 time, next try 02:05"
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
                OPINION,
                None,
                "timed out".to_owned(),
                Outlook::MayPass,
                at("2026-11-20T00:00:00Z"),
            )
            .unwrap();
    }

    assert_eq!(waits.kept(), MAX_WAITS);
    assert!(
        waits
            .for_round(&"round-20261120-070000-0000".parse().unwrap(), OPINION)
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
            OPINION,
            None,
            "timed out".to_owned(),
            Outlook::MayPass,
            at("2026-11-20T00:00:00Z"),
        )
        .unwrap();

    assert!(waits.done(&round(), OPINION));

    assert!(!waits.done(&round(), OPINION));
    assert!(waits.due(&round(), OPINION, at("2026-11-20T00:00:00Z")));
}

#[test]
fn a_network_a_timeout_and_github_failing_may_pass_and_a_refusal_will_not() {
    let cases = [
        (
            GitHubError::Timeout {
                operation: GitHubOperation::GetPullRequest,
                attempts: 3,
                limit: std::time::Duration::from_secs(30),
            },
            Outlook::MayPass,
        ),
        (
            GitHubError::Status {
                operation: GitHubOperation::GetPullRequest,
                status: 503,
                reason: Rejection::Server,
                retry_after: None,
                message: String::new(),
            },
            Outlook::MayPass,
        ),
        (
            GitHubError::Status {
                operation: GitHubOperation::GetPullRequest,
                status: 429,
                reason: Rejection::RateLimited,
                retry_after: None,
                message: String::new(),
            },
            Outlook::MayPass,
        ),
        (
            GitHubError::Status {
                operation: GitHubOperation::GetPullRequest,
                status: 404,
                reason: Rejection::NotFound,
                retry_after: None,
                message: "Not Found".to_owned(),
            },
            Outlook::WillNotPass,
        ),
        (
            GitHubError::Status {
                operation: GitHubOperation::GetPullRequest,
                status: 401,
                reason: Rejection::Unauthorized,
                retry_after: None,
                message: "Bad credentials".to_owned(),
            },
            Outlook::WillNotPass,
        ),
    ];
    for (error, expected) in cases {
        let publish = crate::daemon::owner::rounds::PublishError::from(error);
        assert_eq!(outlook_of(&publish), expected, "{publish:?}");
    }
}

#[test]
fn a_git_failure_waits_only_when_it_reaches_github() {
    let refused = GitError::UnknownRevision {
        revision: "HEAD".to_owned(),
    };
    let publish = crate::daemon::owner::rounds::PublishError::Git(refused);

    assert_eq!(outlook_of(&publish), Outlook::WillNotPass);
}

#[test]
fn a_next_try_on_a_whole_minute_is_shown_at_that_minute() {
    let mut waits = Waits::default();
    let (_, wait) = waits
        .failed(
            &round(),
            &team(),
            OPINION,
            Some(PrNumber::new(NonZeroU32::new(138).unwrap())),
            "timed out".to_owned(),
            Outlook::MayPass,
            at("2026-11-20T00:04:30Z"),
        )
        .unwrap();

    assert_eq!(wait.next_at, Some(at("2026-11-20T00:05:00Z")));
    assert_eq!(
        wait.line(time::UtcOffset::UTC),
        "prepare a second opinion #138, tried 1 time, next try 00:05"
    );
}

#[test]
fn a_refusal_that_waits_for_the_human_is_never_given_up_for_a_new_wait() {
    let mut waits = Waits::default();
    let refused = |index: usize| Step::Round {
        round: format!("round-20261120-070000-{index:04}").parse().unwrap(),
    };
    for index in 0..=MAX_WAITS {
        waits
            .failed(
                &refused(index),
                &team(),
                OPINION,
                None,
                "refused with status 404".to_owned(),
                Outlook::WillNotPass,
                at("2026-11-20T00:00:00Z"),
            )
            .unwrap();
    }

    for index in [0, MAX_WAITS] {
        assert!(
            !waits.due(&refused(index), OPINION, at("2027-11-20T00:00:00Z")),
            "refusal {index} is tried without the human's resume"
        );
    }
}
