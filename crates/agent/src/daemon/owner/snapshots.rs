//! The snapshots the daemon takes when it starts: one of each task whose run a restart or stop cut, when the round the run
//! worked for is still open. What the run left in the worktree is kept before anything else touches it; a snapshot that
//! fails is logged and the daemon goes on.

use std::path::{Path, PathBuf};
use std::str::FromStr;

use agent_core::config::AbsolutePath;
use agent_core::history::Record;
use agent_core::ids::{RoundId, TaskId, TeamId};
use agent_core::log::LogEvent;
use agent_core::state::{RunEnd, RunState, Timestamp};
use agent_runtime::history::Reader;
use agent_runtime::process::error_chain;
use agent_runtime::snapshot::take_snapshot;
use tracing::Instrument;

use super::Owner;

/// One megabyte, in bytes: the unit of `[snapshot] warn_mb`.
const MEGABYTE: u64 = 1024 * 1024;

/// A task whose last run was cut by a crash or a graceful stop while working for an open round.
struct CutTask {
    /// The team the task belongs to.
    team: TeamId,
    /// The open round the interrupted run worked for.
    round: RoundId,
    /// The task whose worktree is to be snapshotted.
    task: TaskId,
    /// The worktree path on disk, before configuration path validation.
    path: PathBuf,
    /// When a graceful stop ended the run (`None` when an unclean crash left the run in `Running` until `recover`).
    graceful_end: Option<Timestamp>,
}

impl Owner {
    /// Takes a snapshot of each task whose last run was cut by a crash (`interrupted`) or by a graceful stop of the daemon
    /// and whose round is still open, and records it in the history as of `now`.
    pub(super) async fn snapshot_interrupted(&mut self, interrupted: &[TaskId], now: Timestamp) {
        let warn_bytes = self.parts.settings.config.snapshot().warn_mb.saturating_mul(MEGABYTE);
        let history_path = self.parts.store.home().history_file();
        for target in self.cut_tasks(interrupted) {
            if let Some(ended_at) = target.graceful_end
                && already_snapshotted(&history_path, &target.round, &target.task, ended_at)
            {
                continue;
            }
            let span = tracing::error_span!(
                "snapshot",
                team = %target.team,
                round = %target.round,
                task = %target.task
            );
            let worktree = match validate_worktree_path(&target.path) {
                Ok(worktree) => worktree,
                Err(cause) => {
                    let _entered = span.enter();
                    tracing::warn!(
                        event = LogEvent::SnapshotFailed.as_str(),
                        task = %target.task,
                        round = %target.round,
                        error = %cause,
                        "a snapshot of an interrupted run cannot be taken: the daemon goes on without it"
                    );
                    continue;
                }
            };
            match take_snapshot(&worktree, &target.round, &target.task, now, warn_bytes)
                .instrument(span.clone())
                .await
            {
                Ok(record) => self.history.record(Record::Snapshot(Box::new(record))),
                Err(error) => {
                    let _entered = span.enter();
                    tracing::warn!(
                        event = LogEvent::SnapshotFailed.as_str(),
                        task = %target.task,
                        round = %target.round,
                        error = error_chain(&error),
                        "a snapshot of an interrupted run cannot be taken: the daemon goes on without it"
                    );
                }
            }
        }
    }

    /// Every task in the state whose last run was cut by an unclean crash (`interrupted`) or by a graceful stop
    /// (`RunEnd::NoStatus { exit_code: None }`) and worked for a round that is still open.
    fn cut_tasks(&self, interrupted: &[TaskId]) -> Vec<CutTask> {
        let mut targets = Vec::new();
        for team in self.parts.state.teams() {
            let crew = team.crew().roster().into_iter().map(|(_, member)| member);
            for member in crew.chain(team.reviewers().iter()) {
                let task = member.task();
                let Some(run) = member.runs().last() else {
                    continue;
                };
                let graceful_end = if interrupted.contains(task) {
                    None
                } else if let RunState::Ended {
                    at,
                    end: RunEnd::NoStatus { exit_code: None },
                } = run.state()
                {
                    Some(*at)
                } else {
                    continue;
                };
                let Some(round) = run
                    .rounds()
                    .iter()
                    .rev()
                    .find(|round| team.round(round).is_some_and(agent_core::state::Round::is_open))
                    .cloned()
                else {
                    continue;
                };
                targets.push(CutTask {
                    team: team.id().clone(),
                    round,
                    task: task.clone(),
                    path: self.parts.store.home().worktree_dir(team.repo(), task),
                    graceful_end,
                });
            }
        }
        targets
    }
}

/// Validates `path` as an [`AbsolutePath`], returning the human-readable cause when `path` is not valid UTF-8 or not an
/// absolute path.
fn validate_worktree_path(path: &Path) -> Result<AbsolutePath, String> {
    let Some(text) = path.to_str() else {
        return Err(format!("the worktree path {} is not valid UTF-8", path.display()));
    };
    AbsolutePath::from_str(text).map_err(|error| error_chain(&error))
}

/// Whether `history_path` already records a snapshot of `task` in `round` taken at or after `ended_at`.
fn already_snapshotted(history_path: &Path, round: &RoundId, task: &TaskId, ended_at: Timestamp) -> bool {
    let Ok(reader) = Reader::open(history_path) else {
        return false;
    };
    let Ok(snapshots) = reader.snapshots_of_round(round) else {
        return false;
    };
    snapshots
        .iter()
        .any(|snapshot| &snapshot.task == task && snapshot.at >= ended_at)
}
