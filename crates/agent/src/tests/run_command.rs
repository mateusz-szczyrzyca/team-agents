//! `agent run` on the command line: how it is parsed, what it needs of its environment before it asks the daemon, and
//! what it prints.

use std::ffi::OsString;
use std::path::PathBuf;
use std::process::{Command as Process, ExitCode};

use agent_core::protocol::{Command as Request, Frame, Reply, Response, Target};
use agent_core::state::MergeMode;

use super::*;
use crate::fixtures::{task, team_id, team_state};
use crate::testing::{Answer, FakeDaemon};

fn environment(dir: &tempfile::TempDir) -> Environment {
    Environment::new(Some(OsString::from(dir.path())))
}

fn parsed(args: &[&str]) -> Result<RunArgs, clap::Error> {
    match parse(std::iter::once("agent").chain(args.iter().copied()))? {
        Parsed::Run(run) => Ok(run),
        other @ Parsed::Cli(_) => panic!("`agent run` was expected: {other:?}"),
    }
}

/// A git checkout in a temporary directory, whose origin is `https://example.com/org/team-agents.git`.
fn checkout() -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().canonicalize().unwrap().join("work");
    std::fs::create_dir(&repo).unwrap();
    for args in [
        ["init", "-q", "-b", "main"].as_slice(),
        ["remote", "add", "origin", "https://example.com/org/team-agents.git"].as_slice(),
    ] {
        let status = Process::new("git")
            .args(args)
            .current_dir(&repo)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .status()
            .unwrap();
        assert!(status.success(), "git {args:?}");
    }
    (dir, repo)
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

// ---- parsing -----------------------------------------------------------------------------------------------------

#[test]
fn the_words_the_file_the_coordinator_and_no_merge_are_parsed() {
    let run = parsed(&[
        "run",
        "claude-1",
        "codex-1",
        "Fix it.",
        "--coordinator",
        "codex-1",
        "--no-merge",
        "-f",
        "list.md",
    ])
    .unwrap();

    assert_eq!(run.words, ["claude-1", "codex-1", "Fix it."]);
    assert_eq!(run.file, Some(PathBuf::from("list.md")));
    assert_eq!(run.coordinator.map(|name| name.to_string()), Some("codex-1".to_owned()));
    assert!(run.no_merge);
}

#[test]
fn without_options_a_run_has_no_file_no_coordinator_and_merges() {
    let run = parsed(&["run", "claude-1", "Fix it."]).unwrap();

    assert_eq!(run.file, None);
    assert_eq!(run.coordinator, None);
    assert!(!run.no_merge);
}

#[test]
fn a_run_without_words_is_a_usage_error() {
    let error = parsed(&["run"]).unwrap_err();

    assert_eq!(error.kind(), clap::error::ErrorKind::MissingRequiredArgument);
}

#[test]
fn a_coordinator_that_is_not_an_agent_name_is_a_usage_error() {
    let error = parsed(&["run", "claude-1", "Fix it.", "--coordinator", "Not A Name"]).unwrap_err();

    assert_eq!(error.kind(), clap::error::ErrorKind::ValueValidation);
}

#[test]
fn the_other_commands_are_parsed_as_before_beside_run() {
    let status = parse(["agent", "status"]).unwrap();
    let nothing = parse(["agent"]).unwrap();

    assert!(
        matches!(
            status,
            Parsed::Cli(Cli {
                command: Some(Command::Status(_))
            })
        ),
        "{status:?}"
    );
    assert!(matches!(nothing, Parsed::Cli(Cli { command: None })), "{nothing:?}");
}

#[test]
fn the_help_lists_the_run_command() {
    let (code, out, _) = agent(&Environment::new(None), &["--help"]);

    assert_eq!(code, ExitCode::SUCCESS);
    assert!(out.contains("run"), "{out}");
}

#[test]
fn agent_without_a_command_shows_the_same_help_with_the_run_command() {
    let (code, bare, _) = agent(&Environment::new(None), &[]);
    let (_, help, _) = agent(&Environment::new(None), &["--help"]);

    assert_eq!(code, ExitCode::SUCCESS);
    assert_eq!(bare, help);
    assert!(bare.contains(RUN_ABOUT), "{bare}");
}

// ---- before the daemon is asked ----------------------------------------------------------------------------------

#[test]
fn words_that_make_no_task_are_refused_before_the_daemon_is_looked_for() {
    let dir = tempfile::tempdir().unwrap();
    let (_work, repo) = checkout();

    let (code, out, err) = agent(&environment(&dir).in_directory(repo), &["run", "claude-1"]);

    assert_eq!(code, ExitCode::FAILURE);
    assert_eq!(out, "");
    assert!(
        err.starts_with("agent: ") && err.contains("give the task as the last argument"),
        "{err}"
    );
    assert!(!err.contains("daemon is not running"), "{err}");
}

#[test]
fn a_run_without_a_daemon_says_it_is_not_running() {
    let dir = tempfile::tempdir().unwrap();
    let (_work, repo) = checkout();

    let (code, out, err) = agent(&environment(&dir).in_directory(repo), &["run", "claude-1", "Fix it."]);

    assert_eq!(code, ExitCode::FAILURE);
    assert_eq!(out, "");
    assert!(err.contains("the daemon is not running"), "{err}");
}

#[test]
fn a_run_without_agents_home_says_what_to_set() {
    let (_work, repo) = checkout();

    let (code, _, err) = agent(
        &Environment::new(None).in_directory(repo),
        &["run", "claude-1", "Fix it."],
    );

    assert_eq!(code, ExitCode::FAILURE);
    assert!(err.contains("AGENTS_HOME is not set"), "{err}");
}

#[test]
fn a_run_where_the_directory_is_unknown_says_so() {
    let dir = tempfile::tempdir().unwrap();

    let (code, _, err) = agent(&environment(&dir), &["run", "claude-1", "Fix it."]);

    assert_eq!(code, ExitCode::FAILURE);
    assert!(err.contains("cannot find the directory the command runs in"), "{err}");
}

// ---- the run --------------------------------------------------------------------------------------------------------

#[tokio::test]
async fn a_run_asks_the_daemon_for_the_team_and_prints_what_started() {
    let dir = tempfile::tempdir().unwrap();
    let (_work, repo) = checkout();
    let daemon = FakeDaemon::serve(&dir.path().join("agent.sock"), |request| {
        let reply = match &request.command {
            Request::Run(_) => Reply::Started {
                team: team_id(0),
                tasks: vec![task(1), task(2), task(3)],
            },
            Request::Show(show) => {
                assert_eq!(show.target, Target::Team(team_id(0)));
                Reply::Team {
                    team: Box::new(team_state().teams()[0].clone()),
                }
            }
            other => panic!("{other:?}"),
        };
        Answer::Lines(vec![
            Frame::Response(Response::ok(request.id, reply)).to_line().unwrap(),
        ])
    });
    let mut out = Vec::new();
    let run = parsed(&[
        "run",
        "inv-opus",
        "claude-1",
        "Fix it.",
        "--no-merge",
        "--coordinator",
        "inv-opus",
    ])
    .unwrap();

    execute_run(&environment(&dir).in_directory(repo.clone()), run, &mut out)
        .await
        .unwrap();

    let text = String::from_utf8(out).unwrap();
    assert!(
        text.starts_with("team team-20261111-153000-t5m8 started on team-agents"),
        "{text}"
    );
    assert!(text.contains("coordinator  inv-opus"), "{text}");
    let Request::Run(sent) = daemon.requests().remove(0).command else {
        panic!("a run was expected first")
    };
    assert_eq!(sent.repo.as_str(), "team-agents");
    assert_eq!(sent.checkout.as_path(), repo);
    assert_eq!(sent.merge, MergeMode::NoMerge);
    assert_eq!(
        sent.coordinator.map(|name| name.to_string()),
        Some("inv-opus".to_owned())
    );
}
