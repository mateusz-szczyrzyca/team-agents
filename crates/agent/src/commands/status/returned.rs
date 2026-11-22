//! What `agent status` says of the returns of an item: how often the lead sent its work back to the member that holds it.

use agent_core::protocol::{MemberSummary, Returned};
use agent_core::state::Round;

/// What follows the status of the task line of `member`: `, returned 3 of 5` once its lead returned the item it holds,
/// and `, returned 3` when there is no limit; nothing before the first return.
pub(super) fn task_note(member: &MemberSummary) -> String {
    member
        .returned
        .as_ref()
        .map_or_else(String::new, |returned| format!(", {}", count(returned)))
}

/// What follows the round line of `round` when the limit of returns of its item is near, that is when one more return
/// reaches it: `, item returned 4 of 5`; nothing otherwise, and nothing when there is no limit.
pub(super) fn round_note(round: &Round, members: &[MemberSummary]) -> String {
    let Some(item) = round.item() else {
        return String::new();
    };
    members
        .iter()
        .filter_map(|member| member.returned.as_ref())
        .find(|returned| returned.item == item)
        .filter(|returned| {
            returned
                .limit
                .is_some_and(|limit| returned.counted.get().saturating_add(1) >= limit.get())
        })
        .map_or_else(String::new, |returned| format!(", item {}", count(returned)))
}

/// `returned 3 of 5`, or `returned 3` when there is no limit.
fn count(returned: &Returned) -> String {
    returned.limit.map_or_else(
        || format!("returned {}", returned.counted),
        |limit| format!("returned {} of {limit}", returned.counted),
    )
}
