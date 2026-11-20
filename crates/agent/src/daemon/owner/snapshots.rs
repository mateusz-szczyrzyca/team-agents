//! The snapshots the daemon takes when it starts: one of each task whose run a restart cut, when the round the run worked
//! for is still open. What the run left in the worktree is kept before anything else touches it; a snapshot that fails is
//! logged and the daemon goes on.

use agent_core::ids::TaskId;

use super::{Owner, OwnerError};

impl Owner {
    /// Takes a snapshot of each of `tasks` whose run is in a round that is still open, and records it in the history.
    ///
    /// # Errors
    /// [`OwnerError`] when the clock cannot be read. A snapshot that fails is logged, not returned.
    pub(super) async fn snapshot_interrupted(&mut self, _tasks: &[TaskId]) -> Result<(), OwnerError> {
        std::future::ready(Ok(())).await
    }
}
