//! `agent status` in its short view and with `--task`: the versions come first, the teams and their rounds follow, and
//! the tasks only with `--task`.

use agent_core::history::MaxTextBytes;
use agent_core::local_time::Zone;
use agent_core::protocol::{
    Command as Wire, Failure, FailureKind, Frame, Reply, Request, RequestId, Response, RoundDetail, TeamSummary,
    Versions,
};
use agent_runtime::history::HistoryDb;
use time::UtcOffset;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixListener;

use super::*;
use crate::fixtures::{at, team_id, team_state};
use crate::testing::{Answer, FakeDaemon};
use crate::versions::of_this_build;

/// The environment of a home in `dir`, pinned to UTC so round timestamps are deterministic.
fn environment(dir: &tempfile::TempDir) -> Environment {
    Environment::new(Some(std::ffi::OsString::from(dir.path()))).with_zone(Zone::Fixed(UtcOffset::UTC))
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

/// The full task listing of [`quiet`] before the short view.
const QUIET_TASKS_GOLDEN: &str = "\
team-20261111-153000-t5m8  team-agents  autopilot  2 pending, 0 in progress, 0 in review, 0 approved, 0 done
  task-20261111-153001-k3x9  coordinator  inv-opus   queued
  task-20261111-153002-k3x9  member       claude-1   queued
  task-20261111-153003-k3x9  worker       glm-1      queued
  task-20261111-153004-k3x9  reviewer     inv-codex  queued
";

/// The full running task listing of the sample team with one open round on item 1.1 in UTC.
const RUNNING_TASKS_GOLDEN: &str = "\
team-20261111-153000-t5m8  team-agents  autopilot  2 pending, 0 in progress, 0 in review, 0 approved, 0 done
  round-20261111-153001-q1w2  item 1.1  open, only local  started 2026-11-11 19:15 +00:00
  task-20261111-153001-k3x9  coordinator  inv-opus   queued
  task-20261111-153002-k3x9  member       claude-1   queued
  task-20261111-153003-k3x9  worker       glm-1      queued
  task-20261111-153004-k3x9  reviewer     inv-codex  queued
";

/// The line of the versions of this build, written out from the crates' own values.
fn this_command_line() -> String {
    let v = of_this_build();
    format!(
        "agent {} · agent-core {} · agent-runtime {} · agent-web {}\n",
        v.agent, v.agent_core, v.agent_runtime, v.agent_web
    )
}

/// Runs `agent status` with `running` and `task` in `dir`, against a legacy fake daemon that answers with `reply` and
/// tells no versions, and returns what it printed.
async fn run_status(dir: &tempfile::TempDir, reply: Reply, running: bool, task: bool) -> String {
    let _fake = FakeDaemon::serve(&socket(dir), move |request| {
        let response = Response::ok(request.id, reply.clone());
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

    let text = run_status(&dir, Reply::Teams { teams: vec![quiet()] }, false, false).await;

    let expected = format!("{}daemon: version unknown\n{QUIET_SHORT}", this_command_line());
    assert_eq!(text, expected, "{text}");
}

#[tokio::test]
async fn the_short_list_says_when_the_daemon_tells_other_versions() {
    let dir = tempfile::tempdir().unwrap();
    let listener = UnixListener::bind(socket(&dir)).unwrap();
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let (read, mut write) = stream.into_split();
        let mut reader = BufReader::new(read);
        let mut line = String::new();
        while reader.read_line(&mut line).await.unwrap() != 0 {
            if let Ok((request, format)) = Request::from_line_negotiated(&line) {
                let mut response = Response::ok(request.id, Reply::Teams { teams: vec![quiet()] });
                response.daemon_versions = Some(Versions {
                    agent: String::from("0.1.0"),
                    agent_core: String::from("0.1.0"),
                    agent_runtime: String::from("0.1.0"),
                    agent_web: String::from("0.1.0"),
                });
                let wire = Frame::Response(response).to_line_for(format).unwrap();
                write.write_all(wire.as_bytes()).await.unwrap();
            } else {
                let refusal = Frame::Response(Response::error(
                    RequestId(0),
                    Failure {
                        kind: FailureKind::Invalid,
                        message: String::from("unsupported protocol"),
                    },
                ))
                .to_line()
                .unwrap();
                write.write_all(refusal.as_bytes()).await.unwrap();
                break;
            }
            line.clear();
        }
    });
    let mut out = Vec::new();

    execute(
        &environment(&dir),
        Command::Status(StatusArgs {
            target: None,
            running: false,
            task: false,
            investigators: false,
            json: false,
        }),
        &mut out,
    )
    .await
    .unwrap();
    server.await.unwrap();

    let text = String::from_utf8(out).unwrap();
    assert_eq!(
        text,
        format!(
            "{}daemon: agent 0.1.0 · agent-core 0.1.0 · agent-runtime 0.1.0 · agent-web 0.1.0 (differs from this command)\n{QUIET_SHORT}",
            this_command_line()
        )
    );
}

#[tokio::test]
async fn the_task_list_follows_the_versions_exactly_as_before() {
    let dir = tempfile::tempdir().unwrap();
    let teams = vec![quiet()];

    let text = run_status(&dir, Reply::Teams { teams: teams.clone() }, false, true).await;

    assert_eq!(
        text,
        format!("{}daemon: version unknown\n{QUIET_TASKS_GOLDEN}", this_command_line())
    );
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
    let env = environment(&dir);
    HistoryDb::open(&env.home().unwrap().history_file(), MaxTextBytes::default()).unwrap();
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
        &env,
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
    assert_eq!(
        text,
        format!(
            "{}daemon: version unknown\n\
             team-20261111-153000-t5m8  team-agents  autopilot  2 pending, 0 in progress, 0 in review, 0 approved, 0 done\n  \
             round-20261111-153001-q1w2  item 1.1  open, only local  started 2026-11-11 19:15 +00:00\n  \
             tasks: 4 queued\n",
            this_command_line()
        )
    );
}

#[tokio::test]
async fn running_with_task_names_every_task_under_its_team() {
    let dir = tempfile::tempdir().unwrap();
    let env = environment(&dir);
    let home = env.home().unwrap();
    HistoryDb::open(&home.history_file(), MaxTextBytes::default()).unwrap();
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
        &env,
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

    let mut running_out = Vec::new();
    let mut connection = Connection::connect(&socket(&dir)).await.unwrap();
    crate::commands::status::status_running(&mut connection, &Activity::new(home, env.zone()), &mut running_out)
        .await
        .unwrap();
    let from_status_running = String::from_utf8(running_out).unwrap();

    let text = String::from_utf8(out).unwrap();
    assert_eq!(
        text,
        format!("{}daemon: version unknown\n{RUNNING_TASKS_GOLDEN}", this_command_line())
    );
    assert_eq!(
        text,
        format!("{}daemon: version unknown\n{from_status_running}", this_command_line())
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
    let env = environment(&dir);
    let home = env.home().unwrap();
    HistoryDb::open(&home.history_file(), MaxTextBytes::default()).unwrap();
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

    for target in [
        Target::Team(team_id(0)),
        Target::Task(crate::fixtures::task(2)),
        Target::Round(round_id),
    ] {
        let mut out = Vec::new();
        execute(
            &env,
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

        let mut active_out = Vec::new();
        let mut connection = Connection::connect(&socket(&dir)).await.unwrap();
        crate::commands::status::status_active(
            &mut connection,
            Some(target.clone()),
            &Activity::new(home.clone(), env.zone()),
            &mut active_out,
        )
        .await
        .unwrap();
        let expected = String::from_utf8(active_out).unwrap();

        assert_eq!(text, expected, "target page for {target:?}");
    }
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
        let listed = teams;
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

    assert_eq!(with_task, without_task);
}
