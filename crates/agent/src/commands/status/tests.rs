use std::num::NonZeroU32;

use agent_core::protocol::{Failure, FailureKind, Frame, MemberSummary, Response, TeamSummary};

use super::*;
use crate::fixtures::{task, team_id, team_state};
use crate::testing::{Answer, FakeDaemon};

fn sample() -> Vec<TeamSummary> {
    vec![TeamSummary::from(&team_state().teams()[0])]
}

#[test]
fn no_teams_are_said_so() {
    assert_eq!(render(&[]), "no teams\n");
}

#[test]
fn a_team_is_a_line_and_each_of_its_tasks_a_line_under_it() {
    assert_eq!(
        render(&sample()),
        "\
team-20261111-153000-t5m8  team-agents  autopilot  2 pending, 0 in progress, 0 in review, 0 approved, 0 done
  task-20261111-153001-k3x9  coordinator  inv-opus   queued
  task-20261111-153002-k3x9  member       claude-1   queued
  task-20261111-153003-k3x9  worker       glm-1      queued
  task-20261111-153004-k3x9  reviewer     inv-codex  queued
"
    );
}

#[test]
fn a_task_with_questions_says_which_round_waits() {
    let mut teams = sample();
    let member: &mut MemberSummary = &mut teams[0].members[1];
    member.open_question = NonZeroU32::new(2);

    let text = render(&teams);

    assert!(
        text.contains(
            "  task-20261111-153002-k3x9  member       claude-1   queued, questions number 2 wait for answers\n"
        ),
        "{text}"
    );
}

#[test]
fn teams_are_separated_by_nothing_but_their_own_lines() {
    let mut teams = sample();
    let mut second = teams[0].clone();
    second.id = team_id(1);
    teams.push(second);

    let text = render(&teams);

    assert_eq!(text.lines().filter(|line| line.starts_with("team-")).count(), 2);
    assert_eq!(text.lines().count(), 10);
}

fn socket(dir: &tempfile::TempDir) -> std::path::PathBuf {
    dir.path().join("agent.sock")
}

#[tokio::test]
async fn the_status_is_what_the_daemon_lists() {
    let dir = tempfile::tempdir().unwrap();
    let teams = sample();
    let listed = teams.clone();
    let daemon = FakeDaemon::serve(&socket(&dir), move |request| {
        let reply = Reply::Teams { teams: listed.clone() };
        Answer::Lines(vec![
            Frame::Response(Response::ok(request.id, reply)).to_line().unwrap(),
        ])
    });
    let mut connection = Connection::connect(daemon.socket()).await.unwrap();
    let mut out = Vec::new();

    status(&mut connection, None, &mut out).await.unwrap();

    assert_eq!(String::from_utf8(out).unwrap(), render(&teams));
    let sent = daemon.requests();
    assert_eq!(sent[0].command, Command::Status(StatusRequest { target: None }));
}

#[tokio::test]
async fn a_target_is_sent_to_the_daemon() {
    let dir = tempfile::tempdir().unwrap();
    let daemon = FakeDaemon::serve(&socket(&dir), |request| {
        Answer::Lines(vec![
            Frame::Response(Response::ok(request.id, Reply::Teams { teams: Vec::new() }))
                .to_line()
                .unwrap(),
        ])
    });
    let mut connection = Connection::connect(daemon.socket()).await.unwrap();
    let target = Target::Task(task(2));

    status(&mut connection, Some(target.clone()), &mut Vec::new())
        .await
        .unwrap();

    assert_eq!(
        daemon.requests()[0].command,
        Command::Status(StatusRequest { target: Some(target) })
    );
}

#[tokio::test]
async fn another_kind_of_reply_is_an_error() {
    let dir = tempfile::tempdir().unwrap();
    let daemon = FakeDaemon::serve(&socket(&dir), |request| {
        let reply = Reply::Applied { tasks: Vec::new() };
        Answer::Lines(vec![
            Frame::Response(Response::ok(request.id, reply)).to_line().unwrap(),
        ])
    });
    let mut connection = Connection::connect(daemon.socket()).await.unwrap();

    let error = status(&mut connection, None, &mut Vec::new()).await.unwrap_err();

    assert!(
        matches!(error, StatusError::UnexpectedReply { kind: "applied" }),
        "{error:?}"
    );
}

#[tokio::test]
async fn a_refusal_of_the_daemon_is_the_error_of_the_command() {
    let dir = tempfile::tempdir().unwrap();
    let daemon = FakeDaemon::serve(&socket(&dir), |request| {
        let failure = Failure {
            kind: FailureKind::NotFound,
            message: "no such task".to_owned(),
        };
        Answer::Lines(vec![
            Frame::Response(Response::error(request.id, failure)).to_line().unwrap(),
        ])
    });
    let mut connection = Connection::connect(daemon.socket()).await.unwrap();

    let error = status(&mut connection, None, &mut Vec::new()).await.unwrap_err();

    assert!(
        matches!(&error, StatusError::Client(ClientError::Refused(failure)) if failure.kind == FailureKind::NotFound),
        "{error:?}"
    );
}

struct ClosedPipe;

impl Write for ClosedPipe {
    fn write(&mut self, _: &[u8]) -> io::Result<usize> {
        Err(io::Error::from(io::ErrorKind::BrokenPipe))
    }

    fn flush(&mut self) -> io::Result<()> {
        Err(io::Error::from(io::ErrorKind::BrokenPipe))
    }
}

#[tokio::test]
async fn a_closed_output_is_an_error_not_a_panic() {
    let dir = tempfile::tempdir().unwrap();
    let teams = sample();
    let daemon = FakeDaemon::serve(&socket(&dir), move |request| {
        let reply = Reply::Teams { teams: teams.clone() };
        Answer::Lines(vec![
            Frame::Response(Response::ok(request.id, reply)).to_line().unwrap(),
        ])
    });
    let mut connection = Connection::connect(daemon.socket()).await.unwrap();

    let error = status(&mut connection, None, &mut ClosedPipe).await.unwrap_err();

    assert!(
        matches!(&error, StatusError::Output(e) if e.kind() == io::ErrorKind::BrokenPipe),
        "{error:?}"
    );
}
