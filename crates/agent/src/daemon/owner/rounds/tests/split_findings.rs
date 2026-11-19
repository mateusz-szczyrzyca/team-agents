//! Two items of one pull request that held its findings apart, as on 19.11.2026: the daemon repairs them at its start,
//! asks the reviewers again, says what the round waits for, and records a fix response for every item of the pull
//! request.
use std::sync::{Arc, Mutex};

use agent_core::decide::wait::{Obligation, Waiting};
use agent_core::ids::{RoundId, TaskId, TeamId};
use agent_core::log::LogEvent;
use agent_core::state::{ExplanationStatus, ItemPhase, NewRun, RunEnd, State, Timestamp, reconcile};
use agent_core::status::{ReviewVerdict, RunOutcome, TaskStatus};
use tracing::instrument::WithSubscriber;
use tracing_subscriber::layer::{Context, SubscriberExt};
use tracing_subscriber::{Layer, Registry};

use crate::daemon::owner::fixture::Daemon;
use crate::daemon::owner::teams::Look;
use crate::daemon::owner::wakes::Wake;
use crate::daemon::owner::{Owner, rounds::Pulls};
use crate::daemon::team::Why;

/// The state of the evidence: items 1.1 and 2.1 on pull request 132, finding 28 explained in 1.1 only.
fn evidence() -> State {
    State::from_json(include_str!(
        "../../../../../../agent-core/tests/fixtures/state-split-explanations.json"
    ))
    .unwrap()
}

/// The report that stopped the daemon: right for item 1.1, wrong for item 2.1 before the repair.
const REPORT: &str = include_str!("../../../../../../agent-core/tests/fixtures/report-split-explanations.txt");

/// The team of the evidence.
fn team() -> TeamId {
    "team-20261119-131420-gt1d".parse().unwrap()
}

/// The lead of the team.
fn lead() -> TaskId {
    "task-20261119-131420-ohs0".parse().unwrap()
}

/// The reviewer whose report was refused; it waits, with its review still pending.
fn refused() -> TaskId {
    "task-20261119-153246-oe7a".parse().unwrap()
}

/// The reviewer whose run the stop of the daemon interrupted.
fn interrupted() -> TaskId {
    "task-20261119-170919-quvr".parse().unwrap()
}

/// The round of item 1.1.
fn round() -> RoundId {
    "round-20261119-131420-xqqr".parse().unwrap()
}

/// The evidence with the ledgers of its two items already brought together.
fn repaired() -> State {
    let mut state = evidence();
    {
        let mut team = state.team_mut(&team()).unwrap();
        let ids = ["1.1".parse().unwrap(), "2.1".parse().unwrap()];
        let mut ledgers: Vec<_> = ids
            .iter()
            .map(|id| team.item(*id).unwrap().findings().unwrap().clone())
            .collect();
        reconcile(&mut ledgers);
        for (id, ledger) in ids.iter().zip(ledgers) {
            *team.item_mut(*id).unwrap().findings_mut().unwrap() = ledger;
        }
    }
    state
}

/// The `event` field of an event logged, and its `finding` field when it has one.
type Logged = (String, Option<String>);

/// The `event` and `finding` fields of every event logged.
#[derive(Clone, Default)]
struct Events(Arc<Mutex<Vec<Logged>>>);

/// Reads the two fields of one event.
#[derive(Default)]
struct Fields {
    /// The `event` field.
    event: String,
    /// The `finding` field.
    finding: Option<String>,
}

impl tracing::field::Visit for Fields {
    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        self.record_debug(field, &value);
    }

    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        let text = format!("{value:?}").trim_matches('"').to_owned();
        match field.name() {
            "event" => self.event = text,
            "finding" => self.finding = Some(text),
            _ => {}
        }
    }
}

impl Layer<Registry> for Events {
    fn on_event(&self, event: &tracing::Event<'_>, _context: Context<'_, Registry>) {
        let mut fields = Fields::default();
        event.record(&mut fields);
        self.0.lock().unwrap().push((fields.event, fields.finding));
    }
}

impl Events {
    /// The findings named by the events that tell a repair at the start.
    fn repaired(&self) -> Vec<String> {
        self.0
            .lock()
            .unwrap()
            .iter()
            .filter(|(event, _)| event == LogEvent::RestartCorrected.as_str())
            .filter_map(|(_, finding)| finding.clone())
            .collect()
    }
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
async fn pulls_of(owner: &Owner) -> Pulls {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../prompts");
    let language = owner.parts.settings.config.language();
    Pulls::new(
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
    .unwrap()
}

/// Runs the start's recovery of an owner of `state` and returns the owner and what was logged.
async fn recovered(state: &State) -> (Daemon, Owner, Events) {
    let daemon = home_with_token(state);
    let (owner, sinks) = daemon.bare_owner().await;
    let pulls = pulls_of(&owner).await;
    let mut owner = owner.with_pulls(pulls);
    let events = Events::default();
    let subscriber = tracing_subscriber::registry().with(events.clone());
    owner.recover().with_subscriber(subscriber).await.unwrap();
    drop(sinks);
    (daemon, owner, events)
}

/// The status of the explanation of finding `number` in item `item`.
fn explanation_of(state: &State, item: &str, number: u32) -> Option<ExplanationStatus> {
    state
        .team(&team())
        .unwrap()
        .item(item.parse().unwrap())
        .unwrap()
        .findings()
        .unwrap()
        .disputes()
        .unwrap()
        .explanations
        .get(&std::num::NonZeroU32::new(number).unwrap())
        .map(|explanation| explanation.status)
}

#[tokio::test]
async fn a_start_repairs_the_evidence_with_one_event_and_the_refused_report_is_then_read_for_both_items() {
    let (_daemon, owner, events) = recovered(&evidence()).await;

    assert_eq!(events.repaired(), vec!["28".to_owned()]);
    let state = &owner.parts.state;
    assert_eq!(explanation_of(state, "2.1", 28), Some(ExplanationStatus::Pending));
    for item in ["1.1", "2.1"] {
        let ledger = state
            .team(&team())
            .unwrap()
            .item(item.parse().unwrap())
            .unwrap()
            .findings()
            .unwrap();
        let report = ledger.parse_report(REPORT, ReviewVerdict::RequestChanges).unwrap();
        assert_eq!(report.answers.len(), 11, "item {item}");
    }
}

#[tokio::test]
async fn a_start_whose_items_agree_repairs_nothing_and_logs_no_repair() {
    let state = repaired();

    let (_daemon, owner, events) = recovered(&state).await;

    assert_eq!(events.repaired(), Vec::<String>::new());
    for item in ["1.1", "2.1"] {
        let id = item.parse().unwrap();
        assert_eq!(
            owner.parts.state.team(&team()).unwrap().item(id).unwrap().findings(),
            state.team(&team()).unwrap().item(id).unwrap().findings()
        );
    }
}

#[tokio::test]
async fn after_a_restart_a_reviewer_whose_report_was_not_taken_is_asked_again() {
    let (_daemon, owner, _events) = recovered(&evidence()).await;

    let status = |task: &TaskId| owner.parts.state.find_task(task).unwrap().member.status();
    assert_eq!(status(&refused()), TaskStatus::Queued);
    assert_eq!(owner.reasons.get(&refused()), Some(&Why::Review));
    assert_eq!(status(&interrupted()), TaskStatus::Queued);
}

#[tokio::test]
async fn a_round_whose_reviewers_wait_with_reviews_pending_waits_for_the_review_not_for_the_human() {
    let mut state = repaired();
    {
        let (_, mut member) = state.find_task_mut(&interrupted()).unwrap();
        member
            .end_run(
                "2026-11-19T17:20:00Z".parse().unwrap(),
                RunEnd::Reported {
                    outcome: RunOutcome::Done,
                },
            )
            .unwrap();
        member.transition(TaskStatus::Waiting).unwrap();
    }
    let daemon = home_with_token(&state);
    let (owner, _sinks) = daemon.bare_owner().await;
    let pulls = pulls_of(&owner).await;
    let owner = owner.with_pulls(pulls);

    let team = owner.parts.state.team(&team()).unwrap();
    let round = team.rounds().iter().find(|found| *found.id() == round()).unwrap();

    assert_eq!(owner.waiting_for(team, round), Some(Waiting::For(Obligation::Review)));
}

/// The repaired evidence after both reviewers asked for changes: both items are fixed again on pull request 132, item
/// 1.1 in its third fix pass and item 2.1, which joined at the first, in its second, both with the pull request's ledger
/// as a joining item takes it; the lead has answered with `message` in a run that ended after the reviews.
fn fixing_again(message: &str) -> State {
    let mut state = repaired();
    let at: Timestamp = "2026-11-19T17:40:00Z".parse().unwrap();
    {
        let mut team = state.team_mut(&team()).unwrap();
        let ledger = team.item("1.1".parse().unwrap()).unwrap().findings().unwrap().clone();
        for item in ["1.1", "2.1"] {
            let id = item.parse().unwrap();
            let mut progress = team.item_mut(id).unwrap();
            for task in [interrupted(), refused()] {
                progress
                    .submit_review(&task, ReviewVerdict::RequestChanges, at)
                    .unwrap();
            }
            let ItemPhase::InReview { pr, .. } = progress.phase().clone() else {
                panic!("item {item} of the evidence is in review")
            };
            progress
                .advance(ItemPhase::Fixing {
                    assignee: "lead-ada".parse().unwrap(),
                    branch: pr.branch.clone(),
                    pr,
                })
                .unwrap();
            let findings = progress.findings_mut().unwrap();
            *findings = ledger.clone();
            findings.verification_mut().unwrap().lead_message = Some(message.to_owned());
        }
    }
    {
        let (_, mut member) = state.find_task_mut(&lead()).unwrap();
        member.transition(TaskStatus::Queued).unwrap();
        member.transition(TaskStatus::Running).unwrap();
    }
    state
        .team_mut(&team())
        .unwrap()
        .begin_run(
            &lead(),
            NewRun {
                id: "run-20261119-174500-fix3".parse().unwrap(),
                at: "2026-11-19T17:45:00Z".parse().unwrap(),
                session: None,
                rounds: vec![round()],
            },
        )
        .unwrap();
    {
        let (_, mut member) = state.find_task_mut(&lead()).unwrap();
        member
            .end_run(
                "2026-11-19T17:55:00Z".parse().unwrap(),
                RunEnd::Reported {
                    outcome: RunOutcome::Done,
                },
            )
            .unwrap();
        member.transition(TaskStatus::Waiting).unwrap();
    }
    state
}

/// The lead's answer to the third fix pass: finding 20 is explained, the other blocking findings are fixed.
const EXPLAINED_20: &str = "FINDING 3: FIXED\nFINDING 18: FIXED\nFINDING 19: FIXED\n\
FINDING 20: EXPLAINED — the outcome is logged one level up — ünïcode\nFINDING 27: FIXED\nFINDING 28: FIXED\n\nSTATUS: DONE";

#[tokio::test]
async fn a_fix_response_explaining_a_finding_makes_the_explanation_pending_in_every_item_of_the_pull_request() {
    let state = fixing_again(EXPLAINED_20);
    let daemon = home_with_token(&state);
    let (owner, _sinks) = daemon.bare_owner().await;
    let pulls = pulls_of(&owner).await;
    let mut owner = owner.with_pulls(pulls.clone());

    owner
        .check_fix_responses(&pulls, &team(), &mut Look::default())
        .unwrap();

    let state = &owner.parts.state;
    assert_eq!(explanation_of(state, "1.1", 20), Some(ExplanationStatus::Pending));
    assert_eq!(explanation_of(state, "2.1", 20), Some(ExplanationStatus::Pending));
    let disputes = |item: &str| {
        state
            .team(&team())
            .unwrap()
            .item(item.parse().unwrap())
            .unwrap()
            .findings()
            .unwrap()
            .disputes()
            .unwrap()
            .clone()
    };
    assert_eq!(disputes("1.1"), disputes("2.1"));
    assert_eq!(disputes("2.1").answered_pass, 3);
}

#[tokio::test]
async fn the_lead_is_not_woken_once_the_fix_response_of_every_item_of_the_pull_request_is_taken() {
    let state = fixing_again(EXPLAINED_20);
    let daemon = home_with_token(&state);
    let (owner, _sinks) = daemon.bare_owner().await;
    let pulls = pulls_of(&owner).await;
    let mut owner = owner.with_pulls(pulls.clone());
    owner
        .check_fix_responses(&pulls, &team(), &mut Look::default())
        .unwrap();

    let wake = owner.wake_member(&team(), &lead(), &mut Look::default()).await.unwrap();

    assert_eq!(wake, Wake::Held);
}
