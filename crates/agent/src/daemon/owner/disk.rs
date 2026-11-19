//! Free disk space of the home and the build directories released when it runs short. The measures and removals walk
//! the disk, so they run on the blocking pool; the owner's loop takes their outcomes from `disk_jobs` and acts on them.

use std::path::{Path, PathBuf};

use agent_core::ids::TaskId;
use agent_core::log::LogEvent;
use agent_core::prompt::{HarnessPrompt, Variables};
use agent_runtime::disk::{
    BuildDirError, DirectorySizeError, FreeSpace, GIBIBYTE, WorktreeUsage, gibibytes_text, largest_worktrees,
    remove_build_directory,
};
use agent_runtime::notify::Notification;
use agent_runtime::process::error_chain;
use tokio::task::JoinError;

use super::Owner;

/// What became of one build directory: the bytes released, or the reason it was kept.
pub(super) enum BuildDirOutcome {
    /// The directory is gone, and this many bytes were released (zero when there was none).
    Released {
        /// Bytes of the removed directory.
        bytes: u64,
    },
    /// The directory was kept, with the cause of the refusal or the failure.
    Kept {
        /// The error chain of the refusal or failure.
        reason: String,
    },
}

impl From<Result<u64, BuildDirError>> for BuildDirOutcome {
    /// Converts the result of a removal: `Ok` releases its bytes, `Err` keeps the directory and its cause.
    fn from(result: Result<u64, BuildDirError>) -> Self {
        match result {
            Ok(bytes) => Self::Released { bytes },
            Err(error) => Self::Kept {
                reason: error_chain(&error),
            },
        }
    }
}

/// The outcome of disk work that ran off the owner's loop; the loop acts on it when it is ready.
pub(super) enum DiskJob {
    /// The removal of the build directory of `task` ended.
    Target {
        /// The task whose worktree held the directory.
        task: TaskId,
        /// Released bytes, or why the directory was kept.
        outcome: BuildDirOutcome,
    },
    /// The low-space notice is ready to be sent; its list of the largest worktrees was measured off the loop.
    LowSpace {
        /// Free space in whole gibibytes at the crossing.
        free_gb: u64,
        /// The `disk.low_free_gb` threshold in whole gibibytes.
        limit_gb: u64,
        /// The largest worktrees, or why they could not be listed.
        largest: Result<Vec<WorktreeUsage>, DirectorySizeError>,
    },
}

impl Owner {
    /// Measures the free disk space of the home, records it for `agent status`, and starts the low-space notice when the
    /// space falls below `disk.low_free_gb`. `None` when the space cannot be measured: the caller then starts nothing.
    pub(super) async fn check_free_space(&mut self) -> Option<u64> {
        let free_bytes = match self.parts.free_space.free_space_bytes(self.parts.store.home().root()) {
            Ok(bytes) => bytes,
            Err(error) => {
                tracing::warn!(error = %error_chain(&error), "cannot measure free disk space: no run starts until it can be measured");
                return None;
            }
        };
        if let Err(error) = std::fs::write(self.parts.store.home().free_space_file(), free_bytes.to_string()) {
            tracing::warn!(error = %error_chain(&error), "cannot record free disk space for agent status");
        }

        let low_limit_gb = self.parts.settings.config.disk().low_free_gb;
        let is_low = free_bytes < low_limit_gb.saturating_mul(GIBIBYTE);
        if is_low && !self.was_low_space {
            self.was_low_space = true;
            self.start_low_space_notice(free_bytes / GIBIBYTE, low_limit_gb);
        } else if !is_low {
            self.was_low_space = false;
        }

        let min_limit_gb = self.parts.settings.config.disk().min_free_gb;
        if free_bytes >= min_limit_gb.saturating_mul(GIBIBYTE) {
            self.was_min_space = false;
        }
        Some(free_bytes)
    }

    /// Removes the build directory of `worktree_dir`; the outcome is logged by the loop when the removal ends.
    pub(super) fn remove_target(&mut self, task: &TaskId, worktree_dir: &Path) {
        let home = self.parts.store.home().clone();
        let dir: PathBuf = worktree_dir.to_path_buf();
        let task = task.clone();
        self.disk_jobs.spawn_blocking(move || DiskJob::Target {
            task,
            outcome: BuildDirOutcome::from(remove_build_directory(&home, &dir)),
        });
    }

    /// Removes the build directory of `task` once its run has ended, when a round closed while the run went on.
    pub(super) fn release_deferred_target(&mut self, task: &TaskId) {
        if let Some(worktree_dir) = self.deferred_targets.remove(task) {
            self.remove_target(task, &worktree_dir);
        }
    }

    /// Starts the measure of the largest worktrees for the low-space notice, which is sent when it is ready.
    fn start_low_space_notice(&mut self, free_gb: u64, limit_gb: u64) {
        let home = self.parts.store.home().clone();
        self.disk_jobs.spawn_blocking(move || DiskJob::LowSpace {
            free_gb,
            limit_gb,
            largest: largest_worktrees(&home, 5),
        });
    }

    /// Acts on the outcome of a disk job the loop has taken: logs a removal, sends the low-space notice.
    pub(super) async fn disk_job_done(&mut self, joined: Result<DiskJob, JoinError>) {
        match joined {
            Ok(DiskJob::Target { task, outcome }) => log_removal(&task, outcome),
            Ok(DiskJob::LowSpace {
                free_gb,
                limit_gb,
                largest,
            }) => self.send_low_space_notice(free_gb, limit_gb, largest).await,
            Err(error) => {
                tracing::warn!(error = %error_chain(&error), "a disk job ended without an outcome");
            }
        }
    }

    /// Tells the human that free disk space is below `disk.low_free_gb`, with the largest worktrees or the reason they
    /// could not be listed.
    async fn send_low_space_notice(
        &mut self,
        free_gb: u64,
        limit_gb: u64,
        largest: Result<Vec<WorktreeUsage>, DirectorySizeError>,
    ) {
        let worktrees = match largest {
            Ok(list) if list.is_empty() => "none".to_owned(),
            Ok(list) => list
                .iter()
                .map(|usage| format!("- {}: {}", usage.path.display(), gibibytes_text(usage.bytes)))
                .collect::<Vec<_>>()
                .join("\n"),
            Err(error) => format!("cannot list the worktrees: {}", error_chain(&error)),
        };
        let values = match Variables::new()
            .with("free", free_gb.to_string())
            .and_then(|v| v.with("limit", limit_gb.to_string()))
            .and_then(|v| v.with("worktrees", worktrees))
        {
            Ok(values) => values,
            Err(error) => {
                tracing::warn!(error = %error_chain(&error), "cannot build the variables of the low disk notice");
                return;
            }
        };
        let rendered = match self.parts.texts.render(HarnessPrompt::NoticeLowDisk, &values) {
            Ok(rendered) => rendered,
            Err(error) => {
                tracing::warn!(error = %error_chain(&error), "cannot render the low disk notice");
                return;
            }
        };
        let mut split = rendered.splitn(2, '\n');
        let title = split.next().unwrap_or("Low free disk space").trim();
        let body = split.next().unwrap_or_default().trim();
        self.deliver(title, body, "low disk notice").await;
    }

    /// Sends a notification, logging a failure: a notice that cannot be sent must not stop the owner.
    pub(super) async fn deliver(&mut self, title: &str, body: &str, what: &str) {
        let notification = match Notification::new(title, body) {
            Ok(notification) => notification,
            Err(error) => {
                tracing::warn!(event = LogEvent::NotificationFailed.as_str(), error = %error_chain(&error), "cannot build the {what}");
                return;
            }
        };
        let now = match agent_core::state::Timestamp::now(&self.parts.clock) {
            Ok(now) => now,
            Err(error) => {
                tracing::warn!(event = LogEvent::NotificationFailed.as_str(), error = %error_chain(&error), "cannot time the {what}");
                return;
            }
        };
        if let Err(error) = self.parts.notifier.notify(now, &notification).await {
            tracing::warn!(event = LogEvent::NotificationFailed.as_str(), error = %error_chain(&error), "cannot deliver the {what}");
        }
    }
}

/// Logs one removal of a build directory: released bytes at `info`, a kept directory at `warn`.
fn log_removal(task: &TaskId, outcome: BuildDirOutcome) {
    match outcome {
        BuildDirOutcome::Released { bytes } => {
            if bytes > 0 {
                tracing::info!(
                    event = LogEvent::BuildDirectoryRemoved.as_str(),
                    task = %task,
                    bytes = bytes,
                    "build directory removed"
                );
            }
        }
        BuildDirOutcome::Kept { reason } => {
            tracing::warn!(
                event = LogEvent::GitCallFailed.as_str(),
                task = %task,
                error = %reason,
                "cannot remove build directory"
            );
        }
    }
}
