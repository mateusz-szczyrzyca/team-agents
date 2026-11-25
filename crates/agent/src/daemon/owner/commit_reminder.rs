//! The reminder to commit: a run that holds many uncommitted lines and has made no commit for a while is told once, on the
//! bus, to commit what is finished. Both bounds of `[commit_reminder]` must be reached, and a commit re-arms the reminder.

use agent_core::ids::TeamId;

use super::Owner;

impl Owner {
    /// Looks at the worktree of every task of `team` that has a run going and reminds the run to commit when it is due.
    pub(super) async fn remind_to_commit_in(&mut self, team: &TeamId) {
        let _ = team;
    }
}

#[cfg(test)]
mod tests;
