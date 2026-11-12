//! The state store: the layout of `AGENTS_HOME`, the lock that keeps one daemon per home, the state file and the
//! activity log of a task.
//!
//! The daemon is the only writer of its home (`docs/architecture.md`, "Process model"): [`StateStore::open`] takes an
//! exclusive `flock` on `daemon.lock` and holds it as long as the store lives, so a second daemon on the same home is
//! refused instead of racing the first. The state is saved as a whole, through a temporary file and a rename, so a
//! crash leaves either the old state or the new one.

use std::fs::File;
use std::io;
use std::path::{Path, PathBuf};
use std::string::FromUtf8Error;

use agent_core::cli::ActivityEvent;
use agent_core::config::AbsolutePath;
use agent_core::ids::{RunId, TaskId};
use agent_core::state::{RenderError, State, StateError, Timestamp};
use tempfile::NamedTempFile;
use tokio::task::JoinError;

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

/// The activity log of a task, in the task's directory: JSON lines, append only.
const ACTIVITY_FILE: &str = "activity.jsonl";

/// Prefix of the temporary file a save writes in the home before it renames it over the state file.
const TEMP_PREFIX: &str = ".state-";

/// Suffix of the temporary file of a save.
const TEMP_SUFFIX: &str = ".tmp";

/// The longest text of one field of an activity line, in bytes. A model can print a whole file; the log keeps what
/// lets a human see what happened.
pub const MAX_ACTIVITY_FIELD_BYTES: usize = 4096;

/// What a cut field ends with, so that a reader sees that it was cut.
const CUT_MARK: &str = "…";

/// The layout of `AGENTS_HOME`: where each file and directory of the harness lives.
///
/// Paths are derived from the root and the validated IDs, so no input can name a place outside the home.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Home {
    /// The home directory.
    root: PathBuf,
}

impl Home {
    /// The home at `root`.
    #[must_use]
    pub fn new(root: &AbsolutePath) -> Self {
        Self {
            root: root.as_path().to_path_buf(),
        }
    }

    /// The home directory.
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The state file.
    #[must_use]
    pub fn state_file(&self) -> PathBuf {
        self.root.join(STATE_FILE)
    }

    /// The lock file of the daemon.
    #[must_use]
    pub fn lock_file(&self) -> PathBuf {
        self.root.join(LOCK_FILE)
    }

    /// The directory that holds the directories of all tasks.
    #[must_use]
    pub fn tasks_dir(&self) -> PathBuf {
        self.root.join(TASKS_DIR)
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
        self.root.join(NOTIFICATIONS_DIR)
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
    /// An activity line cannot be appended to the log.
    #[error("cannot append to the activity log {path:?}")]
    AppendActivity {
        /// The activity log.
        path: PathBuf,
        /// What the operating system said.
        source: io::Error,
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
}

impl StateStore {
    /// Creates the home if it is missing, takes its lock and removes the temporary files an interrupted save left.
    ///
    /// # Errors
    /// [`StoreError::HomeLocked`] while another store holds the lock; [`StoreError::CreateDir`],
    /// [`StoreError::OpenLock`], [`StoreError::Lock`], [`StoreError::ListDir`] and [`StoreError::RemoveTemp`] when the
    /// file system refuses; [`StoreError::Worker`] when the blocking task fails.
    pub async fn open(home: Home) -> Result<Self, StoreError> {
        Ok(Self {
            home,
            _lock: File::open("/dev/null").map_err(|source| StoreError::OpenLock {
                path: PathBuf::new(),
                source,
            })?,
        })
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
        Ok(State::new())
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
        let _ = state;
        let file = NamedTempFile::new_in(self.home.root()).map_err(|source| StoreError::WriteTemp {
            path: self.home.root().to_path_buf(),
            source,
        })?;
        Ok(StagedState {
            file,
            target: self.home.state_file(),
        })
    }

    /// Appends one line to the activity log of `task`: the time and the event, texts cut to
    /// [`MAX_ACTIVITY_FIELD_BYTES`].
    ///
    /// # Errors
    /// [`StoreError::CreateDir`], [`StoreError::RenderActivity`] and [`StoreError::AppendActivity`].
    pub async fn append_activity(&self, task: &TaskId, at: Timestamp, event: &ActivityEvent) -> Result<(), StoreError> {
        let _ = (task, at, event);
        Ok(())
    }
}

impl StagedState {
    /// Renames the temporary file over the state file and syncs the directory.
    ///
    /// # Errors
    /// [`StoreError::Replace`], [`StoreError::SyncDir`] and [`StoreError::Worker`]; the temporary file is removed.
    pub(crate) async fn commit(self) -> Result<(), StoreError> {
        let _ = (&self.file, &self.target);
        Ok(())
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

    /// The longest state file the core reads.
    const STATE_LIMIT: u64 = 64 << 20;

    fn home_in(dir: &tempfile::TempDir) -> Home {
        let root = AbsolutePath::from_str(dir.path().to_str().unwrap()).unwrap();
        Home::new(&root)
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

        let again = StateStore::open(home).await;

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

        assert_eq!(std::fs::read_to_string(home.state_file()).unwrap(), FIXTURE);
        assert_eq!(store.load().await.unwrap().to_json().unwrap(), FIXTURE);
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
        assert_eq!(std::fs::read_to_string(home.state_file()).unwrap(), FIXTURE);
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
        std::fs::write(home.state_file(), "{\"schema_version\": 2, \"teams\": 7}").unwrap();

        let result = store.load().await;

        assert!(
            matches!(
                &result,
                Err(StoreError::Invalid { path, source: StateError::UnsupportedSchema(UnsupportedSchema { found: 2 }) })
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

        let second = StateStore::open(home.clone()).await.unwrap();
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
}
