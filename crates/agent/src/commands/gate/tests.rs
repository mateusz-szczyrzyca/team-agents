//! `agent gate` against a stand-in for the daemon: how the task is found, what is printed, when the command fails, and
//! that it waits for the gate as long as the daemon takes.

use std::num::NonZeroI32;
use std::os::unix::fs::symlink;

use agent_core::config::AbsolutePath;
use agent_core::protocol::{Client, Command, Failure, FailureKind, Frame, GateRequest, Reply, Response};
use agent_core::state::{GateOutcome, GateRecord, GateResult, GateTree};
use std::str::FromStr;
use std::time::Duration;

use super::*;
use crate::client::REQUEST_TIMEOUT;
use crate::fixtures::task;
use crate::testing::{Answer, FakeDaemon};

const HEAD: &str = "26b65de7e4076596768ca96b48a7f94799c93874";

fn home_at(path: &Path) -> Home {
    Home::new(&AbsolutePath::from_str(path.to_str().unwrap()).unwrap())
}

fn socket(dir: &tempfile::TempDir) -> PathBuf {
    dir.path().join("agent.sock")
}

/// A record of a gate whose output is `log`.
fn record(tree: GateTree, outcome: GateOutcome, log: &Path) -> GateRecord {
    GateRecord {
        tree,
        result: GateResult {
            head: HEAD.parse().unwrap(),
            finished_at: "2026-11-13T09:30:00Z".parse().unwrap(),
            outcome,
            log: AbsolutePath::from_str(log.to_str().unwrap()).unwrap(),
        },
    }
}

fn recorded(record: GateRecord) -> impl Fn(&agent_core::protocol::Request) -> Answer + Send + Sync + 'static {
    move |request| {
        Answer::Lines(vec![
            Frame::Response(Response::ok(request.id, Reply::GateRecorded { record: record.clone() }))
                .to_line()
                .unwrap(),
        ])
    }
}

/// A log of `lines` lines, `line 1` to `line N`, in a temporary directory.
fn log_of(dir: &tempfile::TempDir, lines: usize) -> PathBuf {
    let path = dir.path().join("gate-1.log");
    let text: String = (1..=lines).map(|n| format!("line {n}\n")).collect();
    std::fs::write(&path, text).unwrap();
    path
}

/// Runs `agent gate` for task 1 against a daemon that answers with `record`, and returns what it printed and how it ended.
async fn gated(
    record: GateRecord,
    dir: &tempfile::TempDir,
) -> (Result<(), GateError>, String, Vec<agent_core::protocol::Request>) {
    let daemon = FakeDaemon::serve(&socket(dir), recorded(record));
    let mut connection = Connection::connect(daemon.socket()).await.unwrap();
    let mut out = Vec::new();

    let result = gate(&mut connection, task(1), &mut out).await;

    (result, String::from_utf8(out).unwrap(), daemon.requests())
}

// ---- the task of a directory -----------------------------------------------------------------------------------

#[tokio::test]
async fn the_task_of_a_directory_is_the_second_name_under_the_worktrees_of_the_home() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    let home = home_at(&root);
    let worktree = root.join("worktrees/team-agents").join(task(1).as_str());
    std::fs::create_dir_all(worktree.join("crates/agent")).unwrap();

    assert_eq!(task_in(&home, &worktree).await, Some(task(1)));
    assert_eq!(task_in(&home, &worktree.join("crates/agent")).await, Some(task(1)));
}

#[tokio::test]
async fn a_directory_that_is_no_task_worktree_has_no_task() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    let home = home_at(&root);
    std::fs::create_dir_all(root.join("worktrees/team-agents/not-a-task")).unwrap();
    std::fs::create_dir_all(root.join("tasks")).unwrap();
    let elsewhere = tempfile::tempdir().unwrap();

    for directory in [
        root.clone(),
        root.join("worktrees"),
        root.join("worktrees/team-agents"),
        root.join("worktrees/team-agents/not-a-task"),
        root.join("tasks"),
        elsewhere.path().to_path_buf(),
    ] {
        assert_eq!(task_in(&home, &directory).await, None, "{directory:?}");
    }
}

#[tokio::test]
async fn a_worktree_is_found_when_the_home_is_named_through_a_symbolic_link() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    let real = root.join("real");
    let linked = root.join("linked");
    let worktree = real.join("worktrees/team-agents").join(task(1).as_str());
    std::fs::create_dir_all(&worktree).unwrap();
    symlink(&real, &linked).unwrap();

    // The process's working directory is the resolved path; AGENTS_HOME is what the user wrote.
    assert_eq!(task_in(&home_at(&linked), &worktree).await, Some(task(1)));
}

// ---- what is printed ---------------------------------------------------------------------------------------------

#[tokio::test]
async fn a_gate_that_passes_prints_the_end_of_its_output_and_the_verdict_and_succeeds() {
    let dir = tempfile::tempdir().unwrap();
    let log = log_of(&dir, 100);

    let (result, out, requests) = gated(record(GateTree::Clean, GateOutcome::Passed, &log), &dir).await;

    result.unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].client, Client::Cli);
    assert_eq!(requests[0].command, Command::Gate(GateRequest { task: task(1) }));
    let lines: Vec<&str> = out.lines().collect();
    assert!(
        lines[0].contains(task(1).as_str()),
        "the command says what it waits for: {out}"
    );
    assert!(out.contains("line 100\n") && out.contains("line 41\n"), "{out}");
    assert!(!out.contains("line 40\n"), "only the last {TAIL_LINES} lines: {out}");
    let verdict = lines.last().unwrap();
    assert!(verdict.starts_with("gate: passed at 26b65de"), "{out}");
    assert!(
        verdict.contains(log.to_str().unwrap()),
        "the verdict names the log: {out}"
    );
    assert!(!out.contains("not clean"), "{out}");
}

#[tokio::test]
async fn a_short_log_is_printed_whole() {
    let dir = tempfile::tempdir().unwrap();
    let log = log_of(&dir, 3);

    let (result, out, _) = gated(record(GateTree::Clean, GateOutcome::Passed, &log), &dir).await;

    result.unwrap();
    assert!(out.contains("line 1\nline 2\nline 3\n"), "{out}");
}

#[tokio::test]
async fn a_pass_on_a_tree_that_is_not_clean_succeeds_and_says_it_is_not_a_pass_of_the_commit() {
    let dir = tempfile::tempdir().unwrap();
    let log = log_of(&dir, 2);

    let (result, out, _) = gated(record(GateTree::NotClean, GateOutcome::Passed, &log), &dir).await;

    result.unwrap();
    assert!(out.contains("gate: passed at 26b65de, tree not clean"), "{out}");
    assert!(out.contains("not a pass of the commit"), "{out}");
}

#[tokio::test]
async fn a_gate_that_fails_or_times_out_prints_what_it_said_and_fails() {
    let dir = tempfile::tempdir().unwrap();
    let log = log_of(&dir, 2);
    let cases = [
        (
            GateOutcome::Failed {
                exit_code: NonZeroI32::new(2).unwrap(),
            },
            "failed (exit 2) at 26b65de",
        ),
        (GateOutcome::TimedOut, "timed out at 26b65de"),
    ];

    for (outcome, summary) in cases {
        let (result, out, _) = gated(record(GateTree::Clean, outcome, &log), &dir).await;

        let Err(GateError::NotPassed {
            summary: said,
            log: named,
        }) = result
        else {
            panic!("the gate did not pass: {result:?}");
        };
        assert_eq!(said, summary);
        assert_eq!(named, log);
        assert!(
            out.contains("line 2\n") && out.contains(&format!("gate: {summary}")),
            "{out}"
        );
        std::fs::remove_file(dir.path().join("agent.sock")).unwrap();
    }
}

#[tokio::test]
async fn a_log_that_cannot_be_read_is_told_and_the_verdict_still_comes() {
    let dir = tempfile::tempdir().unwrap();
    let missing = dir.path().join("gone.log");

    let (result, out, _) = gated(record(GateTree::Clean, GateOutcome::Passed, &missing), &dir).await;

    result.unwrap();
    assert!(
        out.contains("cannot read the output") && out.contains("gone.log"),
        "{out}"
    );
    assert!(out.contains("gate: passed at 26b65de"), "{out}");
}

// ---- when the daemon does not answer with a result --------------------------------------------------------------------

#[tokio::test]
async fn a_refusal_of_the_daemon_is_the_error_of_the_command() {
    let dir = tempfile::tempdir().unwrap();
    let daemon = FakeDaemon::serve(&socket(&dir), |request| {
        let failure = Failure {
            kind: FailureKind::Refused,
            message: "the gate queue is full".to_owned(),
        };
        Answer::Lines(vec![
            Frame::Response(Response::error(request.id, failure)).to_line().unwrap(),
        ])
    });
    let mut connection = Connection::connect(daemon.socket()).await.unwrap();

    let result = gate(&mut connection, task(1), &mut Vec::new()).await;

    assert!(
        matches!(&result, Err(GateError::Client(ClientError::Refused(failure))) if failure.kind == FailureKind::Refused),
        "{result:?}"
    );
}

#[tokio::test]
async fn another_reply_than_the_result_of_a_gate_is_an_error() {
    let dir = tempfile::tempdir().unwrap();
    let daemon = FakeDaemon::serve(&socket(&dir), |request| {
        Answer::Lines(vec![
            Frame::Response(Response::ok(request.id, Reply::Teams { teams: Vec::new() }))
                .to_line()
                .unwrap(),
        ])
    });
    let mut connection = Connection::connect(daemon.socket()).await.unwrap();

    let result = gate(&mut connection, task(1), &mut Vec::new()).await;

    assert!(
        matches!(result, Err(GateError::UnexpectedReply { kind: "teams" })),
        "{result:?}"
    );
}

#[tokio::test]
async fn a_closed_output_is_an_error_not_a_lost_verdict() {
    struct Closed;
    impl Write for Closed {
        fn write(&mut self, _: &[u8]) -> io::Result<usize> {
            Err(io::Error::from(io::ErrorKind::BrokenPipe))
        }
        fn flush(&mut self) -> io::Result<()> {
            Err(io::Error::from(io::ErrorKind::BrokenPipe))
        }
    }
    let dir = tempfile::tempdir().unwrap();
    let log = log_of(&dir, 1);
    let daemon = FakeDaemon::serve(
        &socket(&dir),
        recorded(record(GateTree::Clean, GateOutcome::Passed, &log)),
    );
    let mut connection = Connection::connect(daemon.socket()).await.unwrap();

    let result = gate(&mut connection, task(1), &mut Closed).await;

    assert!(matches!(result, Err(GateError::Output(_))), "{result:?}");
}

// ---- the wait ---------------------------------------------------------------------------------------------------

/// A daemon that answers a request only after `delay`, with `reply`.
async fn slow_daemon(dir: &tempfile::TempDir, delay: Duration, reply: Reply) -> tokio::task::JoinHandle<()> {
    let listener = tokio::net::UnixListener::bind(socket(dir)).unwrap();
    tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let (read, mut write) = stream.into_split();
        let mut reader = tokio::io::BufReader::new(read);
        let line = crate::lines::read_line(&mut reader, 1 << 20).await.unwrap().unwrap();
        let request = agent_core::protocol::Request::from_line(&line).unwrap();
        tokio::time::sleep(delay).await;
        let answer = Frame::Response(Response::ok(request.id, reply)).to_line().unwrap();
        crate::lines::write_line(&mut write, &answer).await.unwrap();
    })
}

#[tokio::test(start_paused = true)]
async fn a_gate_that_waits_for_its_turn_is_waited_for_beyond_the_time_a_request_may_take() {
    let dir = tempfile::tempdir().unwrap();
    let log = log_of(&dir, 1);
    let told = Reply::GateRecorded {
        record: record(GateTree::Clean, GateOutcome::Passed, &log),
    };
    let serving = slow_daemon(&dir, REQUEST_TIMEOUT * 10, told).await;
    let mut connection = Connection::connect(&socket(&dir)).await.unwrap();

    let result = gate(&mut connection, task(1), &mut Vec::new()).await;

    result.unwrap();
    // Bounded, so that a command that never asked fails the test instead of waiting for the stand-in for ever.
    tokio::time::timeout(REQUEST_TIMEOUT * 20, serving)
        .await
        .expect("the stand-in daemon never got the request")
        .unwrap();
}

#[tokio::test(start_paused = true)]
async fn a_daemon_that_ends_while_a_gate_waits_closes_the_connection_which_is_an_error() {
    let dir = tempfile::tempdir().unwrap();
    let daemon = FakeDaemon::serve(&socket(&dir), |_| Answer::Close);
    let mut connection = Connection::connect(daemon.socket()).await.unwrap();

    let result = gate(&mut connection, task(1), &mut Vec::new()).await;

    assert!(
        matches!(result, Err(GateError::Client(ClientError::Closed))),
        "{result:?}"
    );
}
