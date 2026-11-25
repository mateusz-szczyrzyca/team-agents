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
fn with_opinion(all: &mut State, team: &TeamId, stage: agent_core::state::OpinionStage) {
    use agent_core::review_findings::{Finding, Severity};
    use agent_core::state::{FindingStatus, Findings, OpinionRequest, RecordedFinding};
    use std::num::NonZeroU32;

    let mut team = all.team_mut(team).unwrap();
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
        owner.stuck_obligation(team, team.round(&round).unwrap())
    };

    assert_eq!(found, Some(Obligation::FreeInvestigator));
}

#[tokio::test]
async fn a_failed_reviewer_that_is_not_yet_looked_at_for_replacement_waits_for_the_human() {
    let (state, team, round) = round_with_a_failed_reviewer();

    let found = {
        let daemon = Daemon::new(&state, None);
        let (owner, _sinks) = daemon.bare_owner().await;
        let team = owner.parts.state.team(&team).unwrap();
        owner.stuck_obligation(team, team.round(&round).unwrap())
    };

    assert_eq!(found, Some(Obligation::Human));
}

#[tokio::test]
async fn a_turn_of_the_owner_forgets_the_waits_of_closed_rounds_and_of_closed_or_gone_teams() {
    use super::{Step, WaitingStep};
    use agent_core::state::RoundEnd;
    use agent_runtime::github::Outlook;

    let mut state = team_state();
    let team = team_id(0);
    let open: RoundId = "round-20261120-070000-open".parse().unwrap();
    let closed: RoundId = "round-20261120-070000-shut".parse().unwrap();
    state
        .open_round(&team, closed.clone(), Some("1.1".parse().unwrap()), at(1))
        .unwrap();
    state.close_round(&closed, at(2), RoundEnd::Abandoned).unwrap();
    state
        .open_round(&team, open.clone(), Some("1.2".parse().unwrap()), at(2))
        .unwrap();
    let gone: TeamId = "team-20261120-070000-gone".parse().unwrap();
    let daemon = Daemon::new(&state, None);
    let (mut owner, _sinks) = daemon.bare_owner().await;
    let places = [
        Step::Round { round: open.clone() },
        Step::Round { round: closed },
        Step::Team { team: team.clone() },
        Step::Team { team: gone },
    ];
    for place in &places {
        owner
            .watch
            .github_waits
            .failed(
                place,
                &team,
                WaitingStep::AskOpinion,
                None,
                "refused with status 404".to_owned(),
                Outlook::WillNotPass,
                at(3),
            )
            .unwrap();
    }

    owner.team_life().await.unwrap();

    let kept = |owner: &crate::daemon::owner::Owner| {
        places
            .iter()
            .filter(|place| !owner.watch.github_waits.due(place, WaitingStep::AskOpinion, at(4)))
            .cloned()
            .collect::<Vec<_>>()
    };
    assert_eq!(
        kept(&owner),
        vec![Step::Round { round: open.clone() }, Step::Team { team: team.clone() }]
    );

    owner.closed.insert(team.clone());
    owner.team_life().await.unwrap();

    assert_eq!(kept(&owner), vec![Step::Round { round: open }]);
}

#[tokio::test]
async fn delivery_marker_owns_the_wait_even_when_a_gate_is_running() {
    let (mut state, team, round) = idle_round(in_progress(), LEAD_DONE);
    {
        let mut found = state.team_mut(&team).unwrap();
        let owner = "1.2".parse().unwrap();
        found
            .open_round("round-20261119-090001-ownr".parse().unwrap(), Some(owner), at(10))
            .unwrap();
        found.advance_item(owner, in_progress()).unwrap();
        found
            .advance_item(
                owner,
                ItemPhase::InReview {
                    pr: crate::fixtures::pr(7),
                    reviews: Vec::new(),
                },
            )
            .unwrap();
        found.deliver_round(&round, crate::fixtures::pr(7).number).unwrap();
    }
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
    assert_eq!(found, Some(Obligation::Merge));
}

#[tokio::test]
async fn an_approved_round_merging_the_base_waits_for_the_lead_and_asking_the_human_waits_for_the_human() {
    let pr = crate::fixtures::pr(7);
    let phase = ItemPhase::InReview {
        pr: pr.clone(),
        reviews: vec![crate::fixtures::review("inv-codex", 4)],
    };
    let (mut state, team, round) = idle_round(phase, LEAD_DONE);
    {
        let mut found = state.team_mut(&team).unwrap();
        let mut item = found.item_mut("1.1".parse().unwrap()).unwrap();
        item.submit_review(&task(4), agent_core::status::ReviewVerdict::Approve, at(25))
            .unwrap();
        let ItemPhase::InReview { reviews, .. } = item.phase().clone() else {
            panic!("in review");
        };
        item.advance(ItemPhase::Approved { pr, reviews }).unwrap();
        item.begin_merge_conflict("0123456789abcdef0123456789abcdef01234567".parse().unwrap(), at(35));
    }

    let merging = obligation_after(&state, |_, _, _| {}, &round, &team).await;
    assert_eq!(merging, Some(Obligation::LeadCorrection));

    {
        let mut found = state.team_mut(&team).unwrap();
        let mut item = found.item_mut("1.1".parse().unwrap()).unwrap();
        item.ask_human_about_merge_conflict(at(40));
    }

    let asking = obligation_after(&state, |_, _, _| {}, &round, &team).await;
    assert_eq!(asking, Some(Obligation::Human));
}

/// The round `round` of item 1.1 of the sample team, opened, whose item is given to the task `number`, that is
/// `status`.
fn round_for(number: u32, status: TaskStatus) -> (State, TeamId, RoundId) {
    let mut state = team_state();
    let team_id = team_id(0);
    let round: RoundId = "round-20261119-090001-wait".parse().unwrap();
    {
        let mut team = state.team_mut(&team_id).unwrap();
        team.open_round(round.clone(), Some("1.1".parse().unwrap()), at(10))
            .unwrap();
        let (_, mut member) = team.find_mut(&task(number)).unwrap();
        if member.status() == TaskStatus::Queued && status != TaskStatus::Queued {
            // A queued task runs, then waits: that is how a task comes to wait for its next run.
            member.transition(TaskStatus::Running).unwrap();
        }
        if member.status() != status {
            member.transition(status).unwrap();
        }
    }
    (state, team_id, round)
}

// Instant after a round opens, its task is queued and the run that will name the round has not started yet: the round
// waits for that run, it is not a round that waits for nothing.
#[tokio::test]
async fn a_round_of_a_queued_task_waits_for_the_run_of_the_task() {
    let (state, team, round) = round_for(1, TaskStatus::Queued);
    let daemon = home_with_token(&state);
    let (mut owner, _sinks) = daemon.bare_owner().await;

    let found = waiting_after(&mut owner, &round, &team, |_, _, _| {}).await;

    assert_eq!(found, Some(Waiting::For(Obligation::QueuedRun)));
}

// A round whose task has run, given its answers and has no run yet is queued again with a new run, and waits for
// that run.
#[tokio::test]
async fn a_round_of_a_task_that_waits_for_its_next_run_is_not_a_defect() {
    let (mut state, team, round) = round_for(1, TaskStatus::Running);
    {
        let mut team_state = state.team_mut(&team).unwrap();
        let (_, mut lead) = team_state.find_mut(&task(1)).unwrap();
        lead.begin_run(run(1), at(11), None).unwrap();
        lead.end_run(at(12), LEAD_DONE).unwrap();
        lead.send_back_once_more();
        lead.transition(TaskStatus::Queued).unwrap();
    }
    let daemon = home_with_token(&state);
    let (mut owner, _sinks) = daemon.bare_owner().await;

    let found = waiting_after(&mut owner, &round, &team, |_, _, _| {}).await;

    assert_eq!(found, Some(Waiting::For(Obligation::QueuedRun)));
}

// A round a step of which waits for GitHub keeps that wait, whatever its lead's task is doing.
#[tokio::test]
async fn a_round_whose_step_waits_for_github_is_not_a_queued_run() {
    let (state, team, round) = round_for(1, TaskStatus::Queued);
    let daemon = home_with_token(&state);
    let (mut owner, _sinks) = daemon.bare_owner().await;

    let found = waiting_after(&mut owner, &round, &team, |owner, _, round| {
        owner.watch.waits.insert(round.clone(), Obligation::FreeInvestigator);
    })
    .await;

    assert_eq!(found, Some(Waiting::For(Obligation::FreeInvestigator)));
}

/// What `owner`, with pull requests to keep, says the round `round` of `team` waits for once its scheduling is done.
async fn waiting_after(
    owner: &mut crate::daemon::owner::Owner,
    round: &RoundId,
    team: &TeamId,
    prepare: impl FnOnce(&mut crate::daemon::owner::Owner, &TaskId, &RoundId),
) -> Option<Waiting> {
    owner.set_pulls(pulls_of(owner).await.expect("the fixture home has a GitHub token"));
    prepare(owner, &task(1), round);
    let found = owner.parts.state.team(team).and_then(|team| team.round(round));
    owner.waiting_for(owner.parts.state.team(team).unwrap(), found.unwrap())
}

/// A daemon home with `state` and the token of a GitHub account, so that the owner has its autopilot.
fn home_with_token(state: &State) -> Daemon {
    let daemon = Daemon::new(state, None);
    let secrets = daemon.home.root().join("secrets.toml");
    std::fs::write(&secrets, "github_token = \"ghp_test_token_0123\"\n").unwrap();
    std::fs::set_permissions(&secrets, std::os::unix::fs::PermissionsExt::from_mode(0o600)).unwrap();
    daemon
}

/// The autopilot of `owner`, with the real texts.
async fn pulls_of(owner: &crate::daemon::owner::Owner) -> Option<crate::daemon::owner::Pulls> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../prompts");
    let language = owner.parts.settings.config.language();
    crate::daemon::owner::Pulls::new(
        Arc::new(
            agent_runtime::autopilot_prompts::AutopilotPrompts::load(&root, language)
                .await
                .unwrap(),
        ),
        Arc::new(
            agent_runtime::checks_prompts::ChecksPrompts::load(&root, language)
                .await
                .unwrap(),
        ),
        &owner.parts.settings,
    )
    .unwrap()
}

// The member that keeps the item counts only in a part round: a round of the whole item, with the same lead and the
// same member between two runs, waits for nothing and is still a defect.
#[tokio::test]
async fn a_round_without_a_part_waits_for_no_member_between_its_runs() {
    let (mut state, team, round) = round_for(1, TaskStatus::Done);
    {
        let mut found = state.team_mut(&team).unwrap();
        // The member worked in the round, so it is one of its members, and the item stays given to it.
        found
            .begin_run(
                &task(2),
                agent_core::state::NewRun {
                    id: crate::fixtures::run(3),
                    at: at(11),
                    session: None,
                    rounds: vec![round.clone()],
                },
            )
            .unwrap();
        let (_, mut member) = found.find_mut(&task(2)).unwrap();
        member.transition(TaskStatus::Running).unwrap();
        member.end_run(at(12), LEAD_DONE).unwrap();
        member.transition(TaskStatus::Waiting).unwrap();
    }
    let daemon = home_with_token(&state);
    let (mut owner, _sinks) = daemon.bare_owner().await;

    let found = waiting_after(&mut owner, &round, &team, |_, _, _| {}).await;

    assert_eq!(found, Some(Waiting::Defect));
}

/// Puts the task `number` of `team` to waiting for a message, as a task that ended a run is: queued, then running, then
/// waiting.
fn make_wait(state: &mut State, team: &TeamId, number: u32) {
    let mut found = state.team_mut(team).unwrap();
    let (_, mut member) = found.find_mut(&task(number)).unwrap();
    if member.status() == TaskStatus::Queued {
        member.transition(TaskStatus::Running).unwrap();
    }
    member.transition(TaskStatus::Waiting).unwrap();
}

/// The round of item 1.1 given to `claude-1` (task 2), whose lead (task 1) ended its run and is done and in which
/// every other task waits for a message, with a split of two parts bound to the round when `split` is set.
fn round_of_waiting_tasks(split: bool) -> (State, TeamId, RoundId) {
    let (mut state, team, round) = idle_round(in_progress(), LEAD_DONE);
    for number in [2, 3, 4] {
        make_wait(&mut state, &team, number);
    }
    if split {
        let plan = serde_json::json!({"parent": "1.1", "parts": [
            {"title": "A part", "delivers": "A guide", "checks": "Read it", "depends_on": []},
            {"title": "Another part", "delivers": "Another guide", "checks": "Read it", "depends_on": ["a"]},
        ]});
        state
            .team_mut(&team)
            .unwrap()
            .activate_split(serde_json::from_value(plan).unwrap())
            .unwrap();
    }
    (state, team, round)
}

// A task that waits for a message, an assignment or a merge has no run to come: a round whose tasks all wait, the lead
// among them, waits for nothing, and that is the defect.
#[tokio::test]
async fn a_round_whose_tasks_all_wait_for_a_message_waits_for_nothing() {
    let (mut state, team, round) = round_for(1, TaskStatus::Waiting);
    for number in [2, 3, 4] {
        make_wait(&mut state, &team, number);
    }
    let daemon = home_with_token(&state);
    let (mut owner, _sinks) = daemon.bare_owner().await;

    let found = waiting_after(&mut owner, &round, &team, |_, _, _| {}).await;

    assert_eq!(found, Some(Waiting::Defect));
}

// The member the item is given to, queued behind a busy agent or the limit of runs, has not run in the round yet: its
// run is the one the round waits for, whether the item is split or not.
#[tokio::test]
async fn a_round_whose_assignee_is_queued_for_its_first_run_waits_for_it() {
    for split in [false, true] {
        let (mut state, team, round) = round_of_waiting_tasks(split);
        {
            let mut found = state.team_mut(&team).unwrap();
            let (_, mut member) = found.find_mut(&task(2)).unwrap();
            member.transition(TaskStatus::Queued).unwrap();
        }
        let daemon = home_with_token(&state);
        let (mut owner, _sinks) = daemon.bare_owner().await;

        let found = waiting_after(&mut owner, &round, &team, |_, _, _| {}).await;

        assert_eq!(found, Some(Waiting::For(Obligation::QueuedRun)), "split: {split}");
    }
}

// A member that worked in the round and is queued again for its next run waits for that run: the round is idle, not
// at work, and says so.
#[tokio::test]
async fn a_round_whose_member_is_queued_again_after_a_run_in_it_waits_for_the_run() {
    let (mut state, team, round) = idle_round(in_progress(), LEAD_DONE);
    for number in [2, 4] {
        make_wait(&mut state, &team, number);
    }
    {
        let mut found = state.team_mut(&team).unwrap();
        found
            .begin_run(
                &task(3),
                NewRun {
                    id: run(3),
                    at: at(40),
                    session: None,
                    rounds: vec![round.clone()],
                },
            )
            .unwrap();
        let (_, mut member) = found.find_mut(&task(3)).unwrap();
        member.transition(TaskStatus::Running).unwrap();
        member.end_run(at(50), LEAD_DONE).unwrap();
        member.transition(TaskStatus::Queued).unwrap();
    }
    let daemon = home_with_token(&state);
    let (mut owner, _sinks) = daemon.bare_owner().await;

    let found = waiting_after(&mut owner, &round, &team, |_, _, _| {}).await;

    assert_eq!(found, Some(Waiting::For(Obligation::QueuedRun)));
}

// Rule: the wait of a round names who is obliged and since when: the lead whose run ended without a status owes the
// correction from the end of that run.
#[tokio::test]
async fn the_wait_line_of_a_round_names_who_owes_it_and_since_when() {
    let (state, team, round) = idle_round(in_progress(), RunEnd::NoStatus { exit_code: Some(1) });
    let team = state.team(&team).unwrap();

    let line = crate::daemon::owner::Owner::wait_text(
        team,
        team.round(&round).unwrap(),
        Waiting::For(Obligation::LeadCorrection),
    );

    assert_eq!(
        line,
        format!(
            "the lead's correction, owed by the lead since {}",
            agent_core::local_time::Zone::Utc.stamp(at(30))
        )
    );
}

// Rule: the wait of a round starts when that round went idle: a later run of the same task for another round is not a start
// of this one.
#[tokio::test]
async fn the_wait_starts_from_the_runs_of_this_round_only() {
    let (mut state, team, round) = idle_round(in_progress(), RunEnd::NoStatus { exit_code: Some(1) });
    {
        let mut found = state.team_mut(&team).unwrap();
        let other: RoundId = "round-20261119-090500-else".parse().unwrap();
        found
            .open_round(other.clone(), Some("1.2".parse().unwrap()), at(35))
            .unwrap();
        found
            .begin_run(
                &task(1),
                NewRun {
                    id: run(2),
                    at: at(40),
                    session: None,
                    rounds: vec![other],
                },
            )
            .unwrap();
        let (_, mut lead) = found.find_mut(&task(1)).unwrap();
        lead.end_run(at(50), RunEnd::NoStatus { exit_code: Some(1) }).unwrap();
    }
    let team = state.team(&team).unwrap();

    let line = crate::daemon::owner::Owner::wait_text(
        team,
        team.round(&round).unwrap(),
        Waiting::For(Obligation::LeadCorrection),
    );

    assert_eq!(
        line,
        format!(
            "the lead's correction, owed by the lead since {}",
            agent_core::local_time::Zone::Utc.stamp(at(30))
        )
    );
}

// Rule: the wait for the human starts when the question was asked, not when a run last ended.
#[tokio::test]
async fn the_wait_for_the_human_starts_when_the_question_was_asked() {
    let (mut state, team, round) = idle_round(in_progress(), RunEnd::NoStatus { exit_code: Some(1) });
    {
        let mut found = state.team_mut(&team).unwrap();
        let (_, mut lead) = found.find_mut(&task(1)).unwrap();
        let file =
            agent_core::config::AbsolutePath::try_from("/home/dev/agents/tasks/questions-1.md".to_owned()).unwrap();
        lead.ask_question(agent_core::state::Question::new(
            std::num::NonZeroU32::MIN,
            file,
            at(33),
        ))
        .unwrap();
    }
    let team = state.team(&team).unwrap();

    let line =
        crate::daemon::owner::Owner::wait_text(team, team.round(&round).unwrap(), Waiting::For(Obligation::Human));

    assert_eq!(
        line,
        format!(
            "an answer of the human, owed by the human since {}",
            agent_core::local_time::Zone::Utc.stamp(at(33))
        )
    );
}

#[tokio::test]
async fn the_wait_line_of_a_queued_round_behind_its_agents_run_names_the_agent_and_the_team() {
    let (mut state, team, round) = round_of_waiting_tasks(false);
    {
        let mut found = state.team_mut(&team).unwrap();
        let (_, mut member) = found.find_mut(&task(2)).unwrap();
        member.transition(TaskStatus::Queued).unwrap();
    }
    let other_team_id = team_id(9);
    let mut other_team = state.team(&team).unwrap().clone();
    let other_json = serde_json::to_string(&other_team)
        .unwrap()
        .replace(team.as_str(), other_team_id.as_str())
        .replace(task(1).as_str(), "task-20261111-153051-k3x9")
        .replace(task(2).as_str(), "task-20261111-153052-k3x9")
        .replace(task(3).as_str(), "task-20261111-153053-k3x9")
        .replace(task(4).as_str(), "task-20261111-153054-k3x9")
        .replace(round.as_str(), "round-20261119-090059-wait");
    other_team = serde_json::from_str(&other_json).unwrap();
    state.insert_team(other_team).unwrap();
    {
        let mut other = state.team_mut(&other_team_id).unwrap();
        let busy_task: TaskId = "task-20261111-153052-k3x9".parse().unwrap();
        let (_, mut busy_member) = other.find_mut(&busy_task).unwrap();
        busy_member.transition(TaskStatus::Running).unwrap();
    }
    let daemon = home_with_token(&state);
    let (mut owner, _sinks) = daemon.bare_owner().await;
    owner.set_pulls(pulls_of(&owner).await.expect("the fixture home has a GitHub token"));
    let found_team = owner.parts.state.team(&team).unwrap();
    let found_round = found_team.round(&round).unwrap();

    let line = owner.wait_line(found_team, found_round);

    assert_eq!(
        line,
        Some(format!(
            "the queued run of its task to start, owed by the harness since {}, after the run of claude-1 in {}",
            agent_core::local_time::Zone::Utc.stamp(at(30)),
            other_team_id
        ))
    );
}

#[tokio::test]
async fn a_local_queued_round_without_github_names_the_busy_agent_and_team_it_waits_for() {
    let (mut state, team, round) = round_of_waiting_tasks(false);
    {
        let mut found = state.team_mut(&team).unwrap();
        let (_, mut member) = found.find_mut(&task(2)).unwrap();
        member.transition(TaskStatus::Queued).unwrap();
    }
    let other_team_id = team_id(9);
    let mut other_team = state.team(&team).unwrap().clone();
    let other_json = serde_json::to_string(&other_team)
        .unwrap()
        .replace(team.as_str(), other_team_id.as_str())
        .replace(task(1).as_str(), "task-20261111-153051-k3x9")
        .replace(task(2).as_str(), "task-20261111-153052-k3x9")
        .replace(task(3).as_str(), "task-20261111-153053-k3x9")
        .replace(task(4).as_str(), "task-20261111-153054-k3x9")
        .replace(round.as_str(), "round-20261119-090059-wait");
    other_team = serde_json::from_str(&other_json).unwrap();
    state.insert_team(other_team).unwrap();
    {
        let mut other = state.team_mut(&other_team_id).unwrap();
        let busy_task: TaskId = "task-20261111-153052-k3x9".parse().unwrap();
        let (_, mut busy_member) = other.find_mut(&busy_task).unwrap();
        busy_member.transition(TaskStatus::Running).unwrap();
    }
    let daemon = Daemon::new(&state, None);
    let (owner, _sinks) = daemon.bare_owner().await;
    let found_team = owner.parts.state.team(&team).unwrap();
    let found_round = found_team.round(&round).unwrap();

    let line = owner.wait_line(found_team, found_round);

    assert_eq!(
        line,
        Some(format!(
            "the queued run of its task to start, owed by the harness since {}, after the run of claude-1 in {}",
            agent_core::local_time::Zone::Utc.stamp(at(30)),
            other_team_id
        ))
    );
}

#[tokio::test]
async fn a_round_waiting_for_free_investigators_keeps_its_team_alive() {
    let (mut state, team, round) = idle_round(in_progress(), LEAD_DONE);
    for n in [1, 2, 3, 4] {
        let (_, mut member) = state.find_task_mut(&task(n)).unwrap();
        if member.status() == TaskStatus::Queued {
            member.transition(TaskStatus::Running).unwrap();
            member.transition(TaskStatus::Waiting).unwrap();
        }
    }
    let daemon = Daemon::new(&state, None);
    for n in [1, 2, 3, 4] {
        let path = daemon.home.worktree_dir(&crate::fixtures::repo(), &task(n));
        std::fs::create_dir_all(&path).unwrap();
        std::fs::write(path.join("draft.txt"), "work").unwrap();
    }
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
    daemon.clock.advance(time::Duration::days(1));
    owner.check_idle_teams().await.unwrap();
    assert_eq!(owner.parts.state.teams().len(), 1);
    assert!(daemon.home.worktree_dir(&crate::fixtures::repo(), &task(1)).exists());
    assert_eq!(daemon.notifications(), Vec::<String>::new());
}

/// The task `number` runs for `round`, ends the run with `end` and is paused: what a refusal by the provider makes of
/// it. Its agent is refused until `until`, when there is a time.
fn paused_by(
    state: &mut State,
    team: &TeamId,
    round: &RoundId,
    number: u32,
    end: RunEnd,
    until: Option<agent_core::state::Timestamp>,
) -> agent_core::ids::AgentName {
    let agent = {
        let mut found = state.team_mut(team).unwrap();
        found
            .begin_run(
                &task(number),
                NewRun {
                    id: run(10 + number),
                    at: at(40),
                    session: None,
                    rounds: vec![round.clone()],
                },
            )
            .unwrap();
        let (_, mut member) = found.find_mut(&task(number)).unwrap();
        member.transition(TaskStatus::Running).unwrap();
        member.end_run(at(50), end).unwrap();
        member.transition(TaskStatus::Paused).unwrap();
        member.agent().agent_name()
    };
    if let Some(until) = until {
        state.keep_refused_until(agent.clone(), until);
    }
    agent
}

/// The human has a question open of the task `number`, which took part in `round`: a task that has not run for it is given a
/// finished run first.
fn question_of(state: &mut State, team: &TeamId, round: &RoundId, number: u32) {
    let mut found = state.team_mut(team).unwrap();
    let ran = found
        .find(&task(number))
        .is_some_and(|(_, member)| member.runs().iter().any(|run| run.rounds().contains(round)));
    if !ran {
        found
            .begin_run(
                &task(number),
                NewRun {
                    id: run(30 + number),
                    at: at(41),
                    session: None,
                    rounds: vec![round.clone()],
                },
            )
            .unwrap();
        let (_, mut member) = found.find_mut(&task(number)).unwrap();
        member.transition(TaskStatus::Running).unwrap();
        member.end_run(at(42), LEAD_DONE).unwrap();
        member.transition(TaskStatus::Done).unwrap();
    }
    let (_, mut member) = found.find_mut(&task(number)).unwrap();
    let file = agent_core::config::AbsolutePath::try_from("/home/dev/agents/tasks/questions-1.md".to_owned()).unwrap();
    member
        .ask_question(agent_core::state::Question::new(
            std::num::NonZeroU32::MIN,
            file,
            at(45),
        ))
        .unwrap();
}

/// What the owner says `round` waits for `later` seconds after its clock was at zero: the wait, its line, and the local
/// clock time of `until` as the line writes it.
async fn wait_of(
    state: &State,
    team: &TeamId,
    round: &RoundId,
    later: i64,
    until: agent_core::state::Timestamp,
) -> (Option<Waiting>, Option<String>, String) {
    let daemon = Daemon::new(state, None);
    daemon.clock.advance(time::Duration::seconds(later));
    let (owner, _sinks) = daemon.bare_owner().await;
    let found = owner.parts.state.team(team).unwrap();
    let round = found.round(round).unwrap();
    (
        owner.waiting_for(found, round),
        owner.wait_line(found, round),
        owner.parts.zone.clock_minute(until),
    )
}

/// The round of a review whose reviewer is `inv-codex`, task 4, paused by `end` until `until`; the lead's task 1 has a
/// question open when `asked`.
fn round_of_a_paused_reviewer(
    end: RunEnd,
    asked: bool,
    until: agent_core::state::Timestamp,
) -> (State, TeamId, RoundId, agent_core::ids::AgentName) {
    let phase = ItemPhase::InReview {
        pr: crate::fixtures::pr(7),
        reviews: vec![crate::fixtures::review("inv-codex", 4)],
    };
    let (mut state, team, round) = idle_round(phase, LEAD_DONE);
    let agent = paused_by(&mut state, &team, &round, 4, end, Some(until));
    if asked {
        question_of(&mut state, &team, &round, 1);
    }
    (state, team, round, agent)
}

// Rule (#233): a reviewer whose provider refused it is waited for at its provider, until the agent returns: the line names
// the agent and the time, and nobody is told the human owes anything while no question is open.
#[tokio::test]
async fn a_reviewer_paused_by_its_provider_is_waited_for_at_the_provider_until_it_returns() {
    for end in [RunEnd::RateLimited, RunEnd::QuotaExhausted { resets_at: None }] {
        let until = at(3_600);
        let (state, team, round, agent) = round_of_a_paused_reviewer(end.clone(), false, until);

        let (waiting, line, clock) = wait_of(&state, &team, &round, 60, until).await;

        assert_eq!(waiting, Some(Waiting::Provider), "{end:?}");
        assert_eq!(line, Some(format!("the provider of {agent} until {clock}")), "{end:?}");
    }
}

// Rule: an open question is still the human's to answer, so it is the human's line.
#[tokio::test]
async fn a_round_with_an_open_question_waits_for_the_human_though_its_reviewer_is_paused_by_its_provider() {
    let until = at(3_600);
    let (state, team, round, _agent) = round_of_a_paused_reviewer(RunEnd::RateLimited, true, until);

    let (waiting, line, _) = wait_of(&state, &team, &round, 60, until).await;

    assert_eq!(waiting, Some(Waiting::For(Obligation::Human)));
    assert!(line.is_some_and(|line| line.starts_with("an answer of the human")));
}

// Rule: a pause the human asked for is the human's: the last run was stopped, not refused.
#[tokio::test]
async fn a_reviewer_paused_by_the_human_still_waits_for_the_human() {
    let until = at(3_600);
    let (state, team, round, _agent) = round_of_a_paused_reviewer(RunEnd::Stopped, false, until);

    let (waiting, _, _) = wait_of(&state, &team, &round, 60, until).await;

    assert_eq!(waiting, Some(Waiting::For(Obligation::Human)));
}

// Rule: the provider's wait ends with the refusal: an agent that has returned is not waited for.
#[tokio::test]
async fn a_reviewer_whose_agent_has_returned_is_not_waited_for_at_its_provider() {
    let until = at(3_600);
    let (state, team, round, _agent) = round_of_a_paused_reviewer(RunEnd::RateLimited, false, until);

    let (waiting, _, _) = wait_of(&state, &team, &round, 3_601, until).await;

    assert_ne!(waiting, Some(Waiting::Provider));
}

// Rule: the same holds for a lead and a member of the team, with and without an open question.
#[tokio::test]
async fn a_lead_and_a_member_paused_by_their_provider_are_waited_for_at_the_provider() {
    for (number, other) in [(1, 2), (2, 1)] {
        for asked in [false, true] {
            let until = at(7_200);
            let (mut state, team, round) = round_for(number, TaskStatus::Queued);
            let agent = paused_by(&mut state, &team, &round, number, RunEnd::RateLimited, Some(until));
            if asked {
                question_of(&mut state, &team, &round, other);
            }

            let (waiting, line, clock) = wait_of(&state, &team, &round, 60, until).await;

            if asked {
                assert_eq!(waiting, Some(Waiting::For(Obligation::Human)), "task {number}");
            } else {
                assert_eq!(waiting, Some(Waiting::Provider), "task {number}");
                assert_eq!(
                    line,
                    Some(format!("the provider of {agent} until {clock}")),
                    "task {number}"
                );
            }
        }
    }
}
