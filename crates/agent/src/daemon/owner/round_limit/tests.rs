//! Which round the time limit of a task's run applies to, read from the state.

use agent_core::decide::round_limit::RunMark;
use agent_core::ids::{ItemId, RoundId};
use agent_core::state::{Crew, ItemPhase, MergeMode, NewRun, RunEnd, State, TaskListText, Team, TeamMembers};

use super::{OverdueCut, timed_round};
use crate::daemon::owner::fixture::Daemon;
use crate::fixtures::{at, investigator, member, normal, repo, run, task, team_id, worker};

/// The round of the tests.
fn round() -> RoundId {
    "round-20261111-193001-q1w2".parse().unwrap()
}

/// The item of the tests.
fn item() -> ItemId {
    "1.1".parse().unwrap()
}

/// A team whose lead is task 1 (`inv-opus`), its member task 2 (`claude-1`) and its worker task 3, on a task list of the
/// one item 1.1, with its round open at `at(5)`; the item is `phase`. Each task has a run going for the round since
/// `at(10)`.
fn team_on(phase: Option<ItemPhase>) -> State {
    let crew = Crew::team(TeamMembers {
        coordinator: member(1, investigator("inv-opus")),
        members: vec![member(2, normal("claude-1"))],
        workers: vec![member(3, worker("glm-1"))],
    })
    .unwrap();
    let mut team = Team::new(
        team_id(0),
        repo(),
        at(0),
        MergeMode::Autopilot,
        TaskListText::new("Brief.\n\n1. Feature\n[ ] 1.1 Item\n".into()).unwrap(),
        crew,
    );
    team.open_round(round(), Some(item()), at(5)).unwrap();
    if let Some(phase) = phase {
        team.advance_item(item(), phase).unwrap();
    }
    let mut state = State::new();
    state.insert_team(team).unwrap();
    for n in 1..=3 {
        state
            .begin_run(
                &task(n),
                NewRun {
                    id: run(n),
                    at: at(10),
                    session: None,
                    rounds: vec![round()],
                },
            )
            .unwrap();
    }
    state
}

/// The item in progress with `assignee`.
fn in_progress(assignee: &str) -> ItemPhase {
    ItemPhase::InProgress {
        assignee: assignee.parse().unwrap(),
        branch: format!("agents/{assignee}/task-1.1").parse().unwrap(),
    }
}

#[test]
fn the_round_of_an_item_its_member_works_on_is_timed_from_its_opening() {
    let state = team_on(Some(in_progress("claude-1")));

    let timed = timed_round(&state, &task(2)).expect("the member's round is timed");
    assert_eq!(timed.round, round());
    assert_eq!(timed.item, item());
    assert_eq!(timed.opened_at, at(5));
    assert_eq!(timed.held_since, None);
    assert_eq!(
        timed.runs,
        vec![RunMark {
            started_at: at(10),
            stopped: false,
        }]
    );
}

#[test]
fn an_item_another_agent_works_on_does_not_time_the_task() {
    let state = team_on(Some(in_progress("claude-1")));

    assert_eq!(
        timed_round(&state, &task(1)),
        None,
        "the lead does not work on the item"
    );
}

#[test]
fn a_worker_is_never_timed() {
    let state = team_on(Some(in_progress("glm-1")));

    assert_eq!(timed_round(&state, &task(3)), None);
}

#[test]
fn an_item_not_started_is_not_timed() {
    let state = team_on(None);

    assert_eq!(timed_round(&state, &task(2)), None);
}

#[test]
fn a_closed_round_is_not_timed() {
    let mut state = team_on(Some(in_progress("claude-1")));
    state
        .close_round(&round(), at(20), agent_core::state::RoundEnd::Abandoned)
        .unwrap();

    assert_eq!(timed_round(&state, &task(2)), None);
}

#[test]
fn a_task_with_no_run_going_is_not_timed() {
    let mut state = team_on(Some(in_progress("claude-1")));
    let (_, mut found) = state.find_task_mut(&task(2)).unwrap();
    found.end_run(at(20), RunEnd::Stopped).unwrap();

    assert_eq!(timed_round(&state, &task(2)), None);
}

#[test]
fn a_stopped_run_is_read_for_the_clock() {
    let mut state = team_on(Some(in_progress("claude-1")));
    {
        let (_, mut found) = state.find_task_mut(&task(2)).unwrap();
        found.end_run(at(20), RunEnd::Stopped).unwrap();
    }
    state
        .begin_run(
            &task(2),
            NewRun {
                id: run(4),
                at: at(30),
                session: None,
                rounds: vec![round()],
            },
        )
        .unwrap();

    let timed = timed_round(&state, &task(2)).expect("the member's round is timed");
    assert_eq!(
        timed.runs,
        vec![
            RunMark {
                started_at: at(10),
                stopped: true,
            },
            RunMark {
                started_at: at(30),
                stopped: false,
            },
        ]
    );
}

/// The limit of two hours, which the example configuration gives.
const LIMIT: &str = "[limits]\nround_minutes = 120";

/// Whether the daemon cuts the run of the member, task 2, after the clock went `minutes` past the start of the tests,
/// under the configuration `table`; the cut it keeps, if any.
async fn cut_after(minutes: i64, table: &str) -> Option<OverdueCut> {
    let daemon = Daemon::new(&team_on(Some(in_progress("claude-1"))), None);
    daemon.configure(table);
    daemon.clock.advance(time::Duration::minutes(minutes));
    let (mut owner, _sinks) = daemon.bare_owner().await;
    owner.cut_overdue_round(&task(2));
    owner.cut_overdue_round(&task(2));
    owner.overdue_cuts.get(&task(2)).cloned()
}

#[tokio::test]
async fn a_round_over_its_limit_is_cut_with_its_minutes() {
    // The round was opened at 5 s: at 125 min the clock reads 124 whole minutes.
    let cut = cut_after(125, LIMIT).await;

    assert_eq!(
        cut,
        Some(OverdueCut {
            round: round(),
            item: item(),
            minutes: 124,
            limit: 120,
        })
    );
}

#[tokio::test]
async fn a_round_within_its_limit_is_not_cut() {
    assert_eq!(cut_after(120, LIMIT).await, None, "119 whole minutes since the opening");
}

#[tokio::test]
async fn the_limit_of_the_configuration_is_the_one_read() {
    assert_eq!(cut_after(125, "[limits]\nround_minutes = 180").await, None);
    assert_eq!(
        cut_after(181, "[limits]\nround_minutes = 180")
            .await
            .map(|cut| cut.limit),
        Some(180)
    );
}

#[tokio::test]
async fn no_round_is_cut_unless_the_configuration_gives_a_limit() {
    assert_eq!(cut_after(3 * 24 * 60, "").await, None);
}

#[tokio::test]
async fn a_limit_of_zero_never_cuts_a_round() {
    assert_eq!(cut_after(3 * 24 * 60, "[limits]\nround_minutes = 0").await, None);
}

#[tokio::test]
async fn the_lead_of_a_round_its_member_works_on_is_not_cut() {
    let daemon = Daemon::new(&team_on(Some(in_progress("claude-1"))), None);
    daemon.configure(LIMIT);
    daemon.clock.advance(time::Duration::days(1));
    let (mut owner, _sinks) = daemon.bare_owner().await;

    owner.cut_overdue_round(&task(1));
    owner.cut_overdue_round(&task(3));

    assert!(owner.overdue_cuts.is_empty(), "{:?}", owner.overdue_cuts);
}
