//! What `agent status --investigators` says of each configured investigator: the last two activities in the teams that are
//! not closed, newest first, or that it is free.

use agent_core::ids::InvestigatorName;
use agent_core::protocol::TeamSummary;
use agent_core::state::Activity;
use time::UtcOffset;

/// The view of the investigators `configured` in `teams`: a line for each of the last two activities of each of them in
/// the teams that are not closed, newest first, and a line for the one that has none. The times are the local ones at
/// `offset`.
pub(crate) fn render(_teams: &[TeamSummary], _configured: &[InvestigatorName], _offset: UtcOffset) -> String {
    String::new()
}

#[cfg(test)]
mod tests;
