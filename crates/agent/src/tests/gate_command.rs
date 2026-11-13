//! `agent gate` on the command line: how it is parsed, which task it takes, and what it does before it asks the daemon.

use std::ffi::OsString;
use std::path::PathBuf;
use std::process::ExitCode;

use agent_core::protocol::{Command as Request, Frame, GateRequest, Reply, Response};
use agent_core::state::{GateOutcome, GateRecord, GateResult, GateTree};

use super::*;
use crate::fixtures::task;
use crate::testing::{Answer, FakeDaemon};

fn environment(dir: &tempfile::TempDir) -> Environment {
    Environment::new(Some(OsString::from(dir.path())))
}

fn parsed(args: &[&str]) -> Result<GateArgs, clap::Error> {
    match parse_invocation(std::iter::once("agent").chain(args.iter().copied()))? {
        Invocation::Gate(args) => Ok(args),
        other => panic!("`agent gate` was expected: {other:?}"),
    }
}

#[test]
fn gate_takes_an_optional_task() {
    assert_eq!(parsed(&["gate"]).unwrap().task, None);
    assert_eq!(parsed(&["gate", task(1).as_str()]).unwrap().task, Some(task(1)));
}

#[test]
fn gate_refuses_a_name_that_is_not_a_task_and_a_second_task() {
    use clap::error::ErrorKind;
    let cases = [
        (&["gate", "nonsense"][..], ErrorKind::ValueValidation),
        (&["gate", "team-20261111-153601-k3x9"], ErrorKind::ValueValidation),
        (
            &["gate", "task-20261111-153001-k3x9", "task-20261111-153002-k3x9"],
            ErrorKind::UnknownArgument,
        ),
    ];

    for (args, kind) in cases {
        assert_eq!(parsed(args).unwrap_err().kind(), kind, "{args:?}");
    }
}

#[test]
fn the_help_lists_gate() {
    let help = command().render_help().to_string();

    assert!(help.contains("gate"), "{help}");
}

/// A worktree of task 1 in a home in a temporary directory, and the environment of a command run in it.
fn in_worktree(dir: &tempfile::TempDir) -> (Environment, PathBuf) {
    let root = dir.path().canonicalize().unwrap();
    let worktree = root.join("worktrees/team-agents").join(task(1).as_str());
    std::fs::create_dir_all(&worktree).unwrap();
    (
        Environment::new(Some(OsString::from(&root))).in_directory(worktree.clone()),
        worktree,
    )
}

#[tokio::test]
async fn gate_in_a_worktree_asks_the_daemon_for_the_gate_of_its_task() {
    let dir = tempfile::tempdir().unwrap();
    let (environment, _) = in_worktree(&dir);
    let log = dir.path().join("gate-1.log");
    std::fs::write(&log, "all passed\n").unwrap();
    let record = GateRecord {
        tree: GateTree::Clean,
        result: GateResult {
            head: "26b65de7e4076596768ca96b48a7f94799c93874".parse().unwrap(),
            finished_at: "2026-11-13T09:30:00Z".parse().unwrap(),
            outcome: GateOutcome::Passed,
            log: log.to_str().unwrap().parse().unwrap(),
        },
    };
    let daemon = FakeDaemon::serve(&dir.path().canonicalize().unwrap().join("agent.sock"), move |request| {
        Answer::Lines(vec![
            Frame::Response(Response::ok(request.id, Reply::GateRecorded { record: record.clone() }))
                .to_line()
                .unwrap(),
        ])
    });

    let mut out = Vec::new();
    let result = execute_gate(&environment, GateArgs { task: None }, &mut out).await;

    result.unwrap();
    let out = String::from_utf8(out).unwrap();
    assert!(
        out.contains("all passed") && out.contains("gate: passed at 26b65de"),
        "{out}"
    );
    assert_eq!(
        daemon.requests()[0].command,
        Request::Gate(GateRequest { task: task(1) })
    );
}

#[test]
fn gate_outside_a_worktree_without_a_task_fails_before_it_asks_the_daemon() {
    let dir = tempfile::tempdir().unwrap();
    let environment = environment(&dir).in_directory(dir.path().to_path_buf());

    let (code, out, err) = agent(&environment, &["gate"]);

    assert_eq!(code, ExitCode::FAILURE);
    assert_eq!(out, "");
    assert!(err.contains("not in a task's worktree"), "{err}");
    assert!(err.contains("agent gate task-"), "{err}");
}

#[test]
fn gate_of_a_task_when_no_daemon_runs_says_so() {
    let dir = tempfile::tempdir().unwrap();
    let environment = environment(&dir);

    let (code, _, err) = agent(&environment, &["gate", task(1).as_str()]);

    assert_eq!(code, ExitCode::FAILURE);
    assert!(err.contains("the daemon is not running"), "{err}");
}

/// Runs `agent` with `args` in `environment` and returns its status, standard output and standard error.
fn agent(environment: &Environment, args: &[&str]) -> (ExitCode, String, String) {
    let mut out = Vec::new();
    let mut err = Vec::new();
    let code = run_in(
        environment,
        std::iter::once("agent").chain(args.iter().copied()),
        &mut out,
        &mut err,
    )
    .unwrap();
    (code, String::from_utf8(out).unwrap(), String::from_utf8(err).unwrap())
}

#[tokio::test]
async fn gate_says_first_where_its_result_is_kept_for_a_command_that_is_cut_short() {
    let dir = tempfile::tempdir().unwrap();
    let (environment, _) = in_worktree(&dir);
    let root = dir.path().canonicalize().unwrap();
    let _daemon = FakeDaemon::serve(&root.join("agent.sock"), |request| {
        Answer::Lines(vec![
            Frame::Response(agent_core::protocol::Response::error(
                request.id,
                agent_core::protocol::Failure {
                    kind: agent_core::protocol::FailureKind::Refused,
                    message: "no gate here".to_owned(),
                },
            ))
            .to_line()
            .unwrap(),
        ])
    });

    let mut out = Vec::new();
    let result = execute_gate(&environment, GateArgs { task: None }, &mut out).await;

    result.unwrap_err();
    let out = String::from_utf8(out).unwrap();
    let record = root.join("tasks").join(task(1).as_str()).join("gate.json");
    let first = out.lines().next().unwrap();
    assert!(
        first.contains(record.to_str().unwrap()),
        "the first line names the file of the record: {out}"
    );
}
