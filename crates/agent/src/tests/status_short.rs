//! `agent status` in its short view and with `--task`: the versions come first, the teams and their rounds follow, and
//! the tasks only with `--task`.

use agent_core::protocol::{Command as Wire, Frame, Reply, Response, RoundDetail, TeamSummary, Versions};

use super::*;
use crate::fixtures::{at, team_id, team_state};
use crate::testing::{Answer, FakeDaemon};
use crate::versions::of_this_build;

/// The environment of a home in `dir`.
fn environment(dir: &tempfile::TempDir) -> Environment {
    Environment::new(Some(std::ffi::OsString::from(dir.path())))
}

/// The socket of the fake daemon in `dir`.
fn socket(dir: &tempfile::TempDir) -> std::path::PathBuf {
    dir.path().join("agent.sock")
}

/// The sample team: four queued tasks, two pending items, no round.
fn quiet() -> TeamSummary {
    TeamSummary::from(&team_state().teams()[0])
}

/// The line of the sample team and its count of tasks, as the short view writes them when nothing runs.
const QUIET_SHORT: &str = "team-20261111-153000-t5m8  team-agents  autopilot  2 pending, 0 in progress, 0 in review, 0 approved, 0 done\n  tasks: 4 queued\n";

/// The line of the versions of this build, written out from the crates' own values.
fn this_command_line() -> String {
    let v = of_this_build();
    format!(
        "agent {} · agent-core {} · agent-runtime {} · agent-web {}\n",
        v.agent, v.agent_core, v.agent_runtime, v.agent_web
    )
}

/// Runs `agent status` with `running`, `task` and `target` in `dir`, against a fake daemon that answers with `reply`
/// and tells the versions `daemon`, and returns what it printed.
async fn run_status(
    dir: &tempfile::TempDir,
    reply: Reply,
    daemon: Option<Versions>,
    running: bool,
    task: bool,
) -> String {
    let _fake = FakeDaemon::serve(&socket(dir), move |request| {
        let mut response = Response::ok(request.id, reply.clone());
        response.daemon_versions = daemon.clone();
        Answer::Lines(vec![Frame::Response(response).to_line().unwrap()])
    });
    let mut out = Vec::new();

    execute(
        &environment(dir),
        Command::Status(StatusArgs {
            target: None,
            running,
            task,
            investigators: false,
            json: false,
        }),
        &mut out,
    )
    .await
    .unwrap();

    String::from_utf8(out).unwrap()
}

#[tokio::test]
async fn the_short_list_starts_with_the_versions_and_says_the_daemon_tells_none() {
    let dir = tempfile::tempdir().unwrap();

    let text = run_status(&dir, Reply::Teams { teams: vec![quiet()] }, None, false, false).await;

    assert_eq!(
        text,
        format!(
            "{}{QUIET_SHORT}",
            format!("{}daemon: version unknown\n", this_command_line())
        ),
        "{text}"
    );
}

#[tokio::test]
async fn the_task_list_follows_the_versions_exactly_as_before() {
    let dir = tempfile::tempdir().unwrap();
    let teams = vec![quiet()];

    let text = run_status(&dir, Reply::Teams { teams: teams.clone() }, None, false, true).await;

    assert_eq!(
        text,
        format!(
            "{}daemon: version unknown\n{}",
            this_command_line(),
            crate::commands::status::render(&teams)
        )
    );
}

#[tokio::test]
async fn running_names_the_team_and_its_open_round_and_no_task_line() {
    let dir = tempfile::tempdir().unwrap();
    let mut state = team_state();
    state
        .team_mut(&team_id(0))
        .unwrap()
        .open_round(
            "round-20261111-153001-q1w2".parse().unwrap(),
            Some("1.1".parse().unwrap()),
            at(10),
        )
        .unwrap();
    let team = state.team(&team_id(0)).unwrap().clone();
    let round = team.rounds().last().unwrap().clone();
    let detail = RoundDetail::new(&team, &round, Vec::new());
    let summary = TeamSummary::from(&team);
    let _fake = FakeDaemon::serve(&socket(&dir), move |request| {
        let reply = match &request.command {
            Wire::Show(_) => Reply::Round {
                round: Box::new(detail.clone()),
            },
            _ => Reply::Teams {
                teams: vec![summary.clone()],
            },
        };
        Answer::Lines(vec![
            Frame::Response(Response::ok(request.id, reply)).to_line().unwrap(),
        ])
    });
    let mut out = Vec::new();

    execute(
        &environment(&dir),
        Command::Status(StatusArgs {
            target: None,
            running: true,
            task: false,
            investigators: false,
            json: false,
        }),
        &mut out,
    )
    .await
    .unwrap();

    let text = String::from_utf8(out).unwrap();
    assert!(
        text.starts_with(&format!("{}daemon: version unknown\n", this_command_line())),
        "{text}"
    );
    assert!(text.contains("  round-20261111-153001-q1w2  item 1.1  open"), "{text}");
    assert!(text.contains("  tasks: 4 queued\n"), "{text}");
    assert!(!text.contains("  task-20261111-153002-k3x9"), "{text}");
}

#[tokio::test]
async fn running_with_task_names_every_task_under_its_team() {
    let dir = tempfile::tempdir().unwrap();
    let mut state = team_state();
    state
        .team_mut(&team_id(0))
        .unwrap()
        .open_round(
            "round-20261111-153001-q1w2".parse().unwrap(),
            Some("1.1".parse().unwrap()),
            at(10),
        )
        .unwrap();
    let team = state.team(&team_id(0)).unwrap().clone();
    let round = team.rounds().last().unwrap().clone();
    let detail = RoundDetail::new(&team, &round, Vec::new());
    let summary = TeamSummary::from(&team);
    let _fake = FakeDaemon::serve(&socket(&dir), move |request| {
        let reply = match &request.command {
            Wire::Show(_) => Reply::Round {
                round: Box::new(detail.clone()),
            },
            _ => Reply::Teams {
                teams: vec![summary.clone()],
            },
        };
        Answer::Lines(vec![
            Frame::Response(Response::ok(request.id, reply)).to_line().unwrap(),
        ])
    });
    let mut out = Vec::new();

    execute(
        &environment(&dir),
        Command::Status(StatusArgs {
            target: None,
            running: true,
            task: true,
            investigators: false,
            json: false,
        }),
        &mut out,
    )
    .await
    .unwrap();

    let text = String::from_utf8(out).unwrap();
    assert!(text.starts_with(&this_command_line()), "{text}");
    assert!(
        text.contains("  task-20261111-153002-k3x9  member       claude-1   queued\n"),
        "{text}"
    );
}
