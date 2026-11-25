//! The notes of the waits of the requests in the queue of the pool, in one pass of serving on a bus that is a script.

use std::num::NonZeroU64;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use agent_core::state::Timestamp;
use agent_runtime::agentbus::RequestId;
use agent_runtime::disk::{FakeFreeSpace, SharedFreeSpace};

use super::super::Owner;
use super::super::fixture::{Daemon, pair_state};
use super::super::worker_result::Again;
use crate::fixtures::{task, team_id};

/// How far the clock moves: past the 600 seconds a request waits by default.
const PAST_DEFAULT_LIMIT: i64 = 700;

/// A home whose bus lists `requests` as queued requests of the first task and records what it is asked to set.
struct Rig {
    /// The directory of the bus script and of what it records.
    scripts: tempfile::TempDir,
    /// The daemon's home.
    daemon: Daemon,
    /// The owner of that home.
    owner: Owner,
}

impl Rig {
    /// The rig for the requests of these numbers; each is queued.
    async fn new(numbers: &[u64]) -> Self {
        let scripts = tempfile::tempdir().unwrap();
        let bus = scripts.path().join("bus");
        let list: Vec<_> = numbers
            .iter()
            .map(|number| {
                serde_json::json!({
                    "id": number, "from": "claude-1", "from_task": task(1), "repo": "team-agents",
                    "branch": "main", "status": "queued"
                })
            })
            .collect();
        std::fs::write(
            &bus,
            format!(
                "#!/bin/sh\ncase \"$*\" in\n*helper-list*) printf '%s\\n' '{}' ;;\n*helper-set*) printf '%s\\n' \"$*\" >> \"$(dirname \"$0\")/set.log\" ;;\n*) exit 1 ;;\nesac\n",
                serde_json::Value::Array(list)
            ),
        )
        .unwrap();
        std::fs::set_permissions(&bus, std::fs::Permissions::from_mode(0o755)).unwrap();
        let daemon = Daemon::new(&pair_state(), Some(&bus));
        let config = std::fs::read_to_string(daemon.home.config_file()).unwrap();
        std::fs::write(
            daemon.home.config_file(),
            config.replace(
                "[tiers]",
                "[agents.glm-1]\ncli = \"claude\"\n[tiers]\nworkers = [\"glm-1\"]",
            ),
        )
        .unwrap();
        std::fs::create_dir_all(daemon.home.task_dir(&task(1))).unwrap();
        let (owner, _sinks) = daemon.bare_owner().await;
        Self { scripts, daemon, owner }
    }

    /// The file of the note of request `number` in the directory of the first task.
    fn note(&self, number: u64) -> PathBuf {
        self.daemon
            .home
            .task_dir(&task(1))
            .join(format!("worker-wait-{number}.json"))
    }

    /// Writes a note that says request `number` waits since the time of the fixture's clock at its start.
    fn note_since_start(&self, number: u64) {
        let since = Timestamp::now(&self.daemon.clock).unwrap();
        std::fs::write(
            self.note(number),
            format!(r#"{{"request":"H{number}","since":"{since}"}}"#),
        )
        .unwrap();
    }

    /// What the bus was asked to set, one line per call.
    fn set_calls(&self) -> String {
        read_if_there(&self.scripts.path().join("set.log"))
    }
}

/// The text of the file at `path`, empty when there is none.
fn read_if_there(path: &Path) -> String {
    match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(error) => panic!("{error}"),
    }
}

/// The second try of request `number`, as the daemon holds it after the first try of a worker failed.
fn second_try(number: u64) -> (RequestId, Again) {
    (
        RequestId::new(NonZeroU64::new(number).unwrap()),
        Again {
            team: team_id(0),
            worker: "w-old".parse().unwrap(),
            failed_cli: agent_core::kinds::CliKind::Claude,
            failed_provider: "claude".to_owned(),
            tried: agent_runtime::worker_request::Try {
                number: 2,
                earlier: vec!["first try failed".to_owned()],
            },
            reason: "wrong result".to_owned(),
        },
    )
}

#[tokio::test]
async fn a_request_on_its_second_try_is_returned_when_its_wait_is_over() {
    let mut rig = Rig::new(&[1]).await;
    let (id, again) = second_try(1);
    rig.owner.again.insert(id, again);
    rig.note_since_start(1);
    rig.daemon.clock.advance(time::Duration::seconds(PAST_DEFAULT_LIMIT));

    rig.owner.serve_pool_requests().await.unwrap();

    let calls = rig.set_calls();
    assert!(
        calls.contains("failed") && calls.contains("Nobody took your request"),
        "{calls}"
    );
    assert!(!rig.note(1).exists());
}

#[tokio::test]
async fn every_queued_request_is_noted_while_no_run_may_start() {
    let mut rig = Rig::new(&[1, 2, 3]).await;
    rig.owner.parts.free_space = SharedFreeSpace::new(FakeFreeSpace::new(0));

    rig.owner.serve_pool_requests().await.unwrap();

    assert!(rig.note(1).exists());
    assert!(rig.note(2).exists());
    assert!(rig.note(3).exists());
}

#[tokio::test]
async fn every_queued_request_waiting_too_long_is_returned_while_no_run_may_start() {
    let mut rig = Rig::new(&[1, 2]).await;
    rig.owner.parts.free_space = SharedFreeSpace::new(FakeFreeSpace::new(0));
    rig.note_since_start(1);
    rig.note_since_start(2);
    rig.daemon.clock.advance(time::Duration::seconds(PAST_DEFAULT_LIMIT));

    rig.owner.serve_pool_requests().await.unwrap();

    assert_eq!(rig.set_calls().lines().count(), 2, "{}", rig.set_calls());
}

#[tokio::test]
async fn the_first_pass_removes_the_notes_of_requests_that_are_not_in_the_queue() {
    let mut rig = Rig::new(&[2]).await;
    rig.note_since_start(7);
    rig.note_since_start(2);

    rig.owner.serve_pool_requests().await.unwrap();

    assert!(!rig.note(7).exists(), "the request of that note is not in the queue");
    assert!(rig.note(2).exists(), "the request that waits keeps its note");
}

#[tokio::test]
async fn a_stale_note_that_could_not_be_removed_is_removed_at_a_later_pass() {
    let mut rig = Rig::new(&[2]).await;
    rig.note_since_start(2);
    // A directory under the name of a note: the first removal fails, and the cause is gone when the name is a file again.
    std::fs::create_dir(rig.note(7)).unwrap();

    rig.owner.serve_pool_requests().await.unwrap();
    assert!(rig.note(7).exists(), "the removal fails while the name is a directory");
    std::fs::remove_dir(rig.note(7)).unwrap();
    rig.note_since_start(7);

    rig.owner.serve_pool_requests().await.unwrap();

    assert!(!rig.note(7).exists(), "the note of a request that is not queued is retried");
    assert!(rig.note(2).exists(), "the request that waits keeps its note");
}
