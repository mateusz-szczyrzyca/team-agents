//! The state store: the layout of `AGENTS_HOME`, the lock that keeps one daemon per home, the state file and the
//! activity log of a task.
//!
//! The daemon is the only writer of its home (`docs/architecture.md`, "Process model"): [`StateStore::open`] takes an
//! exclusive `flock` on `daemon.lock` and holds it as long as the store lives, so a second daemon on the same home is
//! refused instead of racing the first. The state is saved as a whole, through a temporary file and a rename, so a
//! crash leaves either the old state or the new one.

use std::borrow::Cow;
use std::fs::File;
use std::io::{self, Write};
use std::num::NonZeroU32;
use std::path::{Path, PathBuf};
use std::string::FromUtf8Error;

use agent_core::cli::ActivityEvent;
use agent_core::config::{AbsolutePath, RepoName};
use agent_core::ids::{RunId, TaskId};
use agent_core::protocol::{Client, ControlAction};
use agent_core::state::{
    ChecksRecord, GateRecord, MAX_STATE_BYTES, PrNumber, RenderError, State, StateError, Timestamp,
};
use tempfile::NamedTempFile;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::task::JoinError;

use crate::text_file::{TextFileError, read_text_file};

/// The state file in the home.
const STATE_FILE: &str = "state.json";

/// The file whose lock says that a daemon owns the home.
const LOCK_FILE: &str = "daemon.lock";

/// The directory of the tasks in the home; one directory per task.
const TASKS_DIR: &str = "tasks";

/// The directory of the runs inside a task's directory; one directory per run.
const RUNS_DIR: &str = "runs";

/// The default directory of the notification files in the home.
const NOTIFICATIONS_DIR: &str = "notifications";

/// The Unix socket the daemon listens on, in the home.
const SOCKET_FILE: &str = "agent.sock";

/// The configuration file in the home.
const CONFIG_FILE: &str = "agents.toml";

/// The file of the secrets in the home.
const SECRETS_FILE: &str = "secrets.toml";

/// The directory of the prompt files in the home, where the installation puts them.
const PROMPTS_DIR: &str = "prompts";

/// The directory of the git worktrees in the home; one directory per repository, inside it one per task.
const WORKTREES_DIR: &str = "worktrees";

/// The activity log of a task, in the task's directory: JSON lines, append only.
const ACTIVITY_FILE: &str = "activity.jsonl";

/// The file in a task's directory that holds the last gate record of the task.
const GATE_RECORD_FILE: &str = "gate.json";

/// The name of the file with the record of the checks before a review, in the directory of the lead's task.
const CHECKS_RECORD_FILE: &str = "checks.json";

/// Prefix of the temporary file a save writes in the home before it renames it over the state file.
const TEMP_PREFIX: &str = ".state-";

/// Suffix of the temporary file of a save.
const TEMP_SUFFIX: &str = ".tmp";

/// The longest text of one field of an activity line, in bytes. A model can print a whole file; the log keeps what
/// lets a human see what happened.
pub const MAX_ACTIVITY_FIELD_BYTES: usize = 4096;

/// What a cut field ends with, so that a reader sees that it was cut.
const CUT_MARK: &str = "…";

/// The name of the environment variable that says where the harness keeps its files: the commands find the daemon's
/// socket through it, and the daemon gives it to the runs it starts so that the commands an agent runs find it too.
pub const HOME_VARIABLE: &str = "AGENTS_HOME";

/// The layout of `AGENTS_HOME`: where each file and directory of the harness lives.
///
/// Paths are derived from the root and the validated IDs, so no input can name a place outside the home.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Home {
    /// The home directory.
    root: AbsolutePath,
}

impl Home {
    /// The home at `root`.
    #[must_use]
    pub fn new(root: &AbsolutePath) -> Self {
        Self { root: root.clone() }
    }

    /// The home directory.
    #[must_use]
    pub fn root(&self) -> &Path {
        self.root.as_path()
    }

    /// The home directory, as the absolute path of UTF-8 text the home was made from.
    #[must_use]
    pub fn absolute_root(&self) -> &AbsolutePath {
        &self.root
    }

    /// The state file.
    #[must_use]
    pub fn state_file(&self) -> PathBuf {
        self.root().join(STATE_FILE)
    }

    /// The lock file of the daemon.
    #[must_use]
    pub fn lock_file(&self) -> PathBuf {
        self.root().join(LOCK_FILE)
    }

    /// The directory that holds the directories of all tasks.
    #[must_use]
    pub fn tasks_dir(&self) -> PathBuf {
        self.root().join(TASKS_DIR)
    }

    /// The directory of `task`: its questions files, its activity log and its runs.
    #[must_use]
    pub fn task_dir(&self, task: &TaskId) -> PathBuf {
        self.tasks_dir().join(task.as_str())
    }

    /// The activity log of `task`.
    #[must_use]
    pub fn activity_log(&self, task: &TaskId) -> PathBuf {
        self.task_dir(task).join(ACTIVITY_FILE)
    }

    /// The directory of one run of `task`: where its output files go.
    #[must_use]
    pub fn run_dir(&self, task: &TaskId, run: &RunId) -> PathBuf {
        self.task_dir(task).join(RUNS_DIR).join(run.as_str())
    }

    /// The default directory of the notification files.
    #[must_use]
    pub fn notifications_dir(&self) -> PathBuf {
        self.root().join(NOTIFICATIONS_DIR)
    }

    /// The Unix socket of the daemon.
    #[must_use]
    pub fn socket_file(&self) -> PathBuf {
        self.root().join(SOCKET_FILE)
    }

    /// The configuration file, `agents.toml`.
    #[must_use]
    pub fn config_file(&self) -> PathBuf {
        self.root().join(CONFIG_FILE)
    }

    /// The file of the secrets, `secrets.toml`.
    #[must_use]
    pub fn secrets_file(&self) -> PathBuf {
        self.root().join(SECRETS_FILE)
    }

    /// The directory the installation keeps the prompt files in.
    #[must_use]
    pub fn prompts_dir(&self) -> PathBuf {
        self.root().join(PROMPTS_DIR)
    }

    /// The directory that holds the worktrees of every repository: `<repo>/<task>` inside it is a task's worktree.
    #[must_use]
    pub fn worktrees_dir(&self) -> PathBuf {
        self.root().join(WORKTREES_DIR)
    }

    /// The git worktree of `task`, a task of a team that works on `repo`.
    #[must_use]
    pub fn worktree_dir(&self, repo: &RepoName, task: &TaskId) -> PathBuf {
        self.worktrees_dir().join(repo.as_str()).join(task.as_str())
    }

    /// The file with the output of the `run`-th gate run of `task`, in the task's directory.
    #[must_use]
    pub fn gate_log(&self, task: &TaskId, run: NonZeroU32) -> PathBuf {
        self.task_dir(task).join(format!("gate-{run}.log"))
    }

    /// The file with the last gate record of `task`, in the task's directory.
    #[must_use]
    pub fn gate_record_file(&self, task: &TaskId) -> PathBuf {
        self.task_dir(task).join(GATE_RECORD_FILE)
    }

    /// The file with the record of the checks before a review that `task`, the lead of a team, keeps, in the task's
    /// directory.
    #[must_use]
    pub fn checks_file(&self, task: &TaskId) -> PathBuf {
        self.task_dir(task).join(CHECKS_RECORD_FILE)
    }

    /// The report of the review of pull request `pr` that `task` gave after `pass` fix passes, in the task's directory.
    #[must_use]
    pub fn review_report_file(&self, task: &TaskId, pr: PrNumber, pass: u32) -> PathBuf {
        self.task_dir(task).join(format!("review-pr{pr}-pass{pass}.md"))
    }

    /// The questions file number `number` of `task`, in the task's directory.
    #[must_use]
    pub fn questions_file(&self, task: &TaskId, number: NonZeroU32) -> PathBuf {
        self.task_dir(task).join(format!("questions-{number}.md"))
    }
}

/// Why a store operation failed.
#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    /// The home directory (or its tasks directory) cannot be created.
    #[error("cannot create the directory {path:?}")]
    CreateDir {
        /// The directory.
        path: PathBuf,
        /// What the operating system said.
        source: io::Error,
    },
    /// The lock file cannot be opened.
    #[error("cannot open the lock file {path:?}")]
    OpenLock {
        /// The lock file.
        path: PathBuf,
        /// What the operating system said.
        source: io::Error,
    },
    /// Another daemon owns the home: it holds the lock.
    #[error("another agent daemon owns this home: {path:?} is locked; stop it first or use another AGENTS_HOME")]
    HomeLocked {
        /// The lock file.
        path: PathBuf,
    },
    /// The lock cannot be taken for a reason other than another owner.
    #[error("cannot lock {path:?}")]
    Lock {
        /// The lock file.
        path: PathBuf,
        /// What the operating system said.
        source: io::Error,
    },
    /// A temporary file of an earlier, interrupted save cannot be removed.
    #[error("cannot remove the leftover temporary file {path:?}")]
    RemoveTemp {
        /// The temporary file.
        path: PathBuf,
        /// What the operating system said.
        source: io::Error,
    },
    /// The home directory cannot be listed to look for leftovers.
    #[error("cannot list the directory {path:?}")]
    ListDir {
        /// The directory.
        path: PathBuf,
        /// What the operating system said.
        source: io::Error,
    },
    /// The state file cannot be read.
    #[error("cannot read the state file {path:?}")]
    Read {
        /// The state file.
        path: PathBuf,
        /// What the operating system said.
        source: io::Error,
    },
    /// The state file is not UTF-8 text.
    #[error("the state file {path:?} is not UTF-8 text")]
    NotUtf8 {
        /// The state file.
        path: PathBuf,
        /// Where the bytes stop being UTF-8.
        source: FromUtf8Error,
    },
    /// The state file is not a state this build reads: too large, another schema version, malformed or inconsistent.
    #[error("the state file {path:?} cannot be used")]
    Invalid {
        /// The state file.
        path: PathBuf,
        /// What is wrong with it; [`StateError::UnsupportedSchema`] for a state of another version.
        source: StateError,
    },
    /// The state cannot be written as JSON.
    #[error("the state cannot be rendered")]
    Render {
        /// What the serializer said.
        source: RenderError,
    },
    /// The temporary file of a save cannot be created, written or synced.
    #[error("cannot write the temporary state file in {path:?}")]
    WriteTemp {
        /// The home directory the file is created in.
        path: PathBuf,
        /// What the operating system said.
        source: io::Error,
    },
    /// The temporary file cannot be renamed over the state file; the old state is untouched.
    #[error("cannot replace the state file {path:?}")]
    Replace {
        /// The state file.
        path: PathBuf,
        /// What the operating system said.
        source: io::Error,
    },
    /// The home directory cannot be synced after the rename.
    #[error("cannot sync the directory {path:?}")]
    SyncDir {
        /// The home directory.
        path: PathBuf,
        /// What the operating system said.
        source: io::Error,
    },
    /// An activity line cannot be written as JSON.
    #[error("an activity line cannot be rendered")]
    RenderActivity {
        /// What the serializer said.
        source: serde_json::Error,
    },
    /// A gate record cannot be written as JSON.
    #[error("a gate record cannot be rendered")]
    RenderGateRecord {
        /// What the serializer said.
        source: serde_json::Error,
    },
    /// An activity line cannot be appended to the log.
    #[error("cannot append to the activity log {path:?}")]
    AppendActivity {
        /// The activity log.
        path: PathBuf,
        /// What the operating system said.
        source: io::Error,
    },
    /// A file of a task's directory cannot be read: the gate record or the report of a review.
    #[error("cannot read the file {path:?} of a task")]
    ReadTaskFile {
        /// The file.
        path: PathBuf,
        /// What is wrong with it.
        source: TextFileError,
    },
    /// The record of the checks of a task cannot be written as JSON.
    #[error("a record of the checks cannot be rendered")]
    RenderChecksRecord {
        /// What the serializer said.
        source: serde_json::Error,
    },
    /// The record of the checks of a task is not one.
    #[error("the record of the checks {path:?} cannot be used")]
    ParseChecksRecord {
        /// The file.
        path: PathBuf,
        /// What the parser said.
        source: serde_json::Error,
    },
    /// The gate record of a task is not a gate record.
    #[error("the gate record {path:?} cannot be used")]
    ParseGateRecord {
        /// The file.
        path: PathBuf,
        /// What the parser said.
        source: serde_json::Error,
    },
    /// The thread that does the blocking file work failed.
    #[error("a file task of the state store failed")]
    Worker {
        /// How the task ended.
        source: JoinError,
    },
}

/// The owner of a home: it holds the lock, reads and saves the state and appends to the activity logs.
///
/// There is one per daemon. Dropping it releases the lock.
#[derive(Debug)]
pub struct StateStore {
    /// The layout of the home.
    home: Home,
    /// The lock file, open: the lock lives as long as this file does.
    _lock: File,
}

/// A state written to its temporary file and not yet renamed over the state file. Dropping it removes the file.
#[derive(Debug)]
pub(crate) struct StagedState {
    /// The temporary file, in the home so that the rename stays on one file system.
    file: NamedTempFile,
    /// The state file it replaces.
    target: PathBuf,
    /// The home directory, synced after the rename so that the rename itself survives a crash.
    dir: PathBuf,
}

/// The body of an activity line: what happened, by kind. Texts are borrowed or, when cut, owned.
#[derive(Debug, serde::Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum ActivityBody<'a> {
    /// A message of the agent.
    Text {
        /// What it said.
        text: Cow<'a, str>,
    },
    /// A shell command of the agent.
    Command {
        /// Its command line.
        command: Cow<'a, str>,
    },
    /// A file the agent changed.
    FileEdit {
        /// The file, as the CLI names it.
        path: Cow<'a, str>,
    },
    /// A call of an agentbus tool.
    BusCall {
        /// The tool.
        tool: Cow<'a, str>,
        /// Its arguments.
        arguments: Cow<'a, str>,
    },
    /// A call of any other tool.
    ToolCall {
        /// The tool.
        name: Cow<'a, str>,
        /// What it was asked to do.
        summary: Cow<'a, str>,
    },
    /// An error of the CLI or of a tool call.
    Error {
        /// What went wrong.
        message: Cow<'a, str>,
    },
    /// Metadata of the run.
    Info {
        /// The metadata.
        message: Cow<'a, str>,
    },
    /// A control of the human.
    Control {
        /// Which control.
        action: ControlAction,
        /// The client it came from.
        by: &'a Client,
    },
}

/// One line of the activity log.
#[derive(Debug, serde::Serialize)]
struct ActivityLine<'a> {
    /// When the event was appended.
    at: Timestamp,
    /// What happened.
    #[serde(flatten)]
    body: ActivityBody<'a>,
}

impl<'a> ActivityLine<'a> {
    /// The line of `event` at `at`, its texts cut to [`MAX_ACTIVITY_FIELD_BYTES`].
    fn new(at: Timestamp, event: &'a ActivityEvent) -> Self {
        let body = match event {
            ActivityEvent::Text(text) => ActivityBody::Text { text: cut(text) },
            ActivityEvent::Command(command) => ActivityBody::Command { command: cut(command) },
            ActivityEvent::FileEdit { path } => ActivityBody::FileEdit { path: cut(path) },
            ActivityEvent::BusCall { tool, arguments } => ActivityBody::BusCall {
                tool: cut(tool),
                arguments: cut(arguments),
            },
            ActivityEvent::ToolCall { name, summary } => ActivityBody::ToolCall {
                name: cut(name),
                summary: cut(summary),
            },
            ActivityEvent::Error(message) => ActivityBody::Error { message: cut(message) },
            ActivityEvent::Info(message) => ActivityBody::Info { message: cut(message) },
            ActivityEvent::Control { action, by } => ActivityBody::Control { action: *action, by },
        };
        Self { at, body }
    }
}

/// `text`, or its first characters that fit in [`MAX_ACTIVITY_FIELD_BYTES`] and [`CUT_MARK`] when it is longer. The cut
/// falls between characters.
fn cut(text: &str) -> Cow<'_, str> {
    if text.len() <= MAX_ACTIVITY_FIELD_BYTES {
        return Cow::Borrowed(text);
    }
    let mut kept: String = text
        .char_indices()
        .take_while(|(index, c)| index + c.len_utf8() <= MAX_ACTIVITY_FIELD_BYTES)
        .map(|(_, c)| c)
        .collect();
    kept.push_str(CUT_MARK);
    Cow::Owned(kept)
}

/// The number of bytes to read of a state file: one more than the limit, so that a file over it is seen as such.
fn state_read_limit() -> u64 {
    // `usize` is never wider than `u64` on a platform Rust supports, so the fallback is not reachable.
    u64::try_from(MAX_STATE_BYTES).map_or(u64::MAX, |max| max.saturating_add(1))
}

/// The most bytes of a file of a task's directory that are read: the gate record and the reports of the reviews are
/// small, and a model or a person can write into the directory.
const MAX_TASK_FILE_BYTES: usize = 1 << 20;

/// The text of the file at `path` in a task's directory; `None` when there is no such file.
async fn read_task_file(path: &Path) -> Result<Option<String>, StoreError> {
    read_text_file(path, MAX_TASK_FILE_BYTES)
        .await
        .map(|found| found.map(|file| file.text))
        .map_err(|source| StoreError::ReadTaskFile {
            path: path.to_path_buf(),
            source,
        })
}

/// The record of the checks of `task` in `home`; `None` when there is none.
///
/// # Errors
/// [`StoreError::ReadTaskFile`] and [`StoreError::ParseChecksRecord`], as for [`StateStore::load_checks_record`].
pub async fn load_checks_record(home: &Home, task: &TaskId) -> Result<Option<ChecksRecord>, StoreError> {
    let path = home.checks_file(task);
    let Some(file) = read_task_file(&path).await? else {
        return Ok(None);
    };
    serde_json::from_str(&file)
        .map(Some)
        .map_err(|source| StoreError::ParseChecksRecord { path, source })
}

/// Runs `job` on a thread that may block and returns its result.
async fn blocking<T: Send + 'static>(
    job: impl FnOnce() -> Result<T, StoreError> + Send + 'static,
) -> Result<T, StoreError> {
    tokio::task::spawn_blocking(job)
        .await
        .map_err(|source| StoreError::Worker { source })?
}

/// Creates `path` and its parents.
fn create_dir(path: &Path) -> Result<(), StoreError> {
    std::fs::create_dir_all(path).map_err(|source| StoreError::CreateDir {
        path: path.to_path_buf(),
        source,
    })
}

/// Takes the lock of the home and removes what an interrupted save left. Returns the open lock file.
fn open_blocking(home: &Home) -> Result<File, StoreError> {
    create_dir(home.root())?;
    create_dir(&home.tasks_dir())?;
    let path = home.lock_file();
    let lock = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&path)
        .map_err(|source| StoreError::OpenLock {
            path: path.clone(),
            source,
        })?;
    match rustix::fs::flock(&lock, rustix::fs::FlockOperation::NonBlockingLockExclusive) {
        Ok(()) => {}
        Err(errno) if errno == rustix::io::Errno::WOULDBLOCK => return Err(StoreError::HomeLocked { path }),
        Err(errno) => {
            return Err(StoreError::Lock {
                path,
                source: errno.into(),
            });
        }
    }
    remove_leftover_temps(home.root())?;
    Ok(lock)
}

/// Removes the temporary files of saves that never reached their rename. Only the holder of the lock calls it, so no
/// save of a live daemon is running.
fn remove_leftover_temps(dir: &Path) -> Result<(), StoreError> {
    let list_error = |source| StoreError::ListDir {
        path: dir.to_path_buf(),
        source,
    };
    for entry in std::fs::read_dir(dir).map_err(list_error)? {
        let entry = entry.map_err(list_error)?;
        // The names of our temporary files are ASCII: a name that is not UTF-8 is someone else's file.
        let ours = entry
            .file_name()
            .to_str()
            .is_some_and(|name| name.starts_with(TEMP_PREFIX) && name.ends_with(TEMP_SUFFIX));
        if ours {
            std::fs::remove_file(entry.path()).map_err(|source| StoreError::RemoveTemp {
                path: entry.path(),
                source,
            })?;
        }
    }
    Ok(())
}

/// Writes `bytes` to a new temporary file in `dir` and syncs it.
fn write_temp(dir: &Path, bytes: &[u8]) -> Result<NamedTempFile, StoreError> {
    let write_error = |source| StoreError::WriteTemp {
        path: dir.to_path_buf(),
        source,
    };
    let mut file = tempfile::Builder::new()
        .prefix(TEMP_PREFIX)
        .suffix(TEMP_SUFFIX)
        .tempfile_in(dir)
        .map_err(write_error)?;
    file.write_all(bytes).map_err(write_error)?;
    file.flush().map_err(write_error)?;
    file.as_file().sync_all().map_err(write_error)?;
    Ok(file)
}

impl StateStore {
    /// Creates the home if it is missing, takes its lock and removes the temporary files an interrupted save left.
    ///
    /// # Errors
    /// [`StoreError::HomeLocked`] while another store holds the lock; [`StoreError::CreateDir`],
    /// [`StoreError::OpenLock`], [`StoreError::Lock`], [`StoreError::ListDir`] and [`StoreError::RemoveTemp`] when the
    /// file system refuses; [`StoreError::Worker`] when the blocking task fails.
    pub async fn open(home: Home) -> Result<Self, StoreError> {
        let layout = home.clone();
        let lock = blocking(move || open_blocking(&layout)).await?;
        Ok(Self { home, _lock: lock })
    }

    /// The layout of the home.
    #[must_use]
    pub fn home(&self) -> &Home {
        &self.home
    }

    /// Reads the state; a home that has none yet has the empty state.
    ///
    /// # Errors
    /// [`StoreError::Read`], [`StoreError::NotUtf8`] and [`StoreError::Invalid`] (which carries
    /// [`StateError::UnsupportedSchema`] for a state of another schema version).
    pub async fn load(&self) -> Result<State, StoreError> {
        let path = self.home.state_file();
        let read_error = |source| StoreError::Read {
            path: path.clone(),
            source,
        };
        let file = match tokio::fs::File::open(&path).await {
            Ok(file) => file,
            // No state file is the specified start of a new home.
            Err(source) if source.kind() == io::ErrorKind::NotFound => return Ok(State::new()),
            Err(source) => return Err(read_error(source)),
        };
        let mut bytes = Vec::new();
        file.take(state_read_limit())
            .read_to_end(&mut bytes)
            .await
            .map_err(read_error)?;
        let text = String::from_utf8(bytes).map_err(|source| StoreError::NotUtf8 {
            path: path.clone(),
            source,
        })?;
        State::from_json(&text).map_err(|source| StoreError::Invalid { path, source })
    }

    /// Saves `state` as a whole: the old state stays whole until the rename, and no temporary file stays behind.
    ///
    /// # Errors
    /// [`StoreError::Render`], [`StoreError::WriteTemp`], [`StoreError::Replace`], [`StoreError::SyncDir`] and
    /// [`StoreError::Worker`]; the old state is intact after any of them.
    pub async fn save(&self, state: &State) -> Result<(), StoreError> {
        self.stage(state).await?.commit().await
    }

    /// Writes `state` to a temporary file in the home, without touching the state file.
    ///
    /// # Errors
    /// [`StoreError::Render`], [`StoreError::WriteTemp`] and [`StoreError::Worker`].
    pub(crate) async fn stage(&self, state: &State) -> Result<StagedState, StoreError> {
        let text = state.to_json().map_err(|source| StoreError::Render { source })?;
        let dir = self.home.root().to_path_buf();
        let target = self.home.state_file();
        let write_dir = dir.clone();
        let file = blocking(move || write_temp(&write_dir, text.as_bytes())).await?;
        Ok(StagedState { file, target, dir })
    }

    /// Saves `record` as the last gate record of `task`, replacing the one before: written to a temporary file in the
    /// task's directory and renamed over [`Home::gate_record_file`], so that a reader sees a whole record or none.
    ///
    /// # Errors
    /// [`StoreError::RenderGateRecord`], [`StoreError::CreateDir`], [`StoreError::WriteTemp`], [`StoreError::Replace`],
    /// [`StoreError::SyncDir`] and [`StoreError::Worker`].
    pub async fn save_gate_record(&self, task: &TaskId, record: &GateRecord) -> Result<(), StoreError> {
        let text = serde_json::to_vec_pretty(record).map_err(|source| StoreError::RenderGateRecord { source })?;
        let dir = self.home.task_dir(task);
        let target = self.home.gate_record_file(task);
        let write_dir = dir.clone();
        let file = blocking(move || {
            create_dir(&write_dir)?;
            write_temp(&write_dir, &text)
        })
        .await?;
        StagedState { file, target, dir }.commit().await
    }

    /// The last gate record of `task`; `None` when the task has never been gated.
    ///
    /// # Errors
    /// [`StoreError::ReadTaskFile`] when the file cannot be read, is too large or is not UTF-8 text;
    /// [`StoreError::ParseGateRecord`] when it is not a gate record.
    pub async fn load_gate_record(&self, task: &TaskId) -> Result<Option<GateRecord>, StoreError> {
        let path = self.home.gate_record_file(task);
        let Some(file) = read_task_file(&path).await? else {
            return Ok(None);
        };
        serde_json::from_str(&file)
            .map(Some)
            .map_err(|source| StoreError::ParseGateRecord { path, source })
    }

    /// Saves `record`, the record of the checks before a review of `task`, in the task's directory, replacing the one
    /// before: written to a temporary file and renamed over [`Home::checks_file`], so that a reader sees a whole record
    /// or none.
    ///
    /// # Errors
    /// [`StoreError::RenderChecksRecord`], [`StoreError::CreateDir`], [`StoreError::WriteTemp`],
    /// [`StoreError::Replace`], [`StoreError::SyncDir`] and [`StoreError::Worker`].
    pub async fn save_checks_record(&self, task: &TaskId, record: &ChecksRecord) -> Result<(), StoreError> {
        let text = serde_json::to_vec_pretty(record).map_err(|source| StoreError::RenderChecksRecord { source })?;
        let dir = self.home.task_dir(task);
        let target = self.home.checks_file(task);
        let write_dir = dir.clone();
        let file = blocking(move || {
            create_dir(&write_dir)?;
            write_temp(&write_dir, &text)
        })
        .await?;
        StagedState { file, target, dir }.commit().await
    }

    /// The record of the checks of `task`; `None` when there is none.
    ///
    /// # Errors
    /// [`StoreError::ReadTaskFile`] when the file cannot be read, is too large or is not UTF-8 text;
    /// [`StoreError::ParseChecksRecord`] when it is not a record of the checks.
    pub async fn load_checks_record(&self, task: &TaskId) -> Result<Option<ChecksRecord>, StoreError> {
        load_checks_record(&self.home, task).await
    }

    /// Saves `report`, the report of a review of pull request `pr` after `pass` fix passes, in the directory of
    /// `task`, replacing the one before: written to a temporary file and renamed, so that a reader sees a whole report
    /// or none.
    ///
    /// # Errors
    /// [`StoreError::CreateDir`], [`StoreError::WriteTemp`], [`StoreError::Replace`], [`StoreError::SyncDir`] and
    /// [`StoreError::Worker`].
    pub async fn save_review_report(
        &self,
        task: &TaskId,
        pr: PrNumber,
        pass: u32,
        report: &str,
    ) -> Result<(), StoreError> {
        let dir = self.home.task_dir(task);
        let target = self.home.review_report_file(task, pr, pass);
        let write_dir = dir.clone();
        let bytes = report.as_bytes().to_vec();
        let file = blocking(move || {
            create_dir(&write_dir)?;
            write_temp(&write_dir, &bytes)
        })
        .await?;
        StagedState { file, target, dir }.commit().await
    }

    /// The report saved for the review of pull request `pr` after `pass` fix passes; `None` when there is none.
    ///
    /// # Errors
    /// [`StoreError::ReadTaskFile`] when the file cannot be read, is too large or is not UTF-8 text.
    pub async fn load_review_report(
        &self,
        task: &TaskId,
        pr: PrNumber,
        pass: u32,
    ) -> Result<Option<String>, StoreError> {
        read_task_file(&self.home.review_report_file(task, pr, pass)).await
    }

    /// Appends one line to the activity log of `task`: the time and the event, texts cut to
    /// [`MAX_ACTIVITY_FIELD_BYTES`].
    ///
    /// The line is written with one `write`, to a file opened for appending; it is not synced, because the log
    /// explains a run to a human and the state does not depend on it.
    ///
    /// # Errors
    /// [`StoreError::CreateDir`], [`StoreError::RenderActivity`] and [`StoreError::AppendActivity`].
    pub async fn append_activity(&self, task: &TaskId, at: Timestamp, event: &ActivityEvent) -> Result<(), StoreError> {
        let mut line = serde_json::to_vec(&ActivityLine::new(at, event))
            .map_err(|source| StoreError::RenderActivity { source })?;
        line.push(b'\n');
        let dir = self.home.task_dir(task);
        let create = dir.clone();
        blocking(move || create_dir(&create)).await?;
        let path = self.home.activity_log(task);
        let append_error = |source| StoreError::AppendActivity {
            path: path.clone(),
            source,
        };
        let mut file = tokio::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .await
            .map_err(append_error)?;
        file.write_all(&line).await.map_err(append_error)?;
        file.flush().await.map_err(append_error)
    }
}

impl StagedState {
    /// Renames the temporary file over the state file and syncs the directory.
    ///
    /// # Errors
    /// [`StoreError::Replace`], [`StoreError::SyncDir`] and [`StoreError::Worker`]; the temporary file is removed when
    /// the rename fails.
    pub(crate) async fn commit(self) -> Result<(), StoreError> {
        let Self { file, target, dir } = self;
        blocking(move || {
            // On failure the error owns the temporary file and removes it when it is dropped.
            file.persist(&target).map_err(|failed| StoreError::Replace {
                path: target.clone(),
                source: failed.error,
            })?;
            File::open(&dir)
                .and_then(|handle| handle.sync_all())
                .map_err(|source| StoreError::SyncDir { path: dir, source })
        })
        .await
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::str::FromStr;

    use agent_core::state::UnsupportedSchema;
    use serde_json::json;

    use super::*;

    /// A state of schema version 1 with two teams: the fixture of the core, which must keep loading.
    const FIXTURE: &str = include_str!("../../agent-core/tests/fixtures/state-v1.json");

    /// What the state of `FIXTURE` is saved as: the same state, migrated to schema version 2.
    const SAVED: &str = include_str!("../../agent-core/tests/fixtures/state-v1-as-v2.json");

    /// The longest state file the core reads.
    const STATE_LIMIT: u64 = 64 << 20;

    fn home_in(dir: &tempfile::TempDir) -> Home {
        let root = AbsolutePath::from_str(dir.path().to_str().unwrap()).unwrap();
        Home::new(&root)
    }

    #[test]
    fn a_home_gives_back_the_absolute_path_it_was_made_from() {
        let root = AbsolutePath::from_str("/srv/agents-ü").unwrap();

        let home = Home::new(&root);

        assert_eq!(home.absolute_root(), &root);
        assert_eq!(home.root(), Path::new("/srv/agents-ü"));
    }

    /// Opens the home again after a store was dropped. Other tests of this binary start processes: a child holds the
    /// open file description of the released lock from its fork to its exec, so a `HomeLocked` that lasts a few
    /// milliseconds is not the lock still being held by the dropped store.
    async fn reopen(home: &Home) -> Result<StateStore, StoreError> {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
        loop {
            match StateStore::open(home.clone()).await {
                Err(StoreError::HomeLocked { .. }) if std::time::Instant::now() < deadline => {
                    tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                }
                other => return other,
            }
        }
    }

    fn task() -> TaskId {
        "task-20261111-193001-k3x9".parse().unwrap()
    }

    fn at() -> Timestamp {
        "2026-11-12T10:00:00Z".parse().unwrap()
    }

    fn fixture_state() -> State {
        State::from_json(FIXTURE).unwrap()
    }

    /// The names in the home directory, without the tasks directory.
    fn names_in(dir: &Path) -> BTreeSet<String> {
        std::fs::read_dir(dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().into_string().unwrap())
            .collect()
    }

    fn activity_lines(home: &Home, task: &TaskId) -> Vec<serde_json::Value> {
        std::fs::read_to_string(home.activity_log(task))
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    }

    // Rule: every path of the layout is derived from the root and the validated IDs.
    #[test]
    fn the_layout_puts_every_file_under_the_root() {
        let root = AbsolutePath::from_str("/srv/agents").unwrap();
        let home = Home::new(&root);
        let run: RunId = "run-20261112-100000-ab12".parse().unwrap();

        assert_eq!(home.root(), Path::new("/srv/agents"));
        assert_eq!(home.state_file(), Path::new("/srv/agents/state.json"));
        assert_eq!(home.lock_file(), Path::new("/srv/agents/daemon.lock"));
        assert_eq!(home.tasks_dir(), Path::new("/srv/agents/tasks"));
        assert_eq!(
            home.task_dir(&task()),
            Path::new("/srv/agents/tasks/task-20261111-193001-k3x9")
        );
        assert_eq!(
            home.activity_log(&task()),
            Path::new("/srv/agents/tasks/task-20261111-193001-k3x9/activity.jsonl")
        );
        assert_eq!(
            home.run_dir(&task(), &run),
            Path::new("/srv/agents/tasks/task-20261111-193001-k3x9/runs/run-20261112-100000-ab12")
        );
        assert_eq!(home.notifications_dir(), Path::new("/srv/agents/notifications"));
    }

    // Rule: opening creates the home, also when its parents are missing.
    #[tokio::test]
    async fn open_creates_a_missing_home() {
        let dir = tempfile::tempdir().unwrap();
        let root = AbsolutePath::from_str(dir.path().join("a/b/home").to_str().unwrap()).unwrap();
        let home = Home::new(&root);

        let store = StateStore::open(home.clone()).await.unwrap();

        assert!(home.root().is_dir());
        assert!(home.lock_file().is_file());
        assert_eq!(store.home(), &home);
    }

    // Rule: one daemon per home — a second lock of the same home fails while the first is held.
    #[tokio::test]
    async fn a_second_open_of_the_same_home_is_refused_while_the_first_is_held() {
        let dir = tempfile::tempdir().unwrap();
        let home = home_in(&dir);
        let first = StateStore::open(home.clone()).await.unwrap();

        let second = StateStore::open(home.clone()).await;

        assert!(
            matches!(&second, Err(StoreError::HomeLocked { path }) if *path == home.lock_file()),
            "{second:?}"
        );
        drop(first);
    }

    // Rule: the lock is released with the store, so a restarted daemon can take the home.
    #[tokio::test]
    async fn the_home_can_be_opened_again_after_the_store_is_dropped() {
        let dir = tempfile::tempdir().unwrap();
        let home = home_in(&dir);
        drop(StateStore::open(home.clone()).await.unwrap());

        let again = reopen(&home).await;

        assert!(again.is_ok(), "{again:?}");
    }

    // Rule: a home without a state file has the empty state.
    #[tokio::test]
    async fn load_gives_the_empty_state_before_the_first_save() {
        let dir = tempfile::tempdir().unwrap();
        let store = StateStore::open(home_in(&dir)).await.unwrap();

        let state = store.load().await.unwrap();

        assert_eq!(state.to_json().unwrap(), State::new().to_json().unwrap());
    }

    // Rule: what is saved is what is loaded, and the file is the core's text of the state.
    #[tokio::test]
    async fn a_saved_state_loads_back_and_is_written_as_the_core_renders_it() {
        let dir = tempfile::tempdir().unwrap();
        let home = home_in(&dir);
        let store = StateStore::open(home.clone()).await.unwrap();

        store.save(&fixture_state()).await.unwrap();

        assert_eq!(std::fs::read_to_string(home.state_file()).unwrap(), SAVED);
        assert_eq!(store.load().await.unwrap().to_json().unwrap(), SAVED);
    }

    // Rule: a save leaves no temporary file and replaces the old state whole.
    #[tokio::test]
    async fn a_second_save_replaces_the_first_and_leaves_no_temporary_file() {
        let dir = tempfile::tempdir().unwrap();
        let home = home_in(&dir);
        let store = StateStore::open(home.clone()).await.unwrap();
        store.save(&fixture_state()).await.unwrap();

        store.save(&State::new()).await.unwrap();

        assert_eq!(
            std::fs::read_to_string(home.state_file()).unwrap(),
            State::new().to_json().unwrap()
        );
        let expected: BTreeSet<String> = ["state.json", "daemon.lock", "tasks"].map(String::from).into();
        assert_eq!(names_in(home.root()), expected);
    }

    // Rule: a save interrupted before its rename leaves the old state whole and no temporary file in the home.
    #[tokio::test]
    async fn a_save_interrupted_before_the_rename_leaves_the_old_state_and_no_temporary_file() {
        let dir = tempfile::tempdir().unwrap();
        let home = home_in(&dir);
        let store = StateStore::open(home.clone()).await.unwrap();
        store.save(&fixture_state()).await.unwrap();

        let staged = store.stage(&State::new()).await.unwrap();
        let temporary = staged.file.path().to_path_buf();
        assert!(temporary.is_file(), "the new state is written before the rename");
        drop(staged);

        assert!(!temporary.exists());
        assert_eq!(std::fs::read_to_string(home.state_file()).unwrap(), SAVED);
        let expected: BTreeSet<String> = ["state.json", "daemon.lock", "tasks"].map(String::from).into();
        assert_eq!(names_in(home.root()), expected);
    }

    // Rule: a process killed between writing and renaming leaves a temporary file; the next daemon removes it.
    #[tokio::test]
    async fn open_removes_the_temporary_file_of_a_crashed_save_and_keeps_other_files() {
        let dir = tempfile::tempdir().unwrap();
        let home = home_in(&dir);
        std::fs::write(home.state_file(), FIXTURE).unwrap();
        std::fs::write(home.root().join(".state-Ab12Cd.tmp"), "{\"half").unwrap();
        std::fs::write(home.root().join("notes.tmp"), "mine").unwrap();

        let _store = StateStore::open(home.clone()).await.unwrap();

        let expected: BTreeSet<String> = ["state.json", "daemon.lock", "tasks", "notes.tmp"]
            .map(String::from)
            .into();
        assert_eq!(names_in(home.root()), expected);
    }

    // Rule: a failed rename is reported with the state file's path and leaves nothing behind.
    #[tokio::test]
    async fn a_failed_rename_is_a_replace_error_and_leaves_no_temporary_file() {
        let dir = tempfile::tempdir().unwrap();
        let home = home_in(&dir);
        let store = StateStore::open(home.clone()).await.unwrap();
        std::fs::create_dir(home.state_file()).unwrap();

        let result = store.save(&State::new()).await;

        assert!(
            matches!(&result, Err(StoreError::Replace { path, .. }) if *path == home.state_file()),
            "{result:?}"
        );
        let expected: BTreeSet<String> = ["state.json", "daemon.lock", "tasks"].map(String::from).into();
        assert_eq!(names_in(home.root()), expected);
    }

    // Rule: a state of another schema version is refused as such, never as malformed.
    #[tokio::test]
    async fn load_refuses_a_schema_version_it_does_not_read() {
        let dir = tempfile::tempdir().unwrap();
        let home = home_in(&dir);
        let store = StateStore::open(home.clone()).await.unwrap();
        std::fs::write(home.state_file(), "{\"schema_version\": 3, \"teams\": 7}").unwrap();

        let result = store.load().await;

        assert!(
            matches!(
                &result,
                Err(StoreError::Invalid { path, source: StateError::UnsupportedSchema(UnsupportedSchema { found: 3 }) })
                    if *path == home.state_file()
            ),
            "{result:?}"
        );
    }

    // Rule: a file that is not a state is an error with its path, not an empty state.
    #[tokio::test]
    async fn load_refuses_a_malformed_state() {
        let dir = tempfile::tempdir().unwrap();
        let home = home_in(&dir);
        let store = StateStore::open(home.clone()).await.unwrap();
        std::fs::write(home.state_file(), "{\"schema_version\": 1").unwrap();

        let result = store.load().await;

        assert!(
            matches!(
                &result,
                Err(StoreError::Invalid {
                    source: StateError::Malformed { .. },
                    ..
                })
            ),
            "{result:?}"
        );
    }

    // Rule: a file that is not UTF-8 is its own error.
    #[tokio::test]
    async fn load_refuses_a_state_that_is_not_utf8() {
        let dir = tempfile::tempdir().unwrap();
        let home = home_in(&dir);
        let store = StateStore::open(home.clone()).await.unwrap();
        std::fs::write(home.state_file(), [b'{', 0xff, 0xfe]).unwrap();

        let result = store.load().await;

        assert!(matches!(&result, Err(StoreError::NotUtf8 { .. })), "{result:?}");
    }

    // Rule: a state over the limit is refused without reading it all.
    #[tokio::test]
    async fn load_refuses_a_state_over_the_limit() {
        let dir = tempfile::tempdir().unwrap();
        let home = home_in(&dir);
        let store = StateStore::open(home.clone()).await.unwrap();
        let file = File::create(home.state_file()).unwrap();
        file.set_len(STATE_LIMIT + 1024).unwrap();

        let result = store.load().await;

        assert!(
            matches!(
                &result,
                Err(StoreError::Invalid {
                    source: StateError::TooLarge { .. },
                    ..
                })
            ),
            "{result:?}"
        );
    }

    // Rule: a state file that cannot be read is a read error with its path.
    #[tokio::test]
    async fn load_reports_a_state_file_that_cannot_be_read() {
        let dir = tempfile::tempdir().unwrap();
        let home = home_in(&dir);
        let store = StateStore::open(home.clone()).await.unwrap();
        std::fs::create_dir(home.state_file()).unwrap();

        let result = store.load().await;

        assert!(
            matches!(&result, Err(StoreError::Read { path, .. }) if *path == home.state_file()),
            "{result:?}"
        );
    }

    // Rule: a home that cannot be created is reported with the directory.
    #[tokio::test]
    async fn open_reports_a_home_that_cannot_be_created() {
        let dir = tempfile::tempdir().unwrap();
        let blocker = dir.path().join("file");
        std::fs::write(&blocker, "x").unwrap();
        let root = AbsolutePath::from_str(blocker.join("home").to_str().unwrap()).unwrap();

        let result = StateStore::open(Home::new(&root)).await;

        assert!(
            matches!(&result, Err(StoreError::CreateDir { path, .. }) if *path == blocker.join("home")),
            "{result:?}"
        );
    }

    // Rule: each event is one JSON line with the time and its kind, in the order appended.
    #[tokio::test]
    async fn append_activity_writes_one_json_line_per_event_in_order() {
        let dir = tempfile::tempdir().unwrap();
        let home = home_in(&dir);
        let store = StateStore::open(home.clone()).await.unwrap();
        let events = [
            ActivityEvent::Text("hello żółć".to_owned()),
            ActivityEvent::Command("cargo test".to_owned()),
            ActivityEvent::FileEdit {
                path: "src/lib.rs".to_owned(),
            },
            ActivityEvent::BusCall {
                tool: "notebook_append".to_owned(),
                arguments: "{}".to_owned(),
            },
            ActivityEvent::ToolCall {
                name: "Read".to_owned(),
                summary: "a file".to_owned(),
            },
            ActivityEvent::Error("boom".to_owned()),
            ActivityEvent::Info("model x".to_owned()),
        ];

        for event in &events {
            store.append_activity(&task(), at(), event).await.unwrap();
        }

        let t = "2026-11-12T10:00:00Z";
        assert_eq!(
            activity_lines(&home, &task()),
            vec![
                json!({"at": t, "kind": "text", "text": "hello żółć"}),
                json!({"at": t, "kind": "command", "command": "cargo test"}),
                json!({"at": t, "kind": "file_edit", "path": "src/lib.rs"}),
                json!({"at": t, "kind": "bus_call", "tool": "notebook_append", "arguments": "{}"}),
                json!({"at": t, "kind": "tool_call", "name": "Read", "summary": "a file"}),
                json!({"at": t, "kind": "error", "message": "boom"}),
                json!({"at": t, "kind": "info", "message": "model x"}),
            ]
        );
    }

    // Rule: the log is append only: a later daemon adds to what an earlier one wrote, and tasks do not mix.
    #[tokio::test]
    async fn the_activity_log_grows_across_stores_and_is_kept_per_task() {
        let dir = tempfile::tempdir().unwrap();
        let home = home_in(&dir);
        let other: TaskId = "task-20261111-193002-k3x9".parse().unwrap();
        let first = StateStore::open(home.clone()).await.unwrap();
        first
            .append_activity(&task(), at(), &ActivityEvent::Text("one".to_owned()))
            .await
            .unwrap();
        first
            .append_activity(&other, at(), &ActivityEvent::Text("elsewhere".to_owned()))
            .await
            .unwrap();
        drop(first);

        let second = reopen(&home).await.unwrap();
        second
            .append_activity(&task(), at(), &ActivityEvent::Text("two".to_owned()))
            .await
            .unwrap();

        let texts: Vec<_> = activity_lines(&home, &task())
            .iter()
            .map(|line| line["text"].as_str().unwrap().to_owned())
            .collect();
        assert_eq!(texts, ["one", "two"]);
        assert_eq!(activity_lines(&home, &other).len(), 1);
    }

    // Rule: a long text is cut at the limit, on a character boundary, and says so.
    #[tokio::test]
    async fn append_activity_cuts_a_long_text_on_a_character_boundary() {
        let dir = tempfile::tempdir().unwrap();
        let home = home_in(&dir);
        let store = StateStore::open(home.clone()).await.unwrap();
        // 'ż' has two bytes and the text starts with one: the limit falls inside the 2048th 'ż'.
        let long = format!("a{}", "ż".repeat(MAX_ACTIVITY_FIELD_BYTES));

        store
            .append_activity(&task(), at(), &ActivityEvent::Text(long))
            .await
            .unwrap();

        let lines = activity_lines(&home, &task());
        let kept = lines[0]["text"].as_str().unwrap();
        assert_eq!(
            kept,
            format!("a{}{CUT_MARK}", "ż".repeat(MAX_ACTIVITY_FIELD_BYTES / 2 - 1))
        );
    }

    // Rule: a text at the limit is kept whole.
    #[tokio::test]
    async fn append_activity_keeps_a_text_at_the_limit_whole() {
        let dir = tempfile::tempdir().unwrap();
        let home = home_in(&dir);
        let store = StateStore::open(home.clone()).await.unwrap();
        let exact = "a".repeat(MAX_ACTIVITY_FIELD_BYTES);

        store
            .append_activity(&task(), at(), &ActivityEvent::Text(exact.clone()))
            .await
            .unwrap();

        assert_eq!(activity_lines(&home, &task())[0]["text"], exact);
    }

    // Rule: a log that cannot be appended to is an error with its path.
    #[tokio::test]
    async fn append_activity_reports_a_log_that_cannot_be_opened() {
        let dir = tempfile::tempdir().unwrap();
        let home = home_in(&dir);
        let store = StateStore::open(home.clone()).await.unwrap();
        std::fs::create_dir_all(home.activity_log(&task())).unwrap();

        let result = store
            .append_activity(&task(), at(), &ActivityEvent::Text("x".to_owned()))
            .await;

        assert!(
            matches!(&result, Err(StoreError::AppendActivity { path, .. }) if *path == home.activity_log(&task())),
            "{result:?}"
        );
    }

    // Rule: a lock file that cannot be opened is an error with its path.
    #[tokio::test]
    async fn open_reports_a_lock_file_that_cannot_be_opened() {
        let dir = tempfile::tempdir().unwrap();
        let home = home_in(&dir);
        std::fs::create_dir_all(home.lock_file()).unwrap();

        let result = StateStore::open(home.clone()).await;

        assert!(
            matches!(&result, Err(StoreError::OpenLock { path, .. }) if *path == home.lock_file()),
            "{result:?}"
        );
    }

    // Rule: a leftover temporary file that cannot be removed stops the daemon with its path: it would be kept
    // for ever otherwise.
    #[tokio::test]
    async fn open_reports_a_leftover_it_cannot_remove() {
        let dir = tempfile::tempdir().unwrap();
        let home = home_in(&dir);
        let leftover = home.root().join(".state-stuck.tmp");
        std::fs::create_dir_all(leftover.join("inside")).unwrap();

        let result = StateStore::open(home).await;

        assert!(
            matches!(&result, Err(StoreError::RemoveTemp { path, .. }) if *path == leftover),
            "{result:?}"
        );
    }

    // Rule: a save into a home that has vanished fails at the temporary file, and says where.
    #[tokio::test]
    async fn save_reports_a_home_that_vanished() {
        let dir = tempfile::tempdir().unwrap();
        let home = home_in(&dir);
        let store = StateStore::open(home.clone()).await.unwrap();
        std::fs::remove_dir_all(home.root()).unwrap();

        let result = store.save(&State::new()).await;

        assert!(
            matches!(&result, Err(StoreError::WriteTemp { path, .. }) if path == home.root()),
            "{result:?}"
        );
    }
}
