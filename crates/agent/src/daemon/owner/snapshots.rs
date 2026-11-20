//! The snapshots the daemon takes when it starts: one of each task whose run a restart cut, when the round the run worked
//! for is still open. What the run left in the worktree is kept before anything else touches it; a snapshot that fails is
//! logged and the daemon goes on.

use std::str::FromStr;

use agent_core::config::AbsolutePath;
use agent_core::history::Record;
use agent_core::ids::{RoundId, TaskId, TeamId};
use agent_core::log::LogEvent;
use agent_core::state::Timestamp;
use agent_runtime::process::error_chain;
use agent_runtime::snapshot::take_snapshot;
use tracing::Instrument;

use super::{Owner, OwnerError};

/// One megabyte, in bytes: the unit of `[snapshot] warn_mb`.
const MEGABYTE: u64 = 1024 * 1024;

impl Owner {
    /// Takes a snapshot of each of `tasks` whose run is in a round that is still open, and records it in the history.
    ///
    /// # Errors
    /// [`OwnerError`] when the clock cannot be read. A snapshot that fails is logged, not returned.
    pub(super) async fn snapshot_interrupted(&mut self, tasks: &[TaskId]) -> Result<(), OwnerError> {
        let now = Timestamp::now(&self.parts.clock)?;
        let warn_bytes = self.parts.settings.config.snapshot().warn_mb.saturating_mul(MEGABYTE);
        for task in tasks {
            let Some((team, round, worktree)) = self.snapshot_target(task) else {
                continue;
            };
            let span = tracing::error_span!("snapshot", team = %team, round = %round, task = %task);
            match take_snapshot(&worktree, &round, task, now, warn_bytes)
                .instrument(span.clone())
                .await
            {
                Ok(record) => self.history.record(Record::Snapshot(Box::new(record))),
                Err(error) => {
                    let _entered = span.enter();
                    tracing::warn!(
                        event = LogEvent::SnapshotFailed.as_str(),
                        task = %task,
                        round = %round,
                        error = error_chain(&error),
                        "a snapshot of an interrupted run cannot be taken: the daemon goes on without it"
                    );
                }
            }
        }
        Ok(())
    }

    /// The team, open round the last run of `task` worked for, and the worktree of `task`; `None` when the task has no
    /// run or no open round, or its worktree path cannot be written as a path of the configuration.
    fn snapshot_target(&self, task: &TaskId) -> Option<(TeamId, RoundId, AbsolutePath)> {
        let found = self.parts.state.find_task(task)?;
        let run = found.member.runs().last()?;
        let round = run
            .rounds()
            .iter()
            .rev()
            .find(|round| found.team.round(round).is_some_and(agent_core::state::Round::is_open))?
            .clone();
        let path = self.parts.store.home().worktree_dir(found.team.repo(), task);
        let text = path.to_str()?;
        let Ok(worktree) = AbsolutePath::from_str(text) else {
            return None;
        };
        Some((found.team.id().clone(), round, worktree))
    }
}
