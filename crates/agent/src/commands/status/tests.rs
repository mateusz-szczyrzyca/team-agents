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

// ---- rounds --------------------------------------------------------------------------------------------------------

use agent_core::config::AbsolutePath;
use agent_core::ids::RoundId;
use agent_core::protocol::{RoundDetail, RoundTask};
use agent_core::state::{ItemPhase, NewRun, RoundEnd, State};

use crate::fixtures::{at, run};

fn round_id(n: u32) -> RoundId {
    format!("round-20261111-1530{n:02}-q1w2").parse().unwrap()
}

/// The sample team with two rounds: 1.1 merged and 1.2 in progress with a run of the member, and the member's task.
fn team_with_rounds() -> State {
    let mut state = team_state();
    {
        let mut team = state.team_mut(&team_id(0)).unwrap();
        team.open_round(round_id(1), Some("1.1".parse().unwrap()), at(10))
            .unwrap();
        team.open_round(round_id(2), Some("1.2".parse().unwrap()), at(20))
            .unwrap();
        let mut first = team.item_mut("1.1".parse().unwrap()).unwrap();
        first
            .advance(ItemPhase::InProgress {
                assignee: "claude-1".parse().unwrap(),
                branch: "agents/claude-1/task-20261111-153002-k3x9-1.1".parse().unwrap(),
            })
            .unwrap();
        let mut second = team.item_mut("1.2".parse().unwrap()).unwrap();
        second
            .advance(ItemPhase::InProgress {
                assignee: "claude-1".parse().unwrap(),
                branch: "agents/claude-1/task-20261111-153002-k3x9-1.2".parse().unwrap(),
            })
            .unwrap();
        team.begin_run(
            &task(2),
            NewRun {
                id: run(1),
                at: at(21),
                session: None,
                rounds: vec![round_id(2)],
            },
        )
        .unwrap();
    }
    state
}

#[test]
fn a_team_lists_its_rounds_under_its_line_with_their_items_and_how_they_ended() {
    let mut state = team_with_rounds();
    // Item 1.1 goes on to its merge, and its round ends there.
    {
        let mut team = state.team_mut(&team_id(0)).unwrap();
        let pr = crate::fixtures::pr(7);
        let mut item = team.item_mut("1.1".parse().unwrap()).unwrap();
        item.advance(ItemPhase::InReview {
            pr: pr.clone(),
            reviews: Vec::new(),
        })
        .unwrap();
        item.ask_reviews(vec![
            crate::fixtures::review("inv-codex", 4),
            crate::fixtures::review("inv-gemini", 5),
        ])
        .unwrap();
        for reviewer in [4, 5] {
            item.submit_review(&task(reviewer), agent_core::status::ReviewVerdict::Approve, at(30))
                .unwrap();
        }
        let ItemPhase::InReview { reviews, .. } = item.phase().clone() else {
            panic!("in review")
        };
        item.advance(ItemPhase::Approved {
            pr: pr.clone(),
            reviews,
        })
        .unwrap();
        item.advance(ItemPhase::Done {
            completion: agent_core::state::Completion::Merged { pr, at: at(40) },
        })
        .unwrap();
        team.close_round(&round_id(1), at(41), RoundEnd::Merged).unwrap();
    }

    let text = render(&[TeamSummary::from(&state.teams()[0])]);

    assert_eq!(
        text,
        "\
team-20261111-153000-t5m8  team-agents  autopilot  0 pending, 1 in progress, 0 in review, 0 approved, 1 done
  round-20261111-153001-q1w2  item 1.1  closed, merged
  round-20261111-153002-q1w2  item 1.2  open
  task-20261111-153001-k3x9  coordinator  inv-opus   queued
  task-20261111-153002-k3x9  member       claude-1   queued
  task-20261111-153003-k3x9  worker       glm-1      queued
  task-20261111-153004-k3x9  reviewer     inv-codex  queued
"
    );
}

#[test]
fn a_round_is_one_line_and_the_round_of_a_whole_task_list_has_no_item() {
    let state = team_with_rounds();
    let mut whole = agent_core::state::Team::new(
        team_id(1),
        crate::fixtures::repo(),
        at(0),
        agent_core::state::MergeMode::Autopilot,
        agent_core::state::TaskListText::new("Tidy up.\n".to_owned()).unwrap(),
        agent_core::state::Crew::solo(crate::fixtures::member(9, crate::fixtures::normal("claude-1"))).unwrap(),
    );
    whole.open_round(round_id(9), None, at(1)).unwrap();
    whole.close_round(&round_id(9), at(2), RoundEnd::Abandoned).unwrap();

    assert_eq!(
        round_line(&state.teams()[0].rounds()[0]),
        "round-20261111-153001-q1w2  item 1.1  open"
    );
    assert_eq!(
        round_line(&whole.rounds()[0]),
        "round-20261111-153009-q1w2  whole task list  closed, abandoned"
    );
}

fn detail(state: &State) -> RoundDetail {
    let team = &state.teams()[0];
    let round = team.round(&round_id(2)).unwrap();
    let (_, member) = team.find(&task(2)).unwrap();
    let tasks = vec![RoundTask {
        task: task(2),
        role: agent_core::kinds::Role::Member,
        agent: member.agent().clone(),
        worktree: AbsolutePath::try_from("/srv/agents/worktrees/team-agents/task-20261111-153002-k3x9".to_owned())
            .unwrap(),
        branch: "agents/claude-1/task-20261111-153002-k3x9-1.2".parse().unwrap(),
        runs: member.runs().to_vec(),
    }];
    RoundDetail::new(team, round, tasks)
}

#[test]
fn a_round_is_shown_with_its_item_its_tasks_with_where_they_work_and_their_runs() {
    let state = team_with_rounds();

    let text = render_round(&detail(&state));

    assert_eq!(
        text,
        "\
round-20261111-153002-q1w2  item 1.2  open  team team-20261111-153000-t5m8
  opened 2026-11-11T19:15:20Z
  item 1.2: The second item (in_progress)
  task-20261111-153002-k3x9  member  claude-1  /srv/agents/worktrees/team-agents/task-20261111-153002-k3x9  agents/claude-1/task-20261111-153002-k3x9-1.2
    run-20261111-153001-r7q2  started 2026-11-11T19:15:21Z  running
"
    );
}

#[test]
fn a_closed_round_says_when_and_how_it_ended_and_an_item_in_review_shows_its_pull_request_and_review_passes() {
    let mut state = team_with_rounds();
    {
        let mut team = state.team_mut(&team_id(0)).unwrap();
        let mut item = team.item_mut("1.2".parse().unwrap()).unwrap();
        item.advance(ItemPhase::InReview {
            pr: crate::fixtures::pr(7),
            reviews: Vec::new(),
        })
        .unwrap();
        item.ask_reviews(vec![crate::fixtures::review("inv-codex", 4)]).unwrap();
        team.close_round(&round_id(2), at(50), RoundEnd::Abandoned).unwrap();
    }

    let text = render_round(&detail(&state));

    assert!(text.contains("  closed 2026-11-11T19:15:50Z, abandoned\n"), "{text}");
    assert!(
        text.contains("  pull request #7 on agents/codex-1/item-1-2\n"),
        "{text}"
    );
    assert!(text.contains("  review pass 1: inv-codex pending\n"), "{text}");
    assert!(
        text.contains("round-20261111-153002-q1w2  item 1.2  closed, abandoned  team "),
        "{text}"
    );
}

#[tokio::test]
async fn the_status_of_a_round_asks_the_daemon_to_show_it_and_writes_what_it_shows() {
    let dir = tempfile::tempdir().unwrap();
    let state = team_with_rounds();
    let shown = detail(&state);
    let sent = shown.clone();
    let daemon = FakeDaemon::serve(&socket(&dir), move |request| {
        let reply = Reply::Round {
            round: Box::new(sent.clone()),
        };
        Answer::Lines(vec![
            Frame::Response(Response::ok(request.id, reply)).to_line().unwrap(),
        ])
    });
    let mut connection = Connection::connect(daemon.socket()).await.unwrap();
    let mut out = Vec::new();

    status(&mut connection, Some(Target::Round(round_id(2))), &mut out)
        .await
        .unwrap();

    assert_eq!(String::from_utf8(out).unwrap(), render_round(&shown));
    assert_eq!(
        daemon.requests()[0].command,
        Command::Show(agent_core::protocol::ShowRequest {
            target: Target::Round(round_id(2))
        })
    );
}

#[tokio::test]
async fn another_kind_of_reply_to_the_status_of_a_round_is_an_error() {
    let dir = tempfile::tempdir().unwrap();
    let daemon = FakeDaemon::serve(&socket(&dir), |request| {
        Answer::Lines(vec![
            Frame::Response(Response::ok(request.id, Reply::Applied { tasks: Vec::new() }))
                .to_line()
                .unwrap(),
        ])
    });
    let mut connection = Connection::connect(daemon.socket()).await.unwrap();

    let error = status(&mut connection, Some(Target::Round(round_id(2))), &mut Vec::new())
        .await
        .unwrap_err();

    assert!(
        matches!(error, StatusError::UnexpectedReply { kind: "applied" }),
        "{error:?}"
    );
}
