//! What the line of a reviewer says: what it did or does, from the record of the team's activities, instead of the one
//! word of its status, which says `waiting` between its reviews.

use agent_core::protocol::MemberSummary;
use agent_core::state::Activity;
use time::UtcOffset;

/// What the line of the reviewer `member` says, `None` when the team recorded no activity of it: the activity replaces
/// the line of its status for a reviewer.
pub(crate) fn status(_member: &MemberSummary, _activities: &[Activity], _offset: UtcOffset) -> Option<String> {
    None
}
