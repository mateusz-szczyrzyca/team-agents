//! The three checks of item 4.1, run through the public API only — the way the daemon of later items uses the
//! crate: a second lock of a home is refused, a stopped process leaves none of its group behind, and a saved state
//! leaves nothing but the home's own files.

#[cfg(test)]
mod runtime_services {
    use std::collections::{BTreeMap, BTreeSet};
    use std::num::NonZeroUsize;
    use std::path::Path;
    use std::str::FromStr;
    use std::time::{Duration, Instant};

    use agent_core::config::AbsolutePath;
    use agent_core::state::State;
    use agent_runtime::process::{ProcessSpec, StopHow, Supervisor};
    use agent_runtime::store::{Home, StateStore, StoreError};

    /// The longest any wait of these tests lasts.
    const PATIENCE: Duration = Duration::from_secs(10);

    fn abs(path: &Path) -> AbsolutePath {
        AbsolutePath::from_str(path.to_str().unwrap()).unwrap()
    }

    fn home_in(dir: &tempfile::TempDir) -> Home {
        Home::new(&abs(dir.path()))
    }

    /// Calls `check` until it returns true, for at most [`PATIENCE`].
    async fn eventually(what: &str, mut check: impl FnMut() -> bool) {
        let deadline = Instant::now() + PATIENCE;
        while !check() {
            assert!(Instant::now() < deadline, "timed out waiting for {what}");
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    /// Whether the process is gone: not in the process table, or a zombie that only waits to be reaped.
    fn is_gone(pid: i32) -> bool {
        let Ok(text) = std::fs::read_to_string(format!("/proc/{pid}/stat")) else {
            return true;
        };
        // The name is in parentheses and may hold spaces: the state letter follows the last parenthesis.
        text.rsplit_once(") ")
            .and_then(|(_, rest)| rest.chars().next())
            .is_none_or(|state| matches!(state, 'Z' | 'X'))
    }

    // Check 1: a second lock of the same home fails with the typed error while the first is held.
    #[tokio::test]
    async fn a_second_lock_of_the_home_fails_with_the_typed_error_while_the_first_is_held() {
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

    // Check 2: a supervised process that started children leaves none of its group behind after a stop.
    #[tokio::test]
    async fn a_stopped_process_leaves_none_of_its_group_behind() {
        let dir = tempfile::tempdir().unwrap();
        let script = "sleep 300 & echo $! >> pids; (trap '' TERM; exec sleep 300) & echo $! >> pids; \
                      echo ready; trap '' TERM; while :; do sleep 1; done";
        let spec = ProcessSpec {
            program: "sh".parse().unwrap(),
            args: vec!["-c".to_owned(), script.to_owned()],
            env: BTreeMap::new(),
            working_dir: abs(dir.path()),
            stdout_file: dir.path().join("out"),
            stderr_file: dir.path().join("err"),
        };
        let process = Supervisor::new(NonZeroUsize::new(1).unwrap())
            .spawn(&spec)
            .await
            .unwrap();
        eventually("the children to start", || {
            std::fs::read_to_string(dir.path().join("out")).is_ok_and(|text| text.lines().any(|line| line == "ready"))
        })
        .await;
        let children: Vec<i32> = std::fs::read_to_string(dir.path().join("pids"))
            .unwrap()
            .lines()
            .map(|line| line.parse().unwrap())
            .collect();
        assert_eq!(children.len(), 2);
        assert!(
            children.iter().all(|pid| !is_gone(*pid)),
            "the children run before the stop"
        );

        let report = process.stop(Duration::from_millis(200)).await.unwrap();

        assert_eq!(report.how, StopHow::Killed, "the leader ignores SIGTERM");
        eventually("the group's processes to end", || {
            children.iter().all(|pid| is_gone(*pid))
        })
        .await;
    }

    // Check 3: a save leaves the state whole and no temporary file in the home.
    #[tokio::test]
    async fn a_save_leaves_the_state_whole_and_no_temporary_file_in_the_home() {
        let dir = tempfile::tempdir().unwrap();
        let home = home_in(&dir);
        let store = StateStore::open(home.clone()).await.unwrap();
        let state = State::new();

        store.save(&state).await.unwrap();
        store.save(&state).await.unwrap();

        let loaded = store.load().await.unwrap();
        assert_eq!(loaded.to_json().unwrap(), state.to_json().unwrap());
        let names: BTreeSet<String> = std::fs::read_dir(home.root())
            .unwrap()
            .map(|entry| entry.unwrap().file_name().into_string().unwrap())
            .collect();
        let expected: BTreeSet<String> = ["state.json", "daemon.lock", "tasks"].map(String::from).into();
        assert_eq!(names, expected);
    }
}
