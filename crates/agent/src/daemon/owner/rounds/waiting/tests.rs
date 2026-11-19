//! A lost scheduling obligation is an error once, never a repeated idle-tick message.

use std::sync::{Arc, Mutex};

use agent_core::decide::wait::{Obligation, Waiting};
use tracing_subscriber::layer::{Context, SubscriberExt};
use tracing_subscriber::{Layer, Registry};

use crate::daemon::owner::fixture::Daemon;
use crate::fixtures::solo_state;

/// Error levels emitted while observing a round.
#[derive(Clone, Default)]
struct Errors(Arc<Mutex<Vec<tracing::Level>>>);

impl Layer<Registry> for Errors {
    fn on_event(&self, event: &tracing::Event<'_>, _context: Context<'_, Registry>) {
        if *event.metadata().level() == tracing::Level::ERROR {
            self.0.lock().unwrap().push(*event.metadata().level());
        }
    }
}

#[tokio::test]
async fn an_unowned_idle_round_is_logged_as_an_error_once() {
    let daemon = Daemon::new(&solo_state(), None);
    let (mut owner, _sinks) = daemon.bare_owner().await;
    let round = "round-20261119-090000-test".parse().unwrap();
    let errors = Errors::default();
    let subscriber = tracing_subscriber::registry().with(errors.clone());
    tracing::subscriber::with_default(subscriber, || {
        owner.observe_wait(&round, None);
        owner.observe_wait(&round, Some(Waiting::For(Obligation::Human)));
        owner.observe_wait(&round, Some(Waiting::Defect));
        owner.observe_wait(&round, Some(Waiting::Defect));
        owner.observe_wait(&round, None);
        owner.observe_wait(&round, Some(Waiting::Defect));
    });
    assert_eq!(*errors.0.lock().unwrap(), vec![tracing::Level::ERROR]);
}

use agent_core::ids::{RoundId, TaskId, TeamId};
use agent_core::state::{ItemPhase, NewRun, RunEnd, State};
use agent_core::status::{RunOutcome, TaskStatus};
use tokio_util::sync::CancellationToken;

use crate::daemon::owner::rounds::publish::{GATE_SEND_BACKS, OwnGate};
use crate::fixtures::{at, run, task, team_id, team_state};

/// How the lead's last run ended, for a round that is idle.
const LEAD_DONE: RunEnd = RunEnd::Reported {
    outcome: RunOutcome::Done,
};

/// The round of item 1.1 of the sample team, whose lead (task 1) has ended a run in it with `end` and waits.
fn idle_round(phase: ItemPhase, end: RunEnd) -> (State, TeamId, RoundId) {
    let mut state = team_state();
    let team_id = team_id(0);
    let round: RoundId = "round-20261119-090000-wait".parse().unwrap();
    {
        let mut team = state.team_mut(&team_id).unwrap();
        let item = "1.1".parse().unwrap();
        team.open_round(round.clone(), Some(item), at(10)).unwrap();
        team.advance_item(
            item,
            ItemPhase::InProgress {
                assignee: "claude-1".parse().unwrap(),
                branch: "agents/inv-opus/task-1-1.1".parse().unwrap(),
            },
        )
        .unwrap();
        if !matches!(phase, ItemPhase::InProgress { .. }) {
            team.advance_item(item, phase).unwrap();
        }
        team.begin_run(
            &task(1),
            NewRun {
                id: run(1),
                at: at(20),
                session: None,
                rounds: vec![round.clone()],
            },
        )
        .unwrap();
        let (_, mut lead) = team.find_mut(&task(1)).unwrap();
        lead.transition(TaskStatus::Running).unwrap();
        lead.end_run(at(30), end).unwrap();
        lead.transition(TaskStatus::Done).unwrap();
    }
    (state, team_id, round)
}

/// What the owner of `state` says the round waits for, after `prepare` set its scheduling.
async fn obligation_after(
    state: &State,
    prepare: impl FnOnce(&mut crate::daemon::owner::Owner, &TaskId, &RoundId),
    round: &RoundId,
    team: &TeamId,
) -> Option<Obligation> {
    let daemon = Daemon::new(state, None);
    let (mut owner, _sinks) = daemon.bare_owner().await;
    prepare(&mut owner, &task(1), round);
    let state = owner.parts.state.clone();
    let team = state.team(team).unwrap();
    owner.round_obligation(team, team.round(round).unwrap())
}

fn in_progress() -> ItemPhase {
    ItemPhase::InProgress {
        assignee: "claude-1".parse().unwrap(),
        branch: "agents/inv-opus/task-1-1.1".parse().unwrap(),
    }
}

#[tokio::test]
async fn a_gate_that_has_completed_is_not_waited_for() {
    let (state, team, round) = idle_round(in_progress(), LEAD_DONE);

    let found = obligation_after(
        &state,
        |owner, lead, _| {
            // The token of a gate that was asked for and is over: the service never removes it.
            owner.gate_stops.insert(lead.clone(), CancellationToken::new());
        },
        &round,
        &team,
    )
    .await;

    assert_eq!(found, None);
}

#[tokio::test]
async fn a_gate_of_the_harness_that_runs_is_waited_for() {
    let (state, team, round) = idle_round(in_progress(), LEAD_DONE);

    let found = obligation_after(
        &state,
        |owner, lead, _| {
            let (_, answer) = tokio::sync::oneshot::channel();
            owner.watch.own_gates.insert(
                lead.clone(),
                OwnGate::Running {
                    tip: "0123456789abcdef0123456789abcdef01234567".parse().unwrap(),
                    retry: false,
                    answer,
                },
            );
        },
        &round,
        &team,
    )
    .await;

    assert_eq!(found, Some(Obligation::Gate));
}

#[tokio::test]
async fn a_lead_sent_back_for_its_gate_too_often_waits_for_the_human() {
    let (state, team, round) = idle_round(in_progress(), LEAD_DONE);

    let found = obligation_after(
        &state,
        |owner, lead, _| {
            owner.watch.gate_sent_back.insert(lead.clone(), GATE_SEND_BACKS);
        },
        &round,
        &team,
    )
    .await;

    assert_eq!(found, Some(Obligation::Human));
}

#[tokio::test]
async fn a_round_in_its_review_waits_for_the_reviewers() {
    let phase = ItemPhase::InReview {
        pr: crate::fixtures::pr(7),
        reviews: Vec::new(),
    };
    let (state, team, round) = idle_round(phase, LEAD_DONE);

    let found = obligation_after(&state, |_, _, _| {}, &round, &team).await;

    assert_eq!(found, Some(Obligation::Review));
}

#[tokio::test]
async fn a_recorded_wait_of_the_scheduler_is_the_round_wait() {
    let (state, team, round) = idle_round(in_progress(), LEAD_DONE);

    let found = obligation_after(
        &state,
        |owner, _, round| {
            owner.watch.waits.insert(round.clone(), Obligation::SecondOpinion);
        },
        &round,
        &team,
    )
    .await;

    assert_eq!(found, Some(Obligation::SecondOpinion));
}

#[tokio::test]
async fn a_lead_whose_run_did_not_end_done_waits_for_its_correction() {
    let (state, team, round) = idle_round(in_progress(), RunEnd::NoStatus { exit_code: Some(1) });

    let found = obligation_after(&state, |_, _, _| {}, &round, &team).await;

    assert_eq!(found, Some(Obligation::LeadCorrection));
}

#[tokio::test]
async fn an_idle_round_with_a_done_lead_and_no_scheduled_work_is_unowned() {
    let (state, team, round) = idle_round(in_progress(), LEAD_DONE);

    let found = obligation_after(&state, |_, _, _| {}, &round, &team).await;

    assert_eq!(found, None);
}

#[tokio::test]
async fn insufficient_independent_investigators_record_an_explicit_round_wait() {
    let (state, team, round) = idle_round(in_progress(), LEAD_DONE);
    let daemon = Daemon::new(&state, None);
    let (mut owner, _sinks) = daemon.bare_owner().await;
    let lead = owner.lead_of(&team).unwrap();
    let item = state.team(&team).unwrap().round(&round).unwrap().item().unwrap();
    assert!(
        !owner
            .try_ask(&team, &lead, &crate::fixtures::pr(7), &[item], at(20))
            .await
            .unwrap()
    );
    assert_eq!(owner.watch.waits.get(&round), Some(&Obligation::FreeInvestigator));
}

/// Gives the item 1.1 of the sample team a ledger with one finding and a second opinion asked about it, in `stage`.
fn with_opinion(state: &mut State, team: &TeamId, stage: agent_core::state::OpinionStage) {
    use agent_core::review_findings::{Finding, Severity};
    use agent_core::state::{FindingStatus, Findings, OpinionRequest, RecordedFinding};
    use std::num::NonZeroU32;

    let mut team = state.team_mut(team).unwrap();
    let mut item = team.item_mut("1.1".parse().unwrap()).unwrap();
    item.start_findings();
    let mut ledger = Findings::new(vec![RecordedFinding {
        number: NonZeroU32::MIN,
        finding: Finding {
            severity: Severity::Blocker,
            rule: "ERR-3".into(),
            place: "lib.rs:1".into(),
            text: "problem → fix".into(),
            check: "read lib.rs:1".into(),
        },
        reporter: "inv-codex".parse().unwrap(),
        pass: NonZeroU32::MIN,
        status: FindingStatus::Open,
    }])
    .unwrap();
    ledger
        .request_opinion(
            NonZeroU32::MIN,
            OpinionRequest {
                pass: NonZeroU32::MIN,
                head: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".parse().unwrap(),
                reason: "café branch is unreachable".to_owned().try_into().unwrap(),
            },
        )
        .unwrap();
    if let agent_core::state::OpinionStage::Assigned { investigator, task } = stage {
        ledger.assign_opinion(NonZeroU32::MIN, investigator, task).unwrap();
    }
    *item.findings_mut().unwrap() = ledger;
}

#[tokio::test]
async fn a_second_opinion_nobody_has_taken_waits_for_a_free_investigator() {
    let (mut state, team, round) = idle_round(in_progress(), LEAD_DONE);
    with_opinion(&mut state, &team, agent_core::state::OpinionStage::Waiting);

    let found = obligation_after(&state, |_, _, _| {}, &round, &team).await;

    assert_eq!(found, Some(Obligation::FreeInvestigator));
}

#[tokio::test]
async fn a_second_opinion_an_investigator_has_taken_waits_for_the_opinion() {
    let (mut state, team, round) = idle_round(in_progress(), LEAD_DONE);
    with_opinion(
        &mut state,
        &team,
        agent_core::state::OpinionStage::Assigned {
            investigator: "inv-codex".parse().unwrap(),
            task: task(4),
        },
    );

    let found = obligation_after(&state, |_, _, _| {}, &round, &team).await;

    assert_eq!(found, Some(Obligation::SecondOpinion));
}

/// The round of item 1.1 in review, where the review of the reviewer `inv-codex` (task 4) is asked and its task failed.
fn round_with_a_failed_reviewer() -> (State, TeamId, RoundId) {
    let phase = ItemPhase::InReview {
        pr: crate::fixtures::pr(7),
        reviews: vec![crate::fixtures::review("inv-codex", 4)],
    };
    let (mut state, team_id, round) = idle_round(phase, LEAD_DONE);
    {
        let mut team = state.team_mut(&team_id).unwrap();
        team.begin_run(
            &task(4),
            NewRun {
                id: run(2),
                at: at(21),
                session: None,
                rounds: vec![round.clone()],
            },
        )
        .unwrap();
        let (_, mut reviewer) = team.find_mut(&task(4)).unwrap();
        reviewer.transition(TaskStatus::Running).unwrap();
        reviewer
            .end_run(at(25), RunEnd::NoStatus { exit_code: Some(1) })
            .unwrap();
        reviewer.transition(TaskStatus::Failed).unwrap();
    }
    (state, team_id, round)
}

#[tokio::test]
async fn a_failed_reviewer_nobody_replaces_waits_for_a_free_investigator() {
    let (state, team, round) = round_with_a_failed_reviewer();

    let found = {
        let daemon = Daemon::new(&state, None);
        let (mut owner, _sinks) = daemon.bare_owner().await;
        owner.watch.waits.insert(round.clone(), Obligation::FreeInvestigator);
        let team = owner.parts.state.team(&team).unwrap();
        owner.waiting_for(team, team.round(&round).unwrap())
    };

    assert_eq!(found, Some(Waiting::For(Obligation::FreeInvestigator)));
}

#[tokio::test]
async fn a_failed_reviewer_that_is_not_yet_looked_at_for_replacement_waits_for_the_human() {
    let (state, team, round) = round_with_a_failed_reviewer();

    let found = {
        let daemon = Daemon::new(&state, None);
        let (owner, _sinks) = daemon.bare_owner().await;
        let team = owner.parts.state.team(&team).unwrap();
        owner.waiting_for(team, team.round(&round).unwrap())
    };

    assert_eq!(found, Some(Waiting::For(Obligation::Human)));
}
