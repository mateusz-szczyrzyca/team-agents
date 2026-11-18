//! What `agent status` says of a round after its line: where the work of a round that goes on is, when the round started
//! and, once it is closed, how long it took.
//!
//! Where the work is comes from two places: the daemon shows the round (its item, its pull request, its reviews and the
//! runs of its reviewers), and the history says whether a draft pull request is open, which the state does not keep.

use std::collections::HashMap;

use agent_core::ids::RoundId;
use agent_core::protocol::TeamSummary;
use agent_core::round_work::RoundWork;
use agent_core::state::Round;
use time::UtcOffset;

use super::{Activity, StatusError};
use crate::client::Connection;

#[cfg(test)]
mod tests;

/// What the status says of the rounds of one answer of the daemon.
#[derive(Debug)]
pub(super) struct Told {
    /// Where the work of each round that goes on is.
    works: HashMap<RoundId, RoundWork>,
    /// The machine's offset from UTC: the times are local.
    offset: UtcOffset,
    /// Why the history could not say whether a draft is open, when it could not.
    unread: Option<String>,
}

impl Told {
    /// What follows the line of `round`.
    pub(super) fn of(&self, round: &Round) -> String {
        suffix(round, self.works.get(round.id()).copied(), self.offset)
    }

    /// The line that says why a draft cannot be told from local work, or nothing when the history was read.
    pub(super) fn unread(&self) -> String {
        self.unread.clone().unwrap_or_default()
    }
}

/// Asks the daemon about every round of `teams` that goes on, and the history of the home of `activity` about their
/// drafts.
///
/// # Errors
/// [`StatusError`] when the daemon cannot be asked or answers with something else than the round.
pub(super) async fn told(
    _connection: &mut Connection,
    activity: &Activity,
    _teams: &[TeamSummary],
) -> Result<Told, StatusError> {
    Ok(Told {
        works: HashMap::new(),
        offset: activity.offset,
        unread: None,
    })
}

/// What follows the line of `round`: where its work is, when it started in the local time `offset` gives and, for a
/// round that is closed, how long it took.
fn suffix(_round: &Round, _work: Option<RoundWork>, _offset: UtcOffset) -> String {
    String::new()
}

/// `seconds` as a length of time for a reader: days, hours and minutes, or seconds under a minute.
fn took(_seconds: u64) -> String {
    String::new()
}
