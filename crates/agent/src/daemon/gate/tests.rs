//! The gate service against real `sh` gate commands and real git worktrees in a temporary home: the order of the
//! queue, the timeout, the records and what is told when a gate ends.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::str::FromStr;
use std::time::{Duration, Instant};

use agent_core::protocol::EventKind;
use agent_core::state::{GateOutcome, GateRecord, GateTree};
use agent_runtime::run::WatchdogHold;
use agent_runtime::store::Home;
use tokio::task::JoinHandle;

use super::*;
use crate::fixtures::{repo, task, team_id};
use crate::testing::ManualClock;

/// The longest any wait of these tests lasts.
const PATIENCE: Duration = Duration::from_secs(30);

/// The gate's timeout when a test does not care: far longer than any gate of these tests.
const LONG: Duration = Duration::from_secs(60);

fn abs(path: &Path) -> AbsolutePath {
    AbsolutePath::from_str(path.to_str().unwrap()).unwrap()
}

/// Runs git for the setup of a test, apart from the configuration of the machine.
fn git(dir: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .args(["-c", "commit.gpgsign=false"])
        .args(args)
        .current_dir(dir)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_AUTHOR_NAME", "Fixture Author")
        .env("GIT_AUTHOR_EMAIL", "fixture@example.com")
        .env("GIT_COMMITTER_NAME", "Fixture Author")
        .env("GIT_COMMITTER_EMAIL", "fixture@example.com")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap().trim().to_owned()
}

/// A gate command that runs `script` with `sh`.
fn sh(script: &str) -> CommandLine {
    CommandLine::try_from(vec!["sh".to_owned(), "-c".to_owned(), script.to_owned()]).unwrap()
}

/// Whether the process is gone: not in the process table, or a zombie that only waits to be reaped.
fn is_gone(pid: i32) -> bool {
    let Ok(text) = std::fs::read_to_string(format!("/proc/{pid}/stat")) else {
        return true;
    };
    text.rsplit_once(") ")
        .and_then(|(_, rest)| rest.chars().next())
        .is_none_or(|state| matches!(state, 'Z' | 'X'))
}

async fn eventually(what: &str, mut check: impl FnMut() -> bool) {
    let deadline = Instant::now() + PATIENCE;
    while !check() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

/// The answer to a gate, as the client gets it.
type Answer = oneshot::Receiver<Result<Reply, Failure>>;

/// A home, a service that is started when [`Rig::start`] is called, and the worktrees of the tasks it is asked about.
struct Rig {
    _dir: tempfile::TempDir,
    home: Home,
    store: Arc<StateStore>,
    queue: GateQueue,
    waiting: Option<mpsc::Receiver<GateJob>>,
    events: broadcast::Receiver<Event>,
    sender: broadcast::Sender<Event>,
    shutdown: CancellationToken,
    service: Option<JoinHandle<()>>,
}

impl Rig {
    async fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let home = Home::new(&abs(&dir.path().canonicalize().unwrap()));
        let store = Arc::new(StateStore::open(home.clone()).await.unwrap());
        let (queue, waiting) = GateQueue::new();
        let (sender, events) = broadcast::channel(16);
        Self {
            _dir: dir,
            home,
            store,
            queue,
            waiting: Some(waiting),
            events,
            sender,
            shutdown: CancellationToken::new(),
            service: None,
        }
    }

    /// Starts the service.
    fn start(&mut self) {
        let parts = GateParts {
            store: Arc::clone(&self.store),
            clock: SharedClock::new(ManualClock::new(crate::fixtures::at(0))),
            events: self.sender.clone(),
            agent_path: vec![abs(Path::new("/usr/bin")), abs(Path::new("/bin"))],
            stop_grace: Duration::from_secs(5),
        };
        let waiting = self.waiting.take().unwrap();
        self.service = Some(tokio::spawn(run(waiting, parts, self.shutdown.clone())));
    }

    /// The worktree of task `n`: a git repository on `main` with one commit.
    fn worktree(&self, n: u32) -> PathBuf {
        let dir = self.home.worktree_dir(&repo(), &task(n));
        std::fs::create_dir_all(&dir).unwrap();
        git(&dir, &["init", "-q", "-b", "main"]);
        std::fs::write(dir.join("README.md"), format!("task {n}\n")).unwrap();
        git(&dir, &["add", "README.md"]);
        git(&dir, &["commit", "-q", "-m", "First"]);
        dir
    }

    /// A job of task `n` that runs `script`, and the client's end of its answer.
    fn job(&self, n: u32, script: &str, timeout: Duration) -> (GateJob, Answer) {
        let (reply, answer) = oneshot::channel();
        let job = GateJob {
            task: task(n),
            team: team_id(1),
            worktree: abs(&self.home.worktree_dir(&repo(), &task(n))),
            command: sh(script),
            timeout,
            cancel: self.shutdown.child_token(),
            hold: None,
            reply,
        };
        (job, answer)
    }

    /// Asks for a gate of task `n` that runs `script`.
    fn ask(&self, n: u32, script: &str, timeout: Duration) -> Answer {
        let (job, answer) = self.job(n, script, timeout);
        self.queue.enqueue(job).unwrap();
        answer
    }

    fn record_file(&self, n: u32) -> PathBuf {
        self.home.gate_record_file(&task(n))
    }

    fn saved_record(&self, n: u32) -> GateRecord {
        serde_json::from_slice(&std::fs::read(self.record_file(n)).unwrap()).unwrap()
    }
}

impl Drop for Rig {
    fn drop(&mut self) {
        self.shutdown.cancel();
    }
}

/// Waits for the answer and returns the record it carries.
async fn recorded(answer: Answer) -> GateRecord {
    match tokio::time::timeout(PATIENCE, answer).await.unwrap().unwrap() {
        Ok(Reply::GateRecorded { record }) => record,
        other => panic!("a record was expected: {other:?}"),
    }
}

/// Waits for the answer and returns the failure it is.
async fn refused(answer: Answer) -> Failure {
    match tokio::time::timeout(PATIENCE, answer).await.unwrap().unwrap() {
        Err(failure) => failure,
        other => panic!("a failure was expected: {other:?}"),
    }
}

#[tokio::test]
async fn two_gates_waiting_run_one_after_the_other_in_the_order_they_were_asked() {
    let mut rig = Rig::new().await;
    let (first_tree, second_tree) = (rig.worktree(1), rig.worktree(2));
    let order = rig.home.root().join("order");
    let script = format!(
        "echo \"start $(basename \"$PWD\")\" >> {order:?}; sleep 0.2; echo \"end $(basename \"$PWD\")\" >> {order:?}"
    );
    // Both are waiting before the service takes the first.
    let first = rig.ask(1, &script, LONG);
    let second = rig.ask(2, &script, LONG);
    rig.start();

    let (first, second) = (recorded(first).await, recorded(second).await);

    assert_eq!(
        std::fs::read_to_string(&order).unwrap(),
        format!(
            "start {t1}\nend {t1}\nstart {t2}\nend {t2}\n",
            t1 = task(1),
            t2 = task(2)
        )
    );
    assert_eq!(first.result.head.as_str(), git(&first_tree, &["rev-parse", "HEAD"]));
    assert_eq!(second.result.head.as_str(), git(&second_tree, &["rev-parse", "HEAD"]));
    assert_ne!(first.result.head, second.result.head);
}

#[tokio::test]
async fn a_gate_that_passes_on_a_clean_tree_is_recorded_with_its_commit_and_its_output() {
    let mut rig = Rig::new().await;
    let tree = rig.worktree(1);
    rig.start();

    let record = recorded(rig.ask(1, "echo all tests passed", LONG)).await;

    assert_eq!(record.tree, GateTree::Clean);
    assert_eq!(record.result.outcome, GateOutcome::Passed);
    assert_eq!(record.result.head.as_str(), git(&tree, &["rev-parse", "HEAD"]));
    assert!(record.counts_as_pass());
    let log = record.result.log.as_path();
    assert_eq!(
        log,
        rig.home.gate_log(&task(1), std::num::NonZeroU32::MIN),
        "the first gate of the task is the first log"
    );
    assert_eq!(std::fs::read_to_string(log).unwrap(), "all tests passed\n");
    assert_eq!(
        rig.saved_record(1),
        record,
        "the record is kept in the task's directory"
    );
}

#[tokio::test]
async fn a_gate_that_fails_is_recorded_with_its_exit_code_and_does_not_count_as_a_pass() {
    let mut rig = Rig::new().await;
    rig.worktree(1);
    rig.start();

    let record = recorded(rig.ask(1, "echo broken >&2; exit 3", LONG)).await;

    assert_eq!(
        record.result.outcome,
        GateOutcome::Failed {
            exit_code: std::num::NonZeroI32::new(3).unwrap()
        }
    );
    assert_eq!(record.tree, GateTree::Clean);
    assert!(!record.counts_as_pass());
    assert_eq!(
        std::fs::read_to_string(record.result.log.as_path()).unwrap(),
        "broken\n",
        "the standard error is in the log too"
    );
}

#[tokio::test]
async fn a_gate_on_a_merge_in_progress_is_run_and_recorded_as_not_clean() {
    let mut rig = Rig::new().await;
    let tree = rig.worktree(1);
    git(&tree, &["switch", "-q", "-c", "side"]);
    std::fs::write(tree.join("side.txt"), "side\n").unwrap();
    git(&tree, &["add", "side.txt"]);
    git(&tree, &["commit", "-q", "-m", "Side"]);
    git(&tree, &["switch", "-q", "main"]);
    git(&tree, &["merge", "-q", "--no-commit", "--no-ff", "side"]);
    rig.start();

    let record = recorded(rig.ask(1, "test -f side.txt", LONG)).await;

    assert_eq!(record.tree, GateTree::NotClean);
    assert_eq!(
        record.result.outcome,
        GateOutcome::Passed,
        "the gate ran on the merged tree"
    );
    assert_eq!(record.result.head.as_str(), git(&tree, &["rev-parse", "HEAD"]));
    assert!(
        !record.counts_as_pass(),
        "a pass on a merge that is not committed is not a pass of HEAD"
    );
}

#[tokio::test]
async fn a_gate_on_a_tree_with_an_untracked_or_a_changed_file_is_recorded_as_not_clean() {
    let mut rig = Rig::new().await;
    let untracked = rig.worktree(1);
    std::fs::write(untracked.join("new.txt"), "new\n").unwrap();
    let changed = rig.worktree(2);
    std::fs::write(changed.join("README.md"), "changed\n").unwrap();
    rig.start();

    let first = recorded(rig.ask(1, "true", LONG)).await;
    let second = recorded(rig.ask(2, "true", LONG)).await;

    assert_eq!(first.tree, GateTree::NotClean);
    assert_eq!(second.tree, GateTree::NotClean);
}

#[tokio::test]
async fn a_gate_over_its_timeout_is_stopped_with_what_it_started_and_recorded_as_timed_out() {
    let mut rig = Rig::new().await;
    rig.worktree(1);
    let pid_file = rig.home.root().join("pid");
    rig.start();
    let started = Instant::now();

    let record = recorded(rig.ask(
        1,
        &format!("echo $$ > {pid_file:?}; sleep 30"),
        Duration::from_millis(300),
    ))
    .await;

    assert_eq!(record.result.outcome, GateOutcome::TimedOut);
    assert!(!record.counts_as_pass());
    assert!(
        started.elapsed() < Duration::from_secs(20),
        "the gate was not stopped at its timeout"
    );
    let pid: i32 = std::fs::read_to_string(&pid_file).unwrap().trim().parse().unwrap();
    eventually("the gate to be gone", || is_gone(pid)).await;
    assert_eq!(rig.saved_record(1), record);
}

#[tokio::test]
async fn the_next_gate_runs_after_one_that_timed_out() {
    let mut rig = Rig::new().await;
    rig.worktree(1);
    rig.worktree(2);
    rig.start();
    let slow = rig.ask(1, "sleep 30", Duration::from_millis(200));
    let after = rig.ask(2, "true", LONG);

    let (slow, after) = (recorded(slow).await, recorded(after).await);

    assert_eq!(slow.result.outcome, GateOutcome::TimedOut);
    assert_eq!(after.result.outcome, GateOutcome::Passed);
}

#[tokio::test]
async fn the_gates_of_a_task_have_a_log_each_and_the_record_is_the_last() {
    let mut rig = Rig::new().await;
    rig.worktree(1);
    rig.start();

    let first = recorded(rig.ask(1, "echo one", LONG)).await;
    let second = recorded(rig.ask(1, "echo two", LONG)).await;

    assert_ne!(first.result.log, second.result.log);
    assert_eq!(std::fs::read_to_string(first.result.log.as_path()).unwrap(), "one\n");
    assert_eq!(std::fs::read_to_string(second.result.log.as_path()).unwrap(), "two\n");
    assert_eq!(rig.saved_record(1), second);
}

#[tokio::test]
async fn a_gate_that_ends_is_told_to_the_subscribers_and_logged_in_the_activity_of_the_task() {
    let mut rig = Rig::new().await;
    rig.worktree(1);
    rig.start();

    let record = recorded(rig.ask(1, "exit 2", LONG)).await;

    let event = tokio::time::timeout(PATIENCE, rig.events.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        event.event,
        EventKind::GateFinished {
            team: team_id(1),
            task: task(1),
            outcome: record.result.outcome,
        }
    );
    let activity = std::fs::read_to_string(rig.home.activity_log(&task(1))).unwrap();
    let line: serde_json::Value = serde_json::from_str(activity.lines().last().unwrap()).unwrap();
    assert_eq!(line["event"]["kind"], serde_json::json!("info"), "{line}");
    assert!(
        line["event"]["text"].as_str().unwrap().contains("gate failed (exit 2)"),
        "{line}"
    );
}

#[tokio::test]
async fn a_gate_of_a_task_that_was_stopped_while_it_waited_is_not_run() {
    let mut rig = Rig::new().await;
    rig.worktree(1);
    let marker = rig.home.root().join("ran");
    let (job, answer) = rig.job(1, &format!("touch {marker:?}"), LONG);
    job.cancel.cancel();
    rig.queue.enqueue(job).unwrap();
    rig.start();

    let failure = refused(answer).await;

    assert_eq!(failure.kind, FailureKind::Conflict);
    assert!(!marker.exists(), "a cancelled gate was run");
    assert!(!rig.record_file(1).exists());
}

#[tokio::test]
async fn a_gate_that_is_stopped_while_it_runs_ends_with_what_it_started_and_records_nothing() {
    let mut rig = Rig::new().await;
    rig.worktree(1);
    let pid_file = rig.home.root().join("pid");
    let (job, answer) = rig.job(1, &format!("echo $$ > {pid_file:?}; sleep 30"), LONG);
    let cancel = job.cancel.clone();
    rig.queue.enqueue(job).unwrap();
    rig.start();
    eventually("the gate to start", || pid_file.is_file()).await;

    cancel.cancel();
    let failure = refused(answer).await;

    assert_eq!(failure.kind, FailureKind::Conflict);
    let pid: i32 = std::fs::read_to_string(&pid_file).unwrap().trim().parse().unwrap();
    eventually("the gate to be gone", || is_gone(pid)).await;
    assert!(!rig.record_file(1).exists());
}

#[tokio::test]
async fn a_worktree_git_cannot_read_refuses_the_gate_and_runs_nothing() {
    let mut rig = Rig::new().await;
    let tree = rig.home.worktree_dir(&repo(), &task(1));
    std::fs::create_dir_all(&tree).unwrap();
    let marker = rig.home.root().join("ran");
    rig.start();

    let failure = refused(rig.ask(1, &format!("touch {marker:?}"), LONG)).await;

    assert_eq!(failure.kind, FailureKind::Refused);
    assert!(failure.message.contains("gate"), "{}", failure.message);
    assert!(!marker.exists());
}

#[tokio::test]
async fn a_gate_command_that_cannot_be_started_refuses_the_gate() {
    let mut rig = Rig::new().await;
    rig.worktree(1);
    let (reply, answer) = oneshot::channel();
    rig.queue
        .enqueue(GateJob {
            task: task(1),
            team: team_id(1),
            worktree: abs(&rig.home.worktree_dir(&repo(), &task(1))),
            command: CommandLine::try_from(vec!["/nonexistent/gate".to_owned()]).unwrap(),
            timeout: LONG,
            cancel: rig.shutdown.child_token(),
            hold: None,
            reply,
        })
        .unwrap();
    rig.start();

    let failure = refused(answer).await;

    assert_eq!(failure.kind, FailureKind::Refused);
    assert!(failure.message.contains("gate"), "{}", failure.message);
}

#[tokio::test]
async fn a_waiting_gate_holds_the_watchdog_off_its_run_until_it_is_answered() {
    let mut rig = Rig::new().await;
    rig.worktree(1);
    let hold = WatchdogHold::new();
    let (mut job, answer) = rig.job(1, "true", LONG);
    job.hold = Some(hold.hold());
    rig.queue.enqueue(job).unwrap();
    let before = hold.last_held();
    assert!(before.is_some(), "a queued gate holds the watchdog");

    rig.start();
    recorded(answer).await;

    let released = hold.last_held();
    assert_eq!(
        hold.last_held(),
        released,
        "the hold is released once the gate is answered"
    );
}

#[tokio::test]
async fn a_gate_asked_beyond_the_queue_is_given_back_full_and_after_the_service_ended_stopped() {
    let rig = Rig::new().await;
    for _ in 0..QUEUE {
        let (job, _answer) = rig.job(1, "true", LONG);
        rig.queue.enqueue(job).unwrap();
    }
    let (job, _answer) = rig.job(1, "true", LONG);

    let full = rig.queue.enqueue(job).unwrap_err();

    assert!(matches!(full, NotQueued::Full(_)), "{full:?}");

    let (queue, waiting) = GateQueue::new();
    drop(waiting);
    let (job, _answer) = rig.job(1, "true", LONG);

    let stopped = queue.enqueue(job).unwrap_err();

    assert!(matches!(stopped, NotQueued::Stopped(_)), "{stopped:?}");
}

#[test]
fn a_signal_is_an_exit_code_above_128_and_a_zero_exit_is_a_pass() {
    use agent_runtime::process::ExitReport;

    assert_eq!(outcome_of(ExitReport::Exited { code: 0 }), GateOutcome::Passed);
    assert_eq!(
        outcome_of(ExitReport::Exited { code: 101 }),
        GateOutcome::Failed {
            exit_code: std::num::NonZeroI32::new(101).unwrap()
        }
    );
    assert_eq!(
        outcome_of(ExitReport::Signaled { signal: 9 }),
        GateOutcome::Failed {
            exit_code: std::num::NonZeroI32::new(137).unwrap()
        }
    );
}
