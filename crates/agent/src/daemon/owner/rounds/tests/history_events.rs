//! What the daemon records of a pull request and its round in the history, in a daemon with a fake GitHub, a bare
//! `origin`, stand-ins for the model CLIs and the real `agentbus`: the pull request opened, marked ready, every review
//! asked and submitted, a fix pass, a push, the approval, the merge or the hand-over to the human, the close, and the end
//! of the round — read back through the reading API after the daemon stopped, never inserted by hand, and compared with
//! what the fake GitHub and the state say.

use agent_core::history::{EventKind, EventRecord, HistoryEvent, Record};
use agent_core::ids::{RoundId, TeamId};
use agent_core::protocol::{Client, Command, GateRequest, Reply};
use agent_core::state::{GateOutcome, GateTree, ItemPhase, MergeMode, PrNumber, RoundEnd};
use agent_core::status::ReviewVerdict;
use agent_core::status::TaskStatus;
use agent_runtime::history::Reader;

use super::rig::{ONE, Rig, investigator, normal};
use crate::client::Connection;

/// A reviewer that approves.
const APPROVING: &str = "done:VERDICT: APPROVE\\nSUMMARY: fine";

/// A reviewer that asks for changes the first time and approves afterwards.
const CHANGES_ONCE: &str = "done:VERDICT: REQUEST_CHANGES\\nBLOCKERS\\n1. [TDD-2] src/lib.rs:7 — the case is untested,done:VERDICT: APPROVE\\nSUMMARY: fixed";

/// The events of `team` the history holds, in the order they were written, with their places.
fn events(rig: &Rig, team: &TeamId) -> Vec<(u64, EventRecord)> {
    Reader::open(&rig.home.history_file())
        .unwrap()
        .team_records(team)
        .unwrap()
        .into_iter()
        .filter_map(|stored| match stored.record {
            Record::Event(event) => Some((stored.sequence, *event)),
            _ => None,
        })
        .collect()
}

/// The place in `events` of the `n`-th (from 0) event of `kind`.
fn place(events: &[(u64, EventRecord)], kind: EventKind, n: usize) -> usize {
    events
        .iter()
        .enumerate()
        .filter(|(_, (_, event))| event.event.kind() == kind)
        .nth(n)
        .unwrap_or_else(|| panic!("no {kind} number {n} in {:?}", kinds(events)))
        .0
}

fn kinds(events: &[(u64, EventRecord)]) -> Vec<EventKind> {
    events.iter().map(|(_, event)| event.event.kind()).collect()
}

fn count(events: &[(u64, EventRecord)], kind: EventKind) -> usize {
    kinds(events).iter().filter(|found| **found == kind).count()
}

/// The round of the only item of the team in `rig`.
fn the_round(rig: &Rig) -> RoundId {
    rig.state().teams()[0].rounds()[0].id().clone()
}

#[tokio::test]
async fn a_solo_round_with_a_fix_pass_is_recorded_from_its_pull_request_to_its_merge() {
    let mut rig = Rig::new(
        &[
            normal("claude-1", "work"),
            investigator("inv-r1", CHANGES_ONCE),
            investigator("inv-r2", APPROVING),
        ],
        5,
        false,
    )
    .await;
    rig.start().await;

    let (team, _) = rig.run(&["claude-1"], ONE, MergeMode::Autopilot, None).await;
    rig.item_where("1.1", "done", |phase| matches!(phase, ItemPhase::Done { .. }))
        .await;
    rig.stop().await.unwrap();

    let events = events(&rig, &team);
    let round = the_round(&rig);
    let pull = rig.github.pulls().remove(0);
    let number = PrNumber::new(std::num::NonZeroU32::new(pull.number).unwrap());
    assert_eq!(count(&events, EventKind::PullOpened), 1, "{:?}", kinds(&events));
    assert_eq!(count(&events, EventKind::PullReady), 1);
    assert_eq!(count(&events, EventKind::ReviewAsked), 4);
    assert_eq!(count(&events, EventKind::ReviewSubmitted), 4);
    assert_eq!(count(&events, EventKind::FixPassStarted), 1);
    assert_eq!(count(&events, EventKind::PullPushed), 1);
    assert_eq!(count(&events, EventKind::PullApproved), 1);
    assert_eq!(count(&events, EventKind::PullMerged), 1);
    assert_eq!(count(&events, EventKind::RoundClosed), 1);
    // The order a pull request lives in.
    let order = [
        place(&events, EventKind::PullOpened, 0),
        place(&events, EventKind::PullReady, 0),
        place(&events, EventKind::ReviewAsked, 0),
        place(&events, EventKind::ReviewSubmitted, 0),
        place(&events, EventKind::FixPassStarted, 0),
        place(&events, EventKind::PullPushed, 0),
        place(&events, EventKind::ReviewAsked, 2),
        place(&events, EventKind::ReviewSubmitted, 2),
        place(&events, EventKind::PullApproved, 0),
        place(&events, EventKind::PullMerged, 0),
        place(&events, EventKind::RoundClosed, 0),
    ];
    assert!(
        order.windows(2).all(|pair| pair[0] < pair[1]),
        "{order:?} in {:?}",
        kinds(&events)
    );
    // Each belongs to the team and the round.
    for (_, event) in &events {
        assert_eq!(event.team, team);
        assert_eq!(event.rounds, std::slice::from_ref(&round), "{:?}", event.event);
    }
    let found = |kind, n| &events[place(&events, kind, n)].1.event;
    let HistoryEvent::PullOpened {
        number: opened,
        branch,
        head: opened_head,
        title,
        body,
    } = found(EventKind::PullOpened, 0)
    else {
        panic!("not an opening")
    };
    assert_eq!(
        (*opened, branch.as_str(), title, body),
        (number, pull.head.as_str(), &pull.title, &pull.body)
    );
    let HistoryEvent::PullPushed { head: pushed, .. } = found(EventKind::PullPushed, 0) else {
        panic!("not a push")
    };
    assert_ne!(pushed, opened_head);
    assert_eq!(pushed.as_str(), rig.remote_tip(&pull.head));
    let HistoryEvent::PullMerged { pr, commit } = found(EventKind::PullMerged, 0) else {
        panic!("not a merge")
    };
    assert_eq!(
        (*pr, commit.as_ref().map(|commit| commit.as_str().to_owned())),
        (number, Some(rig.remote_tip(&pull.base)))
    );
    let HistoryEvent::RoundClosed { round: closed, end } = found(EventKind::RoundClosed, 0) else {
        panic!("not a close")
    };
    assert_eq!((closed, *end), (&round, RoundEnd::Merged));
}

#[tokio::test]
async fn every_review_is_recorded_asked_of_a_reviewer_with_its_pass_and_head_and_submitted_with_its_whole_report() {
    let mut rig = Rig::new(
        &[
            normal("claude-1", "work"),
            investigator("inv-r1", CHANGES_ONCE),
            investigator("inv-r2", APPROVING),
        ],
        5,
        false,
    )
    .await;
    rig.start().await;

    let (team, _) = rig.run(&["claude-1"], ONE, MergeMode::Autopilot, None).await;
    rig.item_where("1.1", "done", |phase| matches!(phase, ItemPhase::Done { .. }))
        .await;
    rig.stop().await.unwrap();

    let events = events(&rig, &team);
    let asked: Vec<_> = events
        .iter()
        .filter_map(|(_, event)| match &event.event {
            HistoryEvent::ReviewAsked {
                pass,
                reviewer,
                head,
                task,
                ..
            } => Some((*pass, reviewer.as_str().to_owned(), head.clone(), task.clone())),
            _ => None,
        })
        .collect();
    let submitted: Vec<_> = events
        .iter()
        .filter_map(|(_, event)| match &event.event {
            HistoryEvent::ReviewSubmitted {
                pass,
                reviewer,
                verdict,
                report,
                task,
                ..
            } => Some((
                *pass,
                reviewer.as_str().to_owned(),
                *verdict,
                report.clone(),
                task.clone(),
            )),
            _ => None,
        })
        .collect();
    let state = rig.state();
    for (pass, reviewer, _, task) in &asked {
        let member = state.find_task(task).unwrap();
        assert_eq!(member.member.agent().name(), reviewer);
        assert!(*pass <= 1);
    }
    assert_eq!(asked.iter().filter(|(pass, ..)| *pass == 0).count(), 2);
    assert_eq!(asked.iter().filter(|(pass, ..)| *pass == 1).count(), 2);
    let first_head = &asked.iter().find(|(pass, ..)| *pass == 0).unwrap().2;
    let second_head = &asked.iter().find(|(pass, ..)| *pass == 1).unwrap().2;
    assert_ne!(first_head, second_head);
    assert!(
        asked
            .iter()
            .filter(|(pass, ..)| *pass == 0)
            .all(|(_, _, head, _)| head == first_head)
    );
    let changes = submitted
        .iter()
        .find(|(pass, reviewer, ..)| *pass == 0 && reviewer == "inv-r1")
        .unwrap();
    assert_eq!(changes.2, ReviewVerdict::RequestChanges);
    assert!(
        changes.3.contains("[TDD-2] src/lib.rs:7 — the case is untested"),
        "{}",
        changes.3
    );
    let approval = submitted
        .iter()
        .find(|(pass, reviewer, ..)| *pass == 0 && reviewer == "inv-r2")
        .unwrap();
    assert_eq!(approval.2, ReviewVerdict::Approve);
    assert!(approval.3.contains("SUMMARY: fine"), "{}", approval.3);
}

#[tokio::test]
async fn a_fix_pass_is_recorded_with_its_number_of_how_many_and_the_findings_the_team_was_sent() {
    let mut rig = Rig::new(
        &[
            normal("claude-1", "work"),
            investigator("inv-r1", CHANGES_ONCE),
            investigator("inv-r2", APPROVING),
        ],
        5,
        false,
    )
    .await;
    rig.start().await;

    let (team, tasks) = rig.run(&["claude-1"], ONE, MergeMode::Autopilot, None).await;
    rig.item_where("1.1", "done", |phase| matches!(phase, ItemPhase::Done { .. }))
        .await;
    rig.stop().await.unwrap();

    let events = events(&rig, &team);
    let HistoryEvent::FixPassStarted { pass, of, findings, .. } =
        &events[place(&events, EventKind::FixPassStarted, 0)].1.event
    else {
        panic!("not a fix pass")
    };
    assert_eq!((pass.get(), *of), (1, 5));
    assert!(
        findings.contains("[TDD-2] src/lib.rs:7 — the case is untested"),
        "{findings}"
    );
    // What the team was sent is what its prompt got.
    let prompt = rig.prompt(&tasks[0], 2).await;
    assert!(prompt.contains(findings.as_str()), "{prompt}");
}

#[tokio::test]
async fn with_no_merge_the_approval_and_the_hand_over_are_recorded_and_the_human_merge_after_them() {
    let mut rig = Rig::new(
        &[
            normal("claude-1", "work"),
            investigator("inv-r1", APPROVING),
            investigator("inv-r2", APPROVING),
        ],
        5,
        false,
    )
    .await;
    rig.start().await;

    let (team, _) = rig.run(&["claude-1"], ONE, MergeMode::NoMerge, None).await;
    rig.notification_with("is approved").await;
    rig.github.merged_by_a_human(1);
    rig.item_where("1.1", "done", |phase| matches!(phase, ItemPhase::Done { .. }))
        .await;
    rig.stop().await.unwrap();

    let events = events(&rig, &team);
    assert_eq!(count(&events, EventKind::PullApproved), 1, "{:?}", kinds(&events));
    assert_eq!(count(&events, EventKind::PullHandedOver), 1);
    let order = [
        place(&events, EventKind::PullApproved, 0),
        place(&events, EventKind::PullHandedOver, 0),
        place(&events, EventKind::RoundClosed, 0),
        place(&events, EventKind::PullMerged, 0),
    ];
    assert!(
        order.windows(2).all(|pair| pair[0] < pair[1]),
        "{order:?} in {:?}",
        kinds(&events)
    );
    let HistoryEvent::RoundClosed { end, .. } = &events[place(&events, EventKind::RoundClosed, 0)].1.event else {
        panic!("not a close")
    };
    assert_eq!(*end, RoundEnd::Approved);
}

#[tokio::test]
async fn a_pull_request_the_human_closed_is_recorded_once_however_often_it_is_looked_at() {
    let mut rig = Rig::new(
        &[
            normal("claude-1", "work"),
            investigator("inv-r1", APPROVING),
            investigator("inv-r2", APPROVING),
        ],
        5,
        false,
    )
    .await;
    rig.start().await;

    let (team, _) = rig.run(&["claude-1"], ONE, MergeMode::NoMerge, None).await;
    rig.notification_with("is approved").await;
    rig.github.closed_by_a_human(1);
    rig.notification_with("closed").await;
    rig.stop().await.unwrap();

    let events = events(&rig, &team);
    assert_eq!(count(&events, EventKind::PullClosed), 1, "{:?}", kinds(&events));
    assert_eq!(count(&events, EventKind::PullMerged), 0);
}

#[tokio::test]
async fn a_gate_the_daemon_ran_is_recorded_with_its_task_head_outcome_and_log_and_linked_to_the_round_of_its_task() {
    let mut rig = Rig::new(
        &[
            normal("claude-1", "commit"),
            investigator("inv-r1", APPROVING),
            investigator("inv-r2", APPROVING),
        ],
        5,
        false,
    )
    .await;
    rig.start().await;
    let (team, tasks) = rig.run(&["claude-1"], ONE, MergeMode::Autopilot, None).await;
    rig.status_is(&tasks[0], TaskStatus::Waiting).await;

    let reply = Connection::connect(&rig.home.socket_file())
        .await
        .unwrap()
        .request_until_answered(Client::Cli, Command::Gate(GateRequest { task: tasks[0].clone() }))
        .await
        .unwrap();
    let Reply::GateRecorded { record } = reply else {
        panic!("a gate record was expected: {reply:?}")
    };
    rig.stop().await.unwrap();

    let events = events(&rig, &team);
    assert_eq!(count(&events, EventKind::Gate), 1, "{:?}", kinds(&events));
    let (_, gate) = &events[place(&events, EventKind::Gate, 0)];
    let HistoryEvent::Gate {
        task,
        started_at,
        record: recorded,
    } = &gate.event
    else {
        panic!("not a gate")
    };
    assert_eq!(task, &tasks[0]);
    assert_eq!(recorded, &record);
    assert_eq!(
        (recorded.tree, recorded.result.outcome),
        (GateTree::Clean, GateOutcome::Passed)
    );
    assert!(*started_at <= gate.at);
    assert_eq!(gate.at, record.result.finished_at);
    assert!(recorded.result.log.as_path().exists());
    assert_eq!(gate.rounds, [the_round(&rig)]);
}
