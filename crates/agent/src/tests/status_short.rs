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

    let expected = format!("{}daemon: version unknown\n{QUIET_SHORT}", this_command_line());
    assert_eq!(text, expected, "{text}");
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

// Rule: `--json` is the bare array of teams as before, with and without `--task`: the flag is taken with `--json` and
// changes nothing in what it prints.
#[test]
fn json_is_taken_with_and_without_task() {
    Cli::try_parse_from(["agent", "status", "--json"]).expect("--json is taken");
    Cli::try_parse_from(["agent", "status", "--json", "--task"]).expect("--json takes --task too");
    Cli::try_parse_from(["agent", "status", "--investigators", "--task"]).expect_err("--investigators takes no --task");
}

// Rule: `--task` shows the tasks of a team list, so it is refused with a target (which already shows its tasks) and with
// the investigators view; `--json` already carries every task, so it is accepted with it.
#[test]
fn task_is_refused_with_a_target_and_with_the_investigators() {
    Cli::try_parse_from(["agent", "status", "team-20261111-153000-t5m8", "--task"])
        .expect_err("a target takes no --task");
    Cli::try_parse_from(["agent", "status", "--investigators", "--task"]).expect_err("--investigators takes no --task");
}

#[tokio::test]
async fn target_pages_for_team_task_and_round_print_what_they_printed() {
    let dir = tempfile::tempdir().unwrap();
    let mut state = team_state();
    let round_id: agent_core::ids::RoundId = "round-20261111-153001-q1w2".parse().unwrap();
    state
        .team_mut(&team_id(0))
        .unwrap()
        .open_round(round_id.clone(), Some("1.1".parse().unwrap()), at(10))
        .unwrap();
    let team = state.team(&team_id(0)).unwrap().clone();
    let round = team.rounds().last().unwrap().clone();
    let detail = RoundDetail::new(&team, &round, Vec::new());
    let summary = TeamSummary::from(&team);
    let reply_summary = summary.clone();
    let reply_detail = detail.clone();
    let _fake = FakeDaemon::serve(&socket(&dir), move |request| {
        let reply = match &request.command {
            Wire::Show(_) => Reply::Round {
                round: Box::new(reply_detail.clone()),
            },
            _ => Reply::Teams {
                teams: vec![reply_summary.clone()],
            },
        };
        Answer::Lines(vec![
            Frame::Response(Response::ok(request.id, reply)).to_line().unwrap(),
        ])
    });

    for target in [Target::Team(team_id(0)), Target::Task(crate::fixtures::task(2))] {
        let mut out = Vec::new();
        execute(
            &environment(&dir),
            Command::Status(StatusArgs {
                target: Some(target.clone()),
                running: false,
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
            !text.starts_with("agent "),
            "target page for {target:?} must not prepend list version header: {text}"
        );
        assert!(
            text.contains("  task-20261111-153002-k3x9  member       claude-1   queued\n"),
            "target page for {target:?} must include task lines: {text}"
        );
    }

    let mut round_out = Vec::new();
    execute(
        &environment(&dir),
        Command::Status(StatusArgs {
            target: Some(Target::Round(round_id)),
            running: false,
            task: false,
            investigators: false,
            json: false,
        }),
        &mut round_out,
    )
    .await
    .unwrap();
    let round_text = String::from_utf8(round_out).unwrap();
    assert!(!round_text.starts_with("agent "), "{round_text}");
    assert!(
        round_text.starts_with("round-20261111-153001-q1w2  item 1.1  open  team team-20261111-153000-t5m8\n"),
        "{round_text}"
    );
}

#[tokio::test]
async fn json_output_is_identical_with_and_without_task() {
    let dir = tempfile::tempdir().unwrap();
    let teams = vec![quiet()];
    let without_task = {
        let listed = teams.clone();
        let _fake = FakeDaemon::serve(&socket(&dir), move |request| {
            Answer::Lines(vec![
                Frame::Response(Response::ok(request.id, Reply::Teams { teams: listed.clone() }))
                    .to_line()
                    .unwrap(),
            ])
        });
        let mut out = Vec::new();
        execute(
            &environment(&dir),
            Command::Status(StatusArgs {
                target: None,
                running: false,
                task: false,
                investigators: false,
                json: true,
            }),
            &mut out,
        )
        .await
        .unwrap();
        String::from_utf8(out).unwrap()
    };

    let dir_with = tempfile::tempdir().unwrap();
    let with_task = {
        let listed = teams.clone();
        let _fake = FakeDaemon::serve(&socket(&dir_with), move |request| {
            Answer::Lines(vec![
                Frame::Response(Response::ok(request.id, Reply::Teams { teams: listed.clone() }))
                    .to_line()
                    .unwrap(),
            ])
        });
        let mut out = Vec::new();
        execute(
            &environment(&dir_with),
            Command::Status(StatusArgs {
                target: None,
                running: false,
                task: true,
                investigators: false,
                json: true,
            }),
            &mut out,
        )
        .await
        .unwrap();
        String::from_utf8(out).unwrap()
    };

    assert_eq!(without_task, with_task);
    assert_eq!(without_task, crate::commands::status::render_json(&teams).unwrap());
}
