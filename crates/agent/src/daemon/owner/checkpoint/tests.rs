//! What the owner takes from a record of a round's history: the checkpoint it leaves, the pull request it names and the
//! round it closes. The checkpoints themselves are driven through a daemon in `rounds/tests/checkpoints.rs`.

use std::num::NonZeroU32;

use agent_core::history::{CheckpointCause, HistoryEvent};
use agent_core::ids::RoundId;
use agent_core::state::{CommitId, PrNumber, RoundEnd};

use super::Step;

/// The number 7.
fn seven() -> PrNumber {
    PrNumber::new(NonZeroU32::new(7).unwrap())
}

#[test]
fn a_pull_request_marked_ready_leaves_its_checkpoint_and_names_its_number() {
    let step = Step::of(&HistoryEvent::PullReady { pr: seven() });

    assert_eq!(step.cause, Some(CheckpointCause::PullReady));
    assert_eq!(step.pull_request, Some(seven()));
    assert_eq!(step.closed, None);
}

#[test]
fn a_push_names_the_pull_request_it_went_into() {
    let head: CommitId = "0123456789abcdef0123456789abcdef01234567".parse().unwrap();

    let step = Step::of(&HistoryEvent::PullPushed { pr: seven(), head });

    assert_eq!(step.cause, Some(CheckpointCause::PullPushed));
    assert_eq!(step.pull_request, Some(seven()));
}

#[test]
fn the_close_of_a_round_names_it_and_leaves_no_checkpoint_of_its_own() {
    let round: RoundId = "round-20261126-180909-vet6".parse().unwrap();
    let step = Step::of(&HistoryEvent::RoundClosed {
        round: round.clone(),
        end: RoundEnd::Merged,
    });

    assert_eq!(step.closed, Some(round));
    assert_eq!(step.cause, None);
    assert_eq!(step.pull_request, None);
}

#[test]
fn a_record_that_is_no_step_of_a_round_leaves_nothing() {
    let step = Step::of(&HistoryEvent::TeamEnded {});

    assert_eq!((step.cause, step.pull_request, step.closed), (None, None, None));
}
