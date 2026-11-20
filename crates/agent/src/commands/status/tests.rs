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

/// The answer of the daemon: the sample team `n` with a round of item 1.1, opened at 1 second, that `close` closes as
/// abandoned at 90 seconds or leaves open, and with `activities`.
pub(super) fn team(n: u32, close: bool, activities: Vec<agent_core::state::Activity>) -> TeamSummary {
    let state = team_state();
    let mut round = state.teams()[0].clone();
    round
        .open_round(
            "round-20261111-153001-q1w2".parse().unwrap(),
            Some("1.1".parse().unwrap()),
            at(1),
        )
        .unwrap();
    if close {
        round
            .close_round(
                &"round-20261111-153001-q1w2".parse().unwrap(),
                at(90),
                RoundEnd::Abandoned,
            )
            .unwrap();
    }
    let mut summary = TeamSummary::from(&round);
    summary.id = format!("team-20261111-1530{n:02}-t5m8").parse().unwrap();
    summary.activities = activities;
    summary
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

// ---- investigators -------------------------------------------------------------------------------------------------

/// A home in `dir` whose `agents.toml` configures the investigators `inv-gemini` and `inv-codex`, and a daemon on its socket
/// that answers `reply` to anything.
fn investigators_home(dir: &std::path::Path, reply: Reply) -> FakeDaemon {
    std::fs::write(
        dir.join("agents.toml"),
        "[agents.claude-1]\ncli = \"claude\"\n\n[investigators.inv-gemini]\ncli = \"claude\"\n\n\
         [investigators.inv-codex]\ncli = \"codex\"\n\n[tiers]\nnormal = [\"claude-1\"]\nworkers = []\n\
         [binaries]\nclaude = \"claude\"\ncodex = \"codex\"\nagentbus = \"agentbus\"\n",
    )
    .unwrap();
    FakeDaemon::serve(&dir.join("agent.sock"), move |request| {
        Answer::Lines(vec![
            Frame::Response(Response::ok(request.id, reply.clone()))
                .to_line()
                .unwrap(),
        ])
    })
}

// Rule: `--investigators` lists the configured investigators with what the daemon answers a status request for the teams.
#[tokio::test]
async fn the_investigator_view_asks_the_daemon_and_lists_the_configured_investigators() {
    let dir = tempfile::tempdir().unwrap();
    let listed = vec![team(1, true, Vec::new()), team(0, false, Vec::new())];
    let daemon = investigators_home(dir.path(), Reply::Teams { teams: listed.clone() });
    let home = Home::new(&AbsolutePath::from_str(dir.path().to_str().unwrap()).unwrap());
    let mut connection = Connection::connect(daemon.socket()).await.unwrap();
    let mut out = Vec::new();

    status_investigators(&mut connection, &home, UtcOffset::UTC, &mut out)
        .await
        .unwrap();

    assert_eq!(
        String::from_utf8(out).unwrap(),
        investigators::render(
            &listed,
            &["inv-codex".parse().unwrap(), "inv-gemini".parse().unwrap()],
            UtcOffset::UTC
        )
    );
    assert_eq!(
        daemon.requests()[0].command,
        Command::Status(StatusRequest { target: None })
    );
}

// Rule: a home whose configuration cannot be read cannot tell which investigators to show, and says so.
#[tokio::test]
async fn the_investigator_view_says_why_a_configuration_it_cannot_read_lists_nobody() {
    let dir = tempfile::tempdir().unwrap();
    let daemon = FakeDaemon::serve(&socket(&dir), |request| {
        Answer::Lines(vec![
            Frame::Response(Response::ok(request.id, Reply::Teams { teams: Vec::new() }))
                .to_line()
                .unwrap(),
        ])
    });
    let home = Home::new(&AbsolutePath::from_str(dir.path().to_str().unwrap()).unwrap());
    let mut connection = Connection::connect(daemon.socket()).await.unwrap();

    let error = status_investigators(&mut connection, &home, UtcOffset::UTC, &mut Vec::new())
        .await
        .unwrap_err();

    assert!(matches!(error, StatusError::Settings(_)), "{error:?}");
    assert!(daemon.requests().is_empty(), "the daemon is not asked");
}

// ---- rounds --------------------------------------------------------------------------------------------------------

use agent_core::config::AbsolutePath;
use std::str::FromStr;

use agent_core::ids::RoundId;
use agent_core::protocol::{RoundDetail, RoundTask};
use agent_core::state::{Findings, ItemPhase, NewRun, RoundEnd, State};

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

/// The activity of a home in `dir`, which has no runs.
fn no_activity(dir: &std::path::Path) -> Activity {
    let home = Home::new(&AbsolutePath::from_str(dir.to_str().unwrap()).unwrap());
    Activity::new(home, UtcOffset::UTC)
}

// Rule: a task that runs says when it was last active; a task that does not run says nothing of it.
#[test]
fn a_task_that_runs_says_when_it_was_last_active_and_the_others_do_not() {
    let mut teams = sample();
    teams[0].members[1].status = TaskStatus::Running;
    let said = |_: &TaskId| Some("last activity 23:10:11".to_owned());

    let text = render_active(&teams, &said);

    assert!(
        text.contains("  task-20261111-153002-k3x9  member       claude-1   running, last activity 23:10:11\n"),
        "{text}"
    );
    assert_eq!(text.matches("last activity").count(), 1, "{text}");
    // A run that has written nothing yet has no time to tell.
    let silent = render_active(&teams, &|_| None);
    assert!(
        silent.contains("  task-20261111-153002-k3x9  member       claude-1   running\n"),
        "{silent}"
    );
}

// Rule: the time is the time of day of the machine, not UTC.
#[test]
fn the_last_activity_is_told_as_the_local_time_of_day() {
    let at = SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_794_694_211);
    let warsaw = UtcOffset::from_hms(1, 0, 0).unwrap();

    assert_eq!(clock_time(at, UtcOffset::UTC), "22:10:11");
    assert_eq!(clock_time(at, warsaw), "23:10:11");
}

// Rule: the last activity of a running task is read from the output of its runs in the home.
#[tokio::test]
async fn the_status_reads_the_last_activity_of_a_running_task_from_the_home() {
    let dir = tempfile::tempdir().unwrap();
    let activity = no_activity(dir.path());
    let run_dir = dir
        .path()
        .join("tasks/task-20261111-153002-k3x9/runs/run-20261111-153005-k3x9");
    std::fs::create_dir_all(&run_dir).unwrap();
    std::fs::write(run_dir.join("out.jsonl"), "{}\n").unwrap();
    let changed = std::fs::metadata(run_dir.join("out.jsonl"))
        .unwrap()
        .modified()
        .unwrap();
    let mut teams = sample();
    teams[0].members[1].status = TaskStatus::Running;
    let listed = teams.clone();
    let daemon = FakeDaemon::serve(&socket(&dir), move |request| {
        let reply = Reply::Teams { teams: listed.clone() };
        Answer::Lines(vec![
            Frame::Response(Response::ok(request.id, reply)).to_line().unwrap(),
        ])
    });
    let mut connection = Connection::connect(daemon.socket()).await.unwrap();
    let mut out = Vec::new();

    status_active(&mut connection, None, &activity, &mut out).await.unwrap();

    let text = String::from_utf8(out).unwrap();
    let expected = format!("running, last activity {}\n", clock_time(changed, UtcOffset::UTC));
    assert!(text.contains(&expected), "{text}");
}

// Rule: `--running` shows what is being worked on: the teams with a round that goes on, and those rounds only.
#[test]
fn the_running_view_keeps_the_teams_with_an_open_round_and_their_open_rounds_only() {
    let mut state = team_with_rounds();
    state
        .team_mut(&team_id(0))
        .unwrap()
        .close_round(&round_id(1), at(41), RoundEnd::Abandoned)
        .unwrap();
    let working = TeamSummary::from(&state.teams()[0]);
    // A team whose every round is closed, and one that never had a round.
    let mut over = working.clone();
    over.id = team_id(1);
    over.rounds.retain(|round| *round.life() != RoundLife::Open);
    let mut without = working.clone();
    without.id = team_id(2);
    without.rounds.clear();

    let shown = running(vec![over, working.clone(), without]);

    assert_eq!(shown.len(), 1);
    assert_eq!(shown[0].id, working.id);
    assert_eq!(
        shown[0].rounds.iter().map(Round::id).collect::<Vec<_>>(),
        [&round_id(2)]
    );
    assert_eq!(shown[0].members, working.members);
    assert_eq!(
        render(&shown).lines().nth(1),
        Some("  round-20261111-153002-q1w2  item 1.2  open")
    );
}

// Rule: with nothing going on, `--running` says so instead of listing what is over.
#[tokio::test]
async fn the_running_view_of_a_daemon_with_no_open_round_says_so() {
    let dir = tempfile::tempdir().unwrap();
    let mut over = TeamSummary::from(&team_state().teams()[0]);
    over.rounds.clear();
    let daemon = FakeDaemon::serve(&socket(&dir), move |request| {
        let reply = Reply::Teams {
            teams: vec![over.clone()],
        };
        Answer::Lines(vec![
            Frame::Response(Response::ok(request.id, reply)).to_line().unwrap(),
        ])
    });
    let mut connection = Connection::connect(daemon.socket()).await.unwrap();
    let mut out = Vec::new();

    status_running(&mut connection, &no_activity(dir.path()), &mut out)
        .await
        .unwrap();

    assert_eq!(String::from_utf8(out).unwrap(), "no team has an open round\n");
    assert_eq!(
        daemon.requests()[0].command,
        Command::Status(StatusRequest { target: None })
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

#[test]
fn a_round_counts_only_open_findings_of_each_severity() {
    use agent_core::review_findings::{Finding, Severity};
    use agent_core::state::{FindingStatus, RecordedFinding};
    let mut state = team_with_rounds();
    {
        let mut team = state.team_mut(&team_id(0)).unwrap();
        let mut item = team.item_mut("1.2".parse().unwrap()).unwrap();
        item.start_findings();
        let mut entries = Vec::new();
        for (index, severity, status) in [
            (1, Severity::Blocker, FindingStatus::Open),
            (2, Severity::Major, FindingStatus::Open),
            (3, Severity::Minor, FindingStatus::Open),
            (4, Severity::Blocker, FindingStatus::Resolved),
        ] {
            entries.push(RecordedFinding {
                number: NonZeroU32::new(index).unwrap(),
                finding: Finding {
                    severity,
                    rule: "ERR-3".into(),
                    place: "lib.rs:1".into(),
                    text: "problem → fix".into(),
                    check: "read lib.rs:1".into(),
                },
                reporter: "inv-codex".parse().unwrap(),
                pass: NonZeroU32::MIN,
                status,
            });
        }
        *item.findings_mut().unwrap() = Findings::new(entries).unwrap();
    }
    let round = detail(&state);
    assert_eq!(round.findings.len(), 4);
    assert!(render_round(&round).contains("open findings: 1 BLOCKER, 1 MAJOR, 1 MINOR"));
}

#[test]
fn disk_line_appears_below_limit_and_not_above() {
    let dir = tempfile::tempdir().unwrap();
    let abs = agent_core::config::AbsolutePath::try_from(dir.path().to_str().unwrap().to_owned()).unwrap();
    let home = Home::new(&abs);
    let activity = Activity::new(home.clone(), time::UtcOffset::UTC);

    std::fs::write(home.free_space_file(), (50 * 1024 * 1024 * 1024u64).to_string()).unwrap();
    assert_eq!(
        activity.disk_line(),
        Some("disk: 50 GB free, limit 100 GB\n".to_owned())
    );

    std::fs::write(home.free_space_file(), (150 * 1024 * 1024 * 1024u64).to_string()).unwrap();
    assert_eq!(activity.disk_line(), None);
}

// ---- the reviewer's line -------------------------------------------------------------------------------------------

use agent_core::state::{ActivityEnd, ActivityOutcome, ActivityRequest, RunEnd};

/// An activity of `investigator` in task `n`, in the round of item 1.1, asked `asked` seconds after 19:15:00 UTC, whose
/// outcome is `outcome`.
fn review_activity(
    investigator: &str,
    n: u32,
    asked: u32,
    request: &ActivityRequest,
    outcome: &ActivityOutcome,
) -> agent_core::state::Activity {
    serde_json::from_value(serde_json::json!({
        "investigator": investigator,
        "task": task(n),
        "rounds": ["round-20261111-153001-q1w2"],
        "request": request,
        "asked_at": at(asked),
        "outcome": outcome,
    }))
    .unwrap()
}

/// The line of the reviewer task `n`, whose recorded activities are `activities`, in the status of the sample team with
/// those activities in its round of item 1.1, whose task `n` `runs` when it does; a task that runs is printed with
/// `last activity 23:10:11`.
fn reviewer_line(n: u32, activities: Vec<agent_core::state::Activity>) -> String {
    reviewer_line_of(n, activities, None)
}

/// [`reviewer_line`] for the reviewer whose task runs at `running`.
fn reviewer_line_of(n: u32, activities: Vec<agent_core::state::Activity>, running: Option<u32>) -> String {
    let mut teams = sample();
    teams[0].activities = activities;
    for member in &mut teams[0].members {
        if Some(member.task.clone()) == running.map(task) {
            member.status = TaskStatus::Running;
        }
    }
    let said = |_: &TaskId| Some("last activity 23:10:11".to_owned());
    let text = render_active(&teams, &said);
    let prefix = format!("  {}  reviewer", task(n));
    text.lines()
        .find(|line| line.starts_with(&prefix))
        .map_or_else(|| panic!("the reviewer {n} has a line: {text}"), str::to_owned)
}

// Rule: between its reviews a reviewer's line says what it did, with the ordinal of the review it gave for the round;
// a reviewer whose work is going or queued says that, with when it was last active; a stopped or failed one says when
// and why.
#[test]
fn a_reviewers_line_says_what_it_did_or_does_rather_than_that_it_waits() {
    let finished = |at| ActivityOutcome::Finished {
        at,
        end: ActivityEnd::Reported,
    };
    let tables: Vec<(Vec<agent_core::state::Activity>, Option<u32>, &str)> = vec![
        (
            vec![
                review_activity("inv-codex", 4, 20, &ActivityRequest::FirstReview, &finished(at(30))),
                review_activity(
                    "inv-codex",
                    4,
                    40,
                    &ActivityRequest::Verification { pass: NonZeroU32::MIN },
                    &finished(at(50)),
                ),
            ],
            None,
            "  task-20261111-153004-k3x9  reviewer     inv-codex  done, 2nd review finished 19:15:50",
        ),
        (
            vec![review_activity(
                "inv-codex",
                4,
                20,
                &ActivityRequest::FirstReview,
                &finished(at(30)),
            )],
            None,
            "  task-20261111-153004-k3x9  reviewer     inv-codex  done, 1st review finished 19:15:30",
        ),
        (
            vec![review_activity(
                "inv-codex",
                4,
                20,
                &ActivityRequest::FirstReview,
                &ActivityOutcome::Working,
            )],
            Some(4),
            "  task-20261111-153004-k3x9  reviewer     inv-codex  working, last activity 23:10:11",
        ),
        (
            vec![review_activity(
                "inv-codex",
                4,
                20,
                &ActivityRequest::FirstReview,
                &ActivityOutcome::Finished {
                    at: at(30),
                    end: ActivityEnd::Stopped,
                },
            )],
            None,
            "  task-20261111-153004-k3x9  reviewer     inv-codex  stopped at 19:15:30, reason: the human stopped it",
        ),
        (
            vec![review_activity(
                "inv-codex",
                4,
                20,
                &ActivityRequest::FirstReview,
                &ActivityOutcome::Finished {
                    at: at(30),
                    end: ActivityEnd::Failed {
                        reason: Some(RunEnd::Hung),
                    },
                },
            )],
            None,
            "  task-20261111-153004-k3x9  reviewer     inv-codex  stopped at 19:15:30, reason: the run showed no activity for too long",
        ),
    ];
    for (activities, running, line) in tables {
        // The task of the reviewer waits between its reviews: its status says nothing of what it did.
        assert_eq!(reviewer_line_of(4, activities, running), line);
    }
}

// Rule: the ordinal counts the reviews that the investigator gave for the round: a review of another round, or of
// another investigator, is not counted.
#[test]
fn the_ordinal_of_a_review_counts_only_the_reviews_this_investigator_gave_for_the_round() {
    let finished = |at| ActivityOutcome::Finished {
        at,
        end: ActivityEnd::Reported,
    };
    let other_round = serde_json::from_value(serde_json::json!({
        "investigator": "inv-codex",
        "task": task(5),
        "rounds": ["round-20261111-153002-q1w2"],
        "request": {"kind": "first_review"},
        "asked_at": at(10),
        "outcome": {"status": "finished", "at": at(15), "end": {"kind": "reported"}},
    }))
    .unwrap();
    let activities = vec![
        other_round,
        review_activity("inv-codex", 4, 20, &ActivityRequest::FirstReview, &finished(at(30))),
        review_activity("inv-gemini", 5, 25, &ActivityRequest::FirstReview, &finished(at(35))),
    ];

    assert_eq!(
        reviewer_line(4, activities),
        "  task-20261111-153004-k3x9  reviewer     inv-codex  done, 1st review finished 19:15:30"
    );
}

// Rule: the ordinal of a review is an English ordinal: the suffix is `st`, `nd`, `rd` or `th` — `th` for 11, 12 and 13,
// and for every number whose last digit is 0 or 4 to 9.
#[test]
fn the_ordinal_of_a_review_is_an_english_ordinal() {
    let cases: [(u32, &str); 16] = [
        (1, "1st"),
        (2, "2nd"),
        (3, "3rd"),
        (4, "4th"),
        (5, "5th"),
        (10, "10th"),
        (11, "11th"),
        (12, "12th"),
        (13, "13th"),
        (14, "14th"),
        (21, "21st"),
        (22, "22nd"),
        (23, "23rd"),
        (24, "24th"),
        (111, "111th"),
        (112, "112th"),
    ];
    for (count, text) in cases {
        // The `count` reviews, the last one the newest and the one the line tells, reported one second apart.
        let activities: Vec<_> = (0..count)
            .map(|index| {
                review_activity(
                    "inv-codex",
                    4,
                    20 + index,
                    &ActivityRequest::FirstReview,
                    &ActivityOutcome::Finished {
                        at: at(20 + index),
                        end: ActivityEnd::Reported,
                    },
                )
            })
            .collect();
        let line = reviewer_line(4, activities);
        let said: Vec<&str> = line.split_whitespace().skip(3).collect();
        assert_eq!(
            (said.first(), said.get(1), said.get(2)),
            (Some(&"done,"), Some(&text), Some(&"review")),
            "{line}"
        );
    }
}

// Rule: a review that ended without a report — a second opinion abandoned, or a failure with no reason the daemon
// knows — says that it stopped, and why.
#[test]
fn a_review_that_ended_without_a_report_says_when_and_why_it_stopped() {
    for (end, reason) in [
        (
            ActivityEnd::OpinionFailed {
                reason: agent_core::state::OpinionFailure::MissingReport,
            },
            "its second opinion was abandoned: the corrections ran out without an opinion",
        ),
        (ActivityEnd::Failed { reason: None }, "it failed"),
    ] {
        let activities = vec![review_activity(
            "inv-codex",
            4,
            20,
            &ActivityRequest::FirstReview,
            &ActivityOutcome::Finished { at: at(30), end },
        )];

        assert_eq!(
            reviewer_line(4, activities),
            format!("  task-20261111-153004-k3x9  reviewer     inv-codex  stopped at 19:15:30, reason: {reason}")
        );
    }
}

// Rule: a reviewer whose review is queued, and whose task does not run, says `working` without a last activity.
#[test]
fn a_reviewer_whose_task_does_not_run_says_working_without_a_last_activity() {
    let activities = vec![review_activity(
        "inv-codex",
        4,
        20,
        &ActivityRequest::FirstReview,
        &ActivityOutcome::Working,
    )];

    assert_eq!(
        reviewer_line(4, activities),
        "  task-20261111-153004-k3x9  reviewer     inv-codex  working"
    );
}

// Rule: a failure says the end of the run the daemon kept, in the words of its status.
#[test]
fn a_review_that_failed_says_the_end_of_its_run() {
    for (end, reason) in [
        (
            RunEnd::NoStatus { exit_code: Some(1) },
            "the run ended without a status",
        ),
        (
            RunEnd::QuotaExhausted { resets_at: None },
            "the account's quota was used up",
        ),
        (RunEnd::RateLimited, "the provider limited the rate of requests"),
        (RunEnd::TurnLimit, "the run used up its turn budget"),
        (RunEnd::Stopped, "the run was stopped"),
    ] {
        let activities = vec![review_activity(
            "inv-codex",
            4,
            20,
            &ActivityRequest::FirstReview,
            &ActivityOutcome::Finished {
                at: at(30),
                end: ActivityEnd::Failed { reason: Some(end) },
            },
        )];

        assert_eq!(
            reviewer_line(4, activities),
            format!("  task-20261111-153004-k3x9  reviewer     inv-codex  stopped at 19:15:30, reason: {reason}")
        );
    }
}

// Rule: a reviewer with no recorded activity keeps the line it had.
#[test]
fn a_reviewer_without_a_recorded_activity_keeps_the_line_of_its_status() {
    assert_eq!(
        reviewer_line(4, Vec::new()),
        "  task-20261111-153004-k3x9  reviewer     inv-codex  queued"
    );
}

// Rule: the lines of the coordinator, the members and the workers are those of their status, also when the team has
// recorded activities of them.
#[test]
fn the_lines_of_the_crew_and_the_workers_do_not_change() {
    let mut teams = sample();
    teams[0].activities = vec![
        review_activity(
            "inv-opus",
            1,
            20,
            &ActivityRequest::TeamMember,
            &ActivityOutcome::Working,
        ),
        review_activity(
            "claude-1",
            2,
            20,
            &ActivityRequest::TeamMember,
            &ActivityOutcome::Working,
        ),
    ];

    let text = render_active(&teams, &|_| None);

    assert_eq!(
        text,
        "\
team-20261111-153000-t5m8  team-agents  autopilot  2 pending, 0 in progress, 0 in review, 0 approved, 0 done
  task-20261111-153001-k3x9  coordinator  inv-opus   queued
  task-20261111-153002-k3x9  member       claude-1   queued
  task-20261111-153003-k3x9  worker       glm-1      queued
  task-20261111-153004-k3x9  reviewer     inv-codex  queued
"
    );
}

// Rule: the ordinal of a reviewer's line counts the reviews the investigator gave: an earlier review of the round that
// ended without its report is not one of them.
#[test]
fn a_reviewers_ordinal_leaves_out_a_review_that_ended_without_its_report() {
    let activities = vec![
        review_activity(
            "inv-codex",
            4,
            20,
            &ActivityRequest::FirstReview,
            &ActivityOutcome::Finished {
                at: at(25),
                end: ActivityEnd::Stopped,
            },
        ),
        review_activity(
            "inv-codex",
            4,
            40,
            &ActivityRequest::Verification { pass: NonZeroU32::MIN },
            &ActivityOutcome::Finished {
                at: at(50),
                end: ActivityEnd::Reported,
            },
        ),
    ];

    assert_eq!(
        reviewer_line(4, activities),
        "  task-20261111-153004-k3x9  reviewer     inv-codex  done, 1st review finished 19:15:50"
    );
}

// Rule: a first review and the verification after it may be asked of one task in the same second: the verification is
// the later review, so the reviewer's line tells it, and it is the second review the investigator gave.
#[test]
fn a_verification_asked_in_the_second_of_the_first_review_is_its_second_review() {
    let activities = vec![
        review_activity(
            "inv-codex",
            4,
            20,
            &ActivityRequest::FirstReview,
            &ActivityOutcome::Finished {
                at: at(30),
                end: ActivityEnd::Reported,
            },
        ),
        review_activity(
            "inv-codex",
            4,
            20,
            &ActivityRequest::Verification { pass: NonZeroU32::MIN },
            &ActivityOutcome::Finished {
                at: at(40),
                end: ActivityEnd::Reported,
            },
        ),
    ];

    assert_eq!(
        reviewer_line(4, activities),
        "  task-20261111-153004-k3x9  reviewer     inv-codex  done, 2nd review finished 19:15:40"
    );
}

// Rule: an abandoned second opinion says the reason the daemon recorded for it.
#[test]
fn an_abandoned_second_opinion_says_the_reason_the_daemon_recorded() {
    use agent_core::state::OpinionFailure;
    for (failure, reason) in [
        (OpinionFailure::Failed, "the run failed"),
        (OpinionFailure::NoStatus, "the run ended without a status"),
        (
            OpinionFailure::MissingReport,
            "the corrections ran out without an opinion",
        ),
        (OpinionFailure::Silent, "the investigator stayed silent too long"),
        (OpinionFailure::Interrupted, "the task can no longer be resumed"),
    ] {
        let activities = vec![review_activity(
            "inv-codex",
            4,
            20,
            &ActivityRequest::SecondOpinion {
                pass: NonZeroU32::MIN,
                finding: NonZeroU32::MIN,
            },
            &ActivityOutcome::Finished {
                at: at(30),
                end: ActivityEnd::OpinionFailed { reason: failure },
            },
        )];

        assert_eq!(
            reviewer_line(4, activities),
            format!(
                "  task-20261111-153004-k3x9  reviewer     inv-codex  stopped at 19:15:30, reason: its second opinion was abandoned: {reason}"
            )
        );
    }
}

#[test]
fn delivery_round_status_names_the_owning_pr_while_waiting_and_after_completion() {
    let mut state = team_with_rounds();
    let mut team = state.team_mut(&team_id(0)).unwrap();
    let pr = crate::fixtures::pr(7);
    team.item_mut("1.1".parse().unwrap())
        .unwrap()
        .advance(ItemPhase::InReview {
            pr: pr.clone(),
            reviews: Vec::new(),
        })
        .unwrap();
    team.deliver_round(&round_id(2), pr.number).unwrap();
    assert_eq!(
        round_line(team.round(&round_id(2)).unwrap()),
        "round-20261111-153002-q1w2  item 1.2  delivered by PR #7, waits for its merge"
    );
    team.complete_delivery(&round_id(2), at(40)).unwrap();
    assert_eq!(
        round_line(team.round(&round_id(2)).unwrap()),
        "round-20261111-153002-q1w2  item 1.2  done, delivered by PR #7"
    );
}

#[test]
fn a_delivery_round_abandoned_by_a_stop_still_names_the_owning_pr() {
    let mut state = team_with_rounds();
    let mut team = state.team_mut(&team_id(0)).unwrap();
    let pr = crate::fixtures::pr(7);
    team.item_mut("1.1".parse().unwrap())
        .unwrap()
        .advance(ItemPhase::InReview {
            pr: pr.clone(),
            reviews: Vec::new(),
        })
        .unwrap();
    team.deliver_round(&round_id(2), pr.number).unwrap();
    team.close_round(&round_id(2), at(30), agent_core::state::RoundEnd::Abandoned)
        .unwrap();
    assert_eq!(
        round_line(team.round(&round_id(2)).unwrap()),
        "round-20261111-153002-q1w2  item 1.2  closed, abandoned, delivered by PR #7"
    );
}

#[test]
fn an_ended_delivery_status_distinguishes_failure_from_waiting() {
    let mut state = team_with_rounds();
    let mut team = state.team_mut(&team_id(0)).unwrap();
    let pr = crate::fixtures::pr(7);
    team.item_mut("1.1".parse().unwrap())
        .unwrap()
        .advance(ItemPhase::InReview {
            pr: pr.clone(),
            reviews: Vec::new(),
        })
        .unwrap();
    team.deliver_round(&round_id(2), pr.number).unwrap();
    team.end_delivery(&round_id(2), at(40)).unwrap();
    assert_eq!(
        round_line(team.round(&round_id(2)).unwrap()),
        "round-20261111-153002-q1w2  item 1.2  closed, abandoned, delivery PR #7 closed without merging"
    );
    team.open_round(round_id(3), Some("1.2".parse().unwrap()), at(41))
        .unwrap();
    assert_eq!(
        round_line(team.round(&round_id(3)).unwrap()),
        "round-20261111-153003-q1w2  item 1.2  open"
    );
}
