//! The request of a worker whose run has ended, kept in a file of the worker's task directory while the harness settles it:
//! its check, the bringing over of its commit and the answer. Each stage is written before the next starts, so that a
//! daemon that stops between two of them goes on from the stage the file names.

use std::path::{Path, PathBuf};

use agent_core::decide::worker_result::CheckOutcome;
use agent_core::ids::{AgentName, ItemId, TaskId, TeamId};
use serde::{Deserialize, Serialize};

use crate::agentbus::RequestId;

/// The start of the name of the file that keeps a request whose worker has ended: `worker-request-<number>.json` in the
/// worker's task directory.
const FILE_PREFIX: &str = "worker-request-";

/// How many files of one request may be kept aside before the next one is refused.
const MOST_ASIDE: u32 = 1000;

/// Why a request that a worker has ended cannot be kept or read.
#[derive(Debug, thiserror::Error)]
pub enum PendingError {
    /// The file of the request cannot be written.
    #[error("the request cannot be kept in {path:?}")]
    Write {
        /// The file.
        path: PathBuf,
        /// The failure.
        source: std::io::Error,
    },
    /// The directory of a worker's task cannot be read.
    #[error("the requests of a worker cannot be read from {path:?}")]
    List {
        /// The directory.
        path: PathBuf,
        /// The failure.
        source: std::io::Error,
    },
    /// The file of a request cannot be read.
    #[error("the request cannot be read from {path:?}")]
    Read {
        /// The file.
        path: PathBuf,
        /// The failure.
        source: std::io::Error,
    },
    /// The file of a request is not what the harness wrote.
    #[error("{path:?} is not the file of a request")]
    Parse {
        /// The file.
        path: PathBuf,
        /// The failure.
        source: serde_json::Error,
    },
    /// The file holds another request than the one its name says.
    #[error("{path:?} holds request H{found}, not the one in its name")]
    Mismatch {
        /// The file.
        path: PathBuf,
        /// The number of the request it holds.
        found: std::num::NonZeroU64,
    },
    /// A request cannot be written as JSON.
    #[error("the request cannot be written as JSON")]
    Encode(#[source] serde_json::Error),
    /// The clock gives a time that cannot be recorded.
    #[error("the clock gives a time that cannot be recorded")]
    Clock(#[source] agent_core::state::InvalidTimestamp),
}

impl PendingError {
    /// The file the failure is about; `None` when it is not about one.
    #[must_use]
    pub fn path(&self) -> Option<&Path> {
        match self {
            Self::Write { path, .. }
            | Self::List { path, .. }
            | Self::Read { path, .. }
            | Self::Parse { path, .. }
            | Self::Mismatch { path, .. } => Some(path),
            Self::Encode(_) | Self::Clock(_) => None,
        }
    }
}

/// The request a file named `worker-request-<n>.json` is for, from its name alone: it holds for a file whose content
/// cannot be read. `None` when the name is not of that form.
#[must_use]
pub fn request_of_file(path: &Path) -> Option<RequestId> {
    let digits = path
        .file_name()?
        .to_str()?
        .strip_prefix(FILE_PREFIX)?
        .strip_suffix(".json")?;
    digits.parse::<std::num::NonZeroU64>().ok().map(RequestId::new)
}

/// Moves a file of a request that cannot be used next to where it was, under a name that the reading does not meet
/// (`<name>.damaged`, or `<name>.damaged-2` and so on when that is taken): the file is kept, never deleted.
///
/// # Errors
/// [`PendingError::Write`] when the file cannot be moved.
pub async fn set_aside(path: &Path) -> Result<PathBuf, PendingError> {
    let name = path.file_name().and_then(|name| name.to_str()).unwrap_or(FILE_PREFIX);
    for attempt in 1_u32..=MOST_ASIDE {
        let suffix = if attempt == 1 {
            String::new()
        } else {
            format!("-{attempt}")
        };
        let target = path.with_file_name(format!("{name}.damaged{suffix}"));
        match tokio::fs::symlink_metadata(&target).await {
            // Taken: the next name is tried.
            Ok(_) => {}
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => {
                return tokio::fs::rename(path, &target)
                    .await
                    .map(|()| target)
                    .map_err(|source| PendingError::Write {
                        path: path.to_owned(),
                        source,
                    });
            }
            Err(source) => {
                return Err(PendingError::Write {
                    path: path.to_owned(),
                    source,
                });
            }
        }
    }
    Err(PendingError::Write {
        path: path.to_owned(),
        source: std::io::Error::new(
            std::io::ErrorKind::AlreadyExists,
            "too many files are set aside already",
        ),
    })
}

/// Whether the pending of `request` in `task_dir` was set aside (kept under a `*.damaged` name) by [`set_aside`]: a
/// pending that was set aside is what is left of a request whose workspace must never be released on its behalf, and
/// this is how the owner tells quarantine from a safely settled leftover.
///
/// # Errors
/// [`PendingError::List`] when the directory cannot be listed.
pub async fn is_set_aside(task_dir: &Path, request: RequestId) -> Result<bool, PendingError> {
    let mut entries = match tokio::fs::read_dir(task_dir).await {
        Ok(entries) => entries,
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(source) => {
            return Err(PendingError::List {
                path: task_dir.to_owned(),
                source,
            });
        }
    };
    let prefix = format!("{FILE_PREFIX}{}.json", request.number());
    loop {
        let entry = match entries.next_entry().await {
            Ok(Some(entry)) => entry,
            Ok(None) => return Ok(false),
            Err(source) => {
                return Err(PendingError::List {
                    path: task_dir.to_owned(),
                    source,
                });
            }
        };
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        // A set-aside file keeps the pending's name and adds `.damaged`, `.damaged-2`, ...: it is still the file of
        // `request`, only the reading does not meet it.
        if request_of_file(&entry.path()).is_none()
            && name.starts_with(&prefix)
            && name.strip_prefix(&prefix).is_some_and(|rest| rest.starts_with(".damaged"))
        {
            return Ok(true);
        }
    }
}

/// A request whose worker's run has ended: who asked, who served, what the worker said and how far the harness got.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Pending {
    /// The request.
    pub request: RequestId,
    /// The team.
    pub team: TeamId,
    /// The member that asked.
    pub requester: AgentName,
    /// Its task.
    pub requester_task: TaskId,
    /// The worker.
    pub worker: AgentName,
    /// Its task.
    pub worker_task: TaskId,
    /// The worker's report, without its status line.
    pub summary: String,
    /// How long the worker's run lasted, in seconds.
    pub run_seconds: u64,
    /// The commit the worker's worktree was at when the request was given to it; absent when it could not be read.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base: Option<String>,
    /// How far the harness got.
    pub stage: Stage,
}

/// How far the harness got with a request: each stage is written to the file before the next one starts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "stage", rename_all = "snake_case")]
pub enum Stage {
    /// The worker's run ended; nothing is checked yet.
    Ended,
    /// What the request asked is read, the worker's commit named and the check run.
    Checked(Box<Checked>),
    /// The result is decided and, when it is good, brought over; the answer is not given yet.
    Settled {
        /// What was checked.
        checked: Box<Checked>,
        /// What became of the result.
        settlement: Settlement,
    },
}

/// What the check of a request's result found.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Checked {
    /// The step of the requester's plan; absent on a request in the old form.
    pub step: Option<ItemId>,
    /// The paths the request lets the worker write; none for a request for facts only.
    pub files: Vec<String>,
    /// The commit the worker's worktree is at now; absent when it could not be read.
    pub head: Option<String>,
    /// Whether the worker committed since the request was given to it.
    pub committed: bool,
    /// The files the worker's commits changed that `files` do not name.
    pub outside: Vec<String>,
    /// What the check found; absent when the request has no check.
    pub found: Option<Found>,
}

/// A check that ran.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Found {
    /// What it found.
    pub outcome: CheckOutcome,
    /// The check as a line.
    pub line: String,
    /// The end of its output.
    pub tail: String,
    /// Why it failed, if it did.
    pub reason: Option<String>,
    /// How long it took, in milliseconds.
    pub duration_ms: u64,
    /// The task-relative file of its output.
    pub output: Option<String>,
}

/// What became of a worker's result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "settled", rename_all = "snake_case")]
pub enum Settlement {
    /// The result is the worker's report, as it always was.
    Report,
    /// The worker's commit is on the requester's branch.
    Brought {
        /// The commit on the requester's branch.
        commit: String,
    },
    /// The commit cannot be written to the requester's branch: the files in conflict, or in the way.
    Conflict {
        /// The files.
        files: Vec<String>,
    },
    /// The result is wrong and the request goes once more to another worker.
    Again {
        /// Why this result did not serve, in the words the requester reads.
        reasons: Vec<String>,
    },
    /// The result is wrong: the reasons, in the words the requester reads.
    Returned {
        /// The reasons.
        reasons: Vec<String>,
    },
}

impl Settlement {
    /// What became of the result, as one word for the daemon's log.
    #[must_use]
    pub const fn outcome(&self) -> &'static str {
        match self {
            Self::Report => "report",
            Self::Brought { .. } => "brought",
            Self::Conflict { .. } => "conflict",
            Self::Again { .. } => "again",
            Self::Returned { .. } => "returned",
        }
    }
}

/// The file of `request` in the task directory `task_dir`.
#[must_use]
pub fn pending_path(task_dir: &Path, request: RequestId) -> PathBuf {
    task_dir.join(format!("{FILE_PREFIX}{}.json", request.number()))
}

/// Writes `pending` to its file, through a temporary file, so that a reader never sees half of it.
///
/// # Errors
/// [`PendingError`].
pub fn write_pending(task_dir: &Path, pending: &Pending) -> Result<(), PendingError> {
    let path = pending_path(task_dir, pending.request);
    let text = serde_json::to_string_pretty(pending).map_err(PendingError::Encode)?;
    let temporary = path.with_extension("json.tmp");
    // The file is small and written from the owner's single task, outside the run engine: a blocking write is the
    // simplest that is also atomic.
    std::fs::write(&temporary, text).map_err(|source| PendingError::Write {
        path: temporary.clone(),
        source,
    })?;
    std::fs::rename(&temporary, &path).map_err(|source| PendingError::Write { path, source })
}

/// The files of the requests kept in the task directory `task_dir`, each read on its own, oldest first by name: a file that
/// cannot be read or is not a request is an error of its own and does not hide the others.
///
/// # Errors
/// [`PendingError::List`] when the directory cannot be listed.
pub async fn read_pending_each(task_dir: &Path) -> Result<Vec<Result<Pending, PendingError>>, PendingError> {
    let mut entries = match tokio::fs::read_dir(task_dir).await {
        Ok(entries) => entries,
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(source) => {
            return Err(PendingError::List {
                path: task_dir.to_owned(),
                source,
            });
        }
    };
    let mut paths = Vec::new();
    loop {
        let entry = match entries.next_entry().await {
            Ok(Some(entry)) => entry,
            Ok(None) => break,
            Err(source) => {
                return Err(PendingError::List {
                    path: task_dir.to_owned(),
                    source,
                });
            }
        };
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        if name.starts_with(FILE_PREFIX) && Path::new(name).extension().is_some_and(|extension| extension == "json") {
            paths.push(entry.path());
        }
    }
    // The number of a request is in its file's name; `10` sorts before `9` as text, so the files are sorted by the number.
    let number = |path: &Path| {
        path.file_stem()
            .and_then(|stem| stem.to_str())
            .and_then(|stem| stem.strip_prefix(FILE_PREFIX))
            .and_then(|digits| digits.parse::<u64>().ok())
            .unwrap_or(u64::MAX)
    };
    paths.sort_by_key(|path| number(path));
    let mut read = Vec::new();
    for path in paths {
        let text = tokio::fs::read_to_string(&path)
            .await
            .map_err(|source| PendingError::Read {
                path: path.clone(),
                source,
            });
        read.push(text.and_then(|text| {
            let pending = serde_json::from_str::<Pending>(&text).map_err(|source| PendingError::Parse {
                path: path.clone(),
                source,
            })?;
            // A file is the request of its name; one that holds another would be settled under the wrong number.
            if request_of_file(&path) == Some(pending.request) {
                Ok(pending)
            } else {
                Err(PendingError::Mismatch {
                    path: path.clone(),
                    found: pending.request.number(),
                })
            }
        }));
    }
    Ok(read)
}

/// Which try of a request a worker's run is, and why the tries before it were wrong; kept in the task directory of the worker
/// of the try from the moment the request is given to it, so that a result settled after a restart is judged as a second try.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Try {
    /// How many workers the request has been given to, this one included: `1` for the first try.
    pub number: u32,
    /// Why each earlier try was wrong, oldest first; one text for each.
    #[serde(default)]
    pub earlier: Vec<String>,
}

impl Try {
    /// The first try, which has no earlier one.
    #[must_use]
    pub const fn first() -> Self {
        Self {
            number: 1,
            earlier: Vec::new(),
        }
    }
}

/// The file that keeps the try of `request` in the task directory `task_dir`.
#[must_use]
pub fn try_path(task_dir: &Path, request: RequestId) -> PathBuf {
    task_dir.join(format!("worker-try-{}.json", request.number()))
}

/// Keeps the try of `request` in `task_dir`, through a temporary file.
///
/// # Errors
/// [`PendingError::Write`] or [`PendingError::Encode`].
pub fn write_try(task_dir: &Path, request: RequestId, tried: &Try) -> Result<(), PendingError> {
    let path = try_path(task_dir, request);
    let text = serde_json::to_string_pretty(tried).map_err(PendingError::Encode)?;
    let temporary = path.with_extension("json.tmp");
    std::fs::write(&temporary, text).map_err(|source| PendingError::Write {
        path: temporary.clone(),
        source,
    })?;
    std::fs::rename(&temporary, &path).map_err(|source| PendingError::Write { path, source })
}

/// The try of `request` kept in `task_dir`; the first try when none is kept.
///
/// # Errors
/// [`PendingError::Read`] or [`PendingError::Parse`] when the file is there and cannot be used.
pub fn read_try(task_dir: &Path, request: RequestId) -> Result<Try, PendingError> {
    let path = try_path(task_dir, request);
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => return Ok(Try::first()),
        Err(source) => return Err(PendingError::Read { path, source }),
    };
    serde_json::from_str(&text).map_err(|source| PendingError::Parse { path, source })
}

/// Whether the task directory `task_dir` holds a file of a request that is not settled yet, one of those in `answered`
/// excepted: a request the daemon has answered is not what keeps a worker busy, even when its file cannot be removed.
#[must_use]
pub fn has_pending_except<S: std::hash::BuildHasher>(
    task_dir: &Path,
    answered: &std::collections::HashSet<RequestId, S>,
) -> bool {
    std::fs::read_dir(task_dir).is_ok_and(|entries| {
        entries.filter_map(Result::ok).any(|entry| {
            let name = entry.file_name();
            let path = entry.path();
            name.to_str().is_some_and(|name| {
                name.starts_with(FILE_PREFIX)
                    && Path::new(name).extension().is_some_and(|extension| extension == "json")
            }) && request_of_file(&path).is_none_or(|request| !answered.contains(&request))
        })
    })
}

/// Whether the task directory `task_dir` holds a file of a request that is not settled yet: a worker with one is busy.
/// A directory that cannot be listed holds none.
#[must_use]
pub fn has_pending(task_dir: &Path) -> bool {
    has_pending_except(task_dir, &std::collections::HashSet::new())
}

#[cfg(test)]
mod tests;
