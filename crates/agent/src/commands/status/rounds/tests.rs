use std::num::NonZeroU32;
use std::path::Path;
use std::str::FromStr;

use agent_core::config::AbsolutePath;
use agent_core::history::{
    EventRecord, HistoryEvent, ItemText, MaxTextBytes, Record, RoundOpening, RoundSubject, TeamStart,
};
use agent_core::protocol::{Command, Frame, Reply, Response, RoundDetail, StatusRequest, Target};
use agent_core::state::{ItemPhase, MergeMode, PrNumber, RoundEnd, State};
use agent_runtime::history::{HistoryDb, Store};
use agent_runtime::store::Home;

use super::super::{render_with, status_active, status_running};
use super::*;
use crate::fixtures::{at, team_id, team_state};
use crate::testing::{Answer, FakeDaemon};

/// Warsaw in winter: an hour ahead of UTC.
fn warsaw() -> UtcOffset {
    UtcOffset::from_hms(1, 0, 0).unwrap()
}

fn round_id(n: u32) -> RoundId {
    format!("round-20261111-1530{n:02}-q1w2").parse().unwrap()
}

/// The sample team with the round 1 for item 1.1, abandoned 31 seconds after it opened, and the round 2 for item
/// 1.2, which the member works on.
fn rounds_state() -> State {
    let mut state = team_state();
    {
        let mut team = state.team_mut(&team_id(0)).unwrap();
        team.open_round(round_id(1), Some("1.1".parse().unwrap()), at(10))
            .unwrap();
        team.open_round(round_id(2), Some("1.2".parse().unwrap()), at(20))
            .unwrap();
        team.item_mut("1.2".parse().unwrap())
            .unwrap()
            .advance(ItemPhase::InProgress {
                assignee: "claude-1".parse().unwrap(),
                branch: "agents/claude-1/task-20261111-153002-k3x9-1.2".parse().unwrap(),
            })
            .unwrap();
        team.close_round(&round_id(1), at(41), RoundEnd::Abandoned).unwrap();
    }
    state
}

/// The round `n` of [`rounds_state`].
fn round_of(state: &State, n: u32) -> Round {
    state.teams()[0].round(&round_id(n)).unwrap().clone()
}

/// A round opened `opened` seconds after 19:15:00 UTC on 2026-11-11 and closed `length` seconds later.
fn closed_after(opened: u32, length: u32) -> Round {
    let mut state = team_state();
    let mut team = state.team_mut(&team_id(0)).unwrap();
    team.open_round(round_id(1), Some("1.1".parse().unwrap()), at(opened))
        .unwrap();
    team.close_round(&round_id(1), at(opened + length), RoundEnd::Abandoned)
        .unwrap();
    team.round(&round_id(1)).unwrap().clone()
}

// Rule: a round says when it started, as the day and the minute in local time.
#[test]
fn a_round_that_goes_on_says_where_its_work_is_and_when_it_started() {
    let round = round_of(&rounds_state(), 2);

    assert_eq!(
        suffix(&round, Some(RoundWork::PullDraft), warsaw()),
        ", PR draft  started 11-11-2026 20:15"
    );
    assert_eq!(
        suffix(&round, Some(RoundWork::OnlyLocal), UtcOffset::UTC),
        ", only local  started 11-11-2026 19:15"
    );
    assert_eq!(suffix(&round, None, UtcOffset::UTC), "  started 11-11-2026 19:15");
}

#[test]
fn a_round_that_is_closed_says_when_it_started_and_how_long_it_took() {
    let round = round_of(&rounds_state(), 1);

    assert_eq!(suffix(&round, None, warsaw()), "  started 11-11-2026 20:15, took 31s");
}

// Rule: the day is the local one: a round opened late in the evening in UTC started the next day in Warsaw.
#[test]
fn the_day_a_round_started_is_the_local_day() {
    // 23:30:00 UTC.
    let round = closed_after(4 * 3600 + 15 * 60, 60);

    assert_eq!(
        suffix(&round, None, UtcOffset::UTC),
        "  started 11-11-2026 23:30, took 1m"
    );
    assert_eq!(suffix(&round, None, warsaw()), "  started 12-11-2026 00:30, took 1m");
}

#[test]
fn how_long_a_round_took_is_told_in_days_hours_and_minutes_or_in_seconds_under_a_minute() {
    let cases: [(u64, &str); 9] = [
        (0, "0s"),
        (59, "59s"),
        (60, "1m"),
        (3599, "59m"),
        (3600, "1h 00m"),
        (10_860, "3h 01m"),
        (86_399, "23h 59m"),
        (86_400, "1d 00h 00m"),
        (97_260, "1d 03h 01m"),
    ];

    for (seconds, text) in cases {
        assert_eq!(took(seconds), text, "{seconds}");
    }
}

#[test]
fn what_is_said_of_a_round_follows_its_line() {
    let teams = vec![TeamSummary::from(&rounds_state().teams()[0])];

    let text = render_with(&teams, &|_| None, &|round| format!(" <{}>", round.id()));

    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(
        lines[1..3],
        [
            "  round-20261111-153001-q1w2  item 1.1  closed, abandoned <round-20261111-153001-q1w2>",
            "  round-20261111-153002-q1w2  item 1.2  open <round-20261111-153002-q1w2>",
        ]
    );
}

// ---- with a daemon and a history -------------------------------------------------------------------------------------

/// The home in `dir`.
fn home(dir: &Path) -> Home {
    Home::new(&AbsolutePath::from_str(dir.to_str().unwrap()).unwrap())
}

/// A daemon that lists the team of `state` and shows its rounds.
fn daemon(dir: &Path, state: &State) -> FakeDaemon {
    let team = state.teams()[0].clone();
    FakeDaemon::serve(&dir.join("agent.sock"), move |request| {
        let reply = match &request.command {
            Command::Status(_) => Reply::Teams {
                teams: vec![TeamSummary::from(&team)],
            },
            Command::Show(show) => {
                let Target::Round(id) = &show.target else {
                    panic!("only rounds are shown: {show:?}");
                };
                Reply::Round {
                    round: Box::new(RoundDetail::new(&team, team.round(id).unwrap(), Vec::new())),
                }
            }
            other => panic!("the status asks for teams and rounds only: {other:?}"),
        };
        Answer::Lines(vec![
            Frame::Response(Response::ok(request.id, reply)).to_line().unwrap(),
        ])
    })
}

/// Writes a history into the home in `dir` that knows the round 2 of [`rounds_state`] and `events` of it.
fn history(dir: &Path, events: Vec<HistoryEvent>) {
    let mut records = vec![
        Record::Team(Box::new(TeamStart {
            team: team_id(0),
            at: at(0),
            repo: "team-agents".to_owned(),
            base_branch: "main".parse().unwrap(),
            merge: MergeMode::Autopilot,
            task_list: "The list.".to_owned(),
            plan: Vec::new(),
        })),
        Record::Round(Box::new(RoundOpening {
            round: round_id(2),
            team: team_id(0),
            at: at(20),
            subject: RoundSubject::Item(ItemText {
                item: "1.2".parse().unwrap(),
                title: "The second item".to_owned(),
                description: "Do it.".to_owned(),
                expected_result: None,
                checks: None,
            }),
        })),
    ];
    records.extend(events.into_iter().map(|event| {
        Record::Event(Box::new(EventRecord {
            at: at(30),
            team: team_id(0),
            rounds: vec![round_id(2)],
            event,
        }))
    }));
    let mut database = HistoryDb::open(&home(dir).history_file(), MaxTextBytes::default()).unwrap();
    database.write(&records).unwrap();
}

fn pull_opened() -> HistoryEvent {
    HistoryEvent::PullOpened {
        number: PrNumber::new(NonZeroU32::new(7).unwrap()),
        branch: "agents/claude-1/task-20261111-153002-k3x9-1.2".parse().unwrap(),
        head: "0123456789abcdef0123456789abcdef01234567".parse().unwrap(),
        title: "Item 1.2".to_owned(),
        body: "What and why.".to_owned(),
    }
}

/// What `agent status --running` writes for [`rounds_state`] in the home in `dir`, and the commands the daemon was sent.
async fn running(dir: &Path) -> (String, Vec<Command>) {
    let daemon = daemon(dir, &rounds_state());
    let mut connection = Connection::connect(daemon.socket()).await.unwrap();
    let mut out = Vec::new();

    status_running(&mut connection, &Activity::new(home(dir), warsaw()), &mut out)
        .await
        .unwrap();

    let commands = daemon.requests().into_iter().map(|request| request.command).collect();
    (String::from_utf8(out).unwrap(), commands)
}

// Rule: the draft the harness opens before its checks is told from the history; the state does not keep it.
#[tokio::test]
async fn a_round_whose_pull_request_was_opened_and_is_not_ready_is_a_draft() {
    let dir = tempfile::tempdir().unwrap();
    history(dir.path(), vec![pull_opened()]);

    let (text, commands) = running(dir.path()).await;

    assert_eq!(
        text,
        "\
team-20261111-153000-t5m8  team-agents  autopilot  1 pending, 1 in progress, 0 in review, 0 approved, 0 done
  round-20261111-153002-q1w2  item 1.2  open, PR draft  started 11-11-2026 20:15
  task-20261111-153001-k3x9  coordinator  inv-opus   queued
  task-20261111-153002-k3x9  member       claude-1   queued
  task-20261111-153003-k3x9  worker       glm-1      queued
  task-20261111-153004-k3x9  reviewer     inv-codex  queued
"
    );
    // The daemon is asked for the teams, then for each round that goes on, and for no round that is closed.
    assert_eq!(
        commands,
        [
            Command::Status(StatusRequest { target: None }),
            Command::Show(agent_core::protocol::ShowRequest {
                target: Target::Round(round_id(2)),
            }),
        ]
    );
}

#[tokio::test]
async fn a_round_with_no_pull_request_in_the_history_is_only_local() {
    let dir = tempfile::tempdir().unwrap();
    history(dir.path(), Vec::new());

    let (text, _) = running(dir.path()).await;

    assert_eq!(
        text.lines().nth(1),
        Some("  round-20261111-153002-q1w2  item 1.2  open, only local  started 11-11-2026 20:15")
    );
}

// Rule: nothing is guessed: without a history the status says that it cannot tell a draft from local work, and why.
#[tokio::test]
async fn without_a_history_the_status_says_that_it_cannot_tell_a_draft_from_local_work() {
    let dir = tempfile::tempdir().unwrap();

    let (text, _) = running(dir.path()).await;

    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(
        lines[1],
        "  round-20261111-153002-q1w2  item 1.2  open, only local or PR draft  started 11-11-2026 20:15"
    );
    let last = lines.last().unwrap();
    assert!(
        last.starts_with("the history cannot say whether a draft pull request is open: "),
        "{text}"
    );
    assert!(last.contains("history.db"), "{text}");
}

// Rule: the plain status lists the closed rounds too, with how long they took, and asks the daemon about none of them.
#[tokio::test]
async fn the_status_of_every_team_says_how_long_its_closed_rounds_took() {
    let dir = tempfile::tempdir().unwrap();
    history(dir.path(), Vec::new());
    let daemon = daemon(dir.path(), &rounds_state());
    let mut connection = Connection::connect(daemon.socket()).await.unwrap();
    let mut out = Vec::new();

    status_active(
        &mut connection,
        None,
        &Activity::new(home(dir.path()), warsaw()),
        &mut out,
    )
    .await
    .unwrap();

    let text = String::from_utf8(out).unwrap();
    assert_eq!(
        text.lines().collect::<Vec<_>>()[1..3],
        [
            "  round-20261111-153001-q1w2  item 1.1  closed, abandoned  started 11-11-2026 20:15, took 31s",
            "  round-20261111-153002-q1w2  item 1.2  open, only local  started 11-11-2026 20:15",
        ]
    );
    assert_eq!(daemon.requests().len(), 2);
}

// Rule: a team with no round that goes on costs no question to the daemon beyond the list and none to the history.
#[tokio::test]
async fn a_status_with_no_round_that_goes_on_reads_no_history() {
    let dir = tempfile::tempdir().unwrap();
    let mut state = rounds_state();
    state
        .team_mut(&team_id(0))
        .unwrap()
        .close_round(&round_id(2), at(50), RoundEnd::Abandoned)
        .unwrap();
    let daemon = daemon(dir.path(), &state);
    let mut connection = Connection::connect(daemon.socket()).await.unwrap();
    let mut out = Vec::new();

    status_active(
        &mut connection,
        None,
        &Activity::new(home(dir.path()), warsaw()),
        &mut out,
    )
    .await
    .unwrap();

    let text = String::from_utf8(out).unwrap();
    assert!(!text.contains("the history cannot say"), "{text}");
    assert_eq!(daemon.requests().len(), 1);
}

#[test]
fn unanswered_opinions_show_the_finding_investigator_assignment_time_and_free_wait() {
    use agent_core::state::{OpinionAttempt, OpinionRequest, OpinionStage, SecondOpinion};
    let state = rounds_state();
    let team = state.team(&team_id(0)).unwrap();
    let mut detail = RoundDetail::new(team, &round_of(&state, 2), Vec::new());
    for (n, stage) in [
        (
            1,
            OpinionStage::Assigned {
                investigator: "inv-cafe".parse().unwrap(),
                task: "task-20261111-153001-k3x9".parse().unwrap(),
            },
        ),
        (2, OpinionStage::Waiting),
        (
            3,
            OpinionStage::Assigned {
                investigator: "inv-legacy".parse().unwrap(),
                task: "task-20261111-153003-k3x9".parse().unwrap(),
            },
        ),
    ] {
        detail.opinions.insert(
            NonZeroU32::new(n).unwrap(),
            SecondOpinion {
                request: OpinionRequest {
                    pass: NonZeroU32::MIN,
                    head: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".parse().unwrap(),
                    reason: "café boundary".to_owned().try_into().unwrap(),
                },
                stage,
            },
        );
    }
    detail.opinion_attempts.insert(
        NonZeroU32::MIN,
        vec![OpinionAttempt {
            investigator: "inv-cafe".parse().unwrap(),
            task: "task-20261111-153001-k3x9".parse().unwrap(),
            asked_at: Some(at(10)),
            failure: None,
        }],
    );
    assert_eq!(opinion_waits(&detail, warsaw()), include_str!("opinions.golden"));
    detail.opinions.clear();
    assert_eq!(opinion_waits(&detail, warsaw()), "");
    let mut legacy = serde_json::to_value(&detail).unwrap();
    legacy.as_object_mut().unwrap().remove("opinion_attempts");
    assert_eq!(
        serde_json::from_value::<RoundDetail>(legacy)
            .unwrap()
            .opinion_attempts
            .len(),
        0
    );
}

#[tokio::test]
async fn idle_round_lines_and_details_say_what_the_daemon_waits_for() {
    let dir = tempfile::tempdir().unwrap();
    let state = rounds_state();
    let team = state.teams()[0].clone();
    let round = team.round(&round_id(2)).unwrap();
    let mut detail = RoundDetail::new(&team, round, Vec::new());
    detail.waiting = Some("an answer of the human".to_owned());
    let detailed = super::super::render_round(&detail);
    let daemon = FakeDaemon::serve(&dir.path().join("agent.sock"), move |request| {
        let reply = match &request.command {
            Command::Status(_) => Reply::Teams {
                teams: vec![TeamSummary::from(&team)],
            },
            Command::Show(_) => Reply::Round {
                round: Box::new(detail.clone()),
            },
            other => panic!("unexpected status request: {other:?}"),
        };
        Answer::Lines(vec![
            Frame::Response(Response::ok(request.id, reply)).to_line().unwrap(),
        ])
    });
    let mut connection = Connection::connect(daemon.socket()).await.unwrap();
    let mut out = Vec::new();
    status_running(&mut connection, &Activity::new(home(dir.path()), warsaw()), &mut out)
        .await
        .unwrap();
    let text = String::from_utf8(out).unwrap();
    assert!(
        text.lines()
            .any(|line| line.contains(round_id(2).as_str()) && line.contains("waits for an answer of the human")),
        "{text}"
    );
    assert!(
        detailed
            .lines()
            .any(|line| line.contains(round_id(2).as_str()) && line.contains("waits for an answer of the human")),
        "{detailed}"
    );
}
