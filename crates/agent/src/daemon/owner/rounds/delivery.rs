//! Verification and handling of coordinator delivery reports (`DELIVERED: #N`).

use agent_core::ids::{ItemId, RoundId, TaskId, TeamId};
use agent_core::kinds::Role;
use agent_core::log::LogEvent;
use agent_core::protocol::EventKind;
use agent_core::state::{Completion, ItemPhase, PrNumber, PullRequest, RoundEnd, RoundLife, Timestamp};
use agent_core::status::ItemStatus;
use agent_runtime::git::{Revision, TreeState};

use super::super::teams::Look;
use super::super::{Owner, OwnerError};
use super::agent_name;

impl Owner {
    /// Handles a coordinator's delivery report for `team_id`.
    /// Returns `true` if accepted, `false` if refused.
    pub(in crate::daemon::owner) async fn coordinator_delivered(
        &mut self,
        team_id: &TeamId,
        task: &TaskId,
        delivered_pr: PrNumber,
        look: &mut Look,
    ) -> Result<bool, OwnerError> {
        let _ = (team_id, task, delivered_pr, look);
        Ok(false)
    }
}
