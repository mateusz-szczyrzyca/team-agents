//! The commands of the command line: how they are parsed, what they ask of the daemon and what they print.

use std::ffi::OsString;
use std::process::ExitCode;

use agent_core::protocol::{Command as Request, Frame, Reply, Response, StatusRequest, Target};

use super::*;
use crate::fixtures::{task, team_state};
use crate::testing::{Answer, FakeDaemon};

fn environment(dir: &tempfile::TempDir) -> Environment {
    Environment::new(Some(OsString::from(dir.path())))
}

fn lines(request: &agent_core::protocol::Request, reply: Reply) -> Answer {
    Answer::Lines(vec![
        Frame::Response(Response::ok(request.id, reply)).to_line().unwrap(),
    ])
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

#[test]
fn status_without_agents_home_fails_and_says_what_to_set() {
    let (code, out, err) = agent(&Environment::new(None), &["status"]);

    assert_eq!(code, ExitCode::FAILURE);
    assert_eq!(out, "");
    assert!(
        err.starts_with("agent: ") && err.contains("AGENTS_HOME is not set"),
        "{err}"
    );
}

#[test]
fn status_with_no_daemon_says_it_is_not_running_and_how_to_start_it() {
    let dir = tempfile::tempdir().unwrap();

    let (code, out, err) = agent(&environment(&dir), &["status"]);

    assert_eq!(code, ExitCode::FAILURE);
    assert_eq!(out, "");
    assert!(
        err.contains("the daemon is not running") && err.contains("agent daemon"),
        "{err}"
    );
}

#[tokio::test]
async fn status_prints_the_teams_the_daemon_lists() {
    let dir = tempfile::tempdir().unwrap();
    let teams = vec![agent_core::protocol::TeamSummary::from(&team_state().teams()[0])];
    let listed = teams.clone();
    let daemon = FakeDaemon::serve(&dir.path().join("agent.sock"), move |request| {
        lines(request, Reply::Teams { teams: listed.clone() })
    });
    let mut out = Vec::new();

    let result = execute(
        &environment(&dir),
        Command::Status(StatusArgs { target: None }),
        &mut out,
    )
    .await;

    result.unwrap();
    assert_eq!(String::from_utf8(out).unwrap(), crate::commands::status::render(&teams));
    assert_eq!(
        daemon.requests()[0].command,
        Request::Status(StatusRequest { target: None })
    );
}

#[test]
fn the_target_of_status_is_a_team_or_a_task_and_nothing_else() {
    let team = "team-20261111-153000-t5m8";
    let task_id = task(2);

    let by_team = Cli::try_parse_from(["agent", "status", team]).unwrap();
    let by_task = Cli::try_parse_from(["agent", "status", task_id.as_str()]).unwrap();
    let by_nothing = Cli::try_parse_from(["agent", "status"]).unwrap();
    let refused = Cli::try_parse_from(["agent", "status", "bogus"]).unwrap_err();

    let target = |cli: Cli| match cli.command {
        Some(Command::Status(args)) => args.target,
        Some(Command::Daemon(_)) | None => panic!("not the status command"),
    };
    assert_eq!(target(by_team), Some(Target::Team(team.parse().unwrap())));
    assert_eq!(target(by_task), Some(Target::Task(task_id)));
    assert_eq!(target(by_nothing), None);
    assert_eq!(refused.kind(), clap::error::ErrorKind::ValueValidation);
}

#[test]
fn a_target_that_is_neither_is_refused_with_status_2_and_a_message_naming_it() {
    let (code, out, err) = agent(&Environment::new(None), &["status", "bogus"]);

    assert_eq!(code, ExitCode::from(2));
    assert_eq!(out, "");
    assert!(err.contains("bogus"), "{err}");
}

#[test]
fn the_daemon_takes_an_optional_prompts_directory() {
    let plain = Cli::try_parse_from(["agent", "daemon"]).unwrap();
    let given = Cli::try_parse_from(["agent", "daemon", "--prompts-dir", "/srv/prompts"]).unwrap();

    let dir = |cli: Cli| match cli.command {
        Some(Command::Daemon(args)) => args.prompts_dir,
        _ => panic!("not the daemon"),
    };
    assert_eq!(dir(plain), None);
    assert_eq!(dir(given), Some(std::path::PathBuf::from("/srv/prompts")));
}

#[test]
fn the_daemon_without_agents_home_fails_and_says_what_to_set() {
    let (code, out, err) = agent(&Environment::new(None), &["daemon"]);

    assert_eq!(code, ExitCode::FAILURE);
    assert_eq!(out, "");
    assert!(
        err.starts_with("agent: ") && err.contains("AGENTS_HOME is not set"),
        "{err}"
    );
}

#[test]
fn the_status_of_a_round_is_asked_by_its_id_and_the_other_commands_do_not_take_one() {
    let round = "round-20261111-153001-q1w2";

    let status = Cli::try_parse_from(["agent", "status", round]).unwrap();
    let stop = crate::parse_invocation(["agent", "stop", round]).unwrap_err();
    let malformed = Cli::try_parse_from(["agent", "status", "round-20261311-153001-q1w2"]).unwrap_err();

    let target = match status.command {
        Some(Command::Status(args)) => args.target,
        Some(Command::Daemon(_)) | None => panic!("not the status command"),
    };
    assert_eq!(target, Some(Target::Round(round.parse().unwrap())));
    assert_eq!(stop.kind(), clap::error::ErrorKind::ValueValidation);
    assert_eq!(malformed.kind(), clap::error::ErrorKind::ValueValidation);
    assert!(malformed.to_string().contains("round ID"), "{malformed}");
}
