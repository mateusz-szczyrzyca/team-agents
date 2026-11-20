//! Stop the real owner precisely between settling and scheduling, then boot the daemon from its saved state.

use std::path::Path;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use agent_core::state::{ItemPhase, MergeMode};
use agent_core::status::TaskStatus;
use agent_runtime::agentbus::AgentBus;
use agent_runtime::harness_prompts::HarnessPrompts;
use agent_runtime::history::Writer;
use agent_runtime::process::Supervisor;
use agent_runtime::prompts::PromptLoader;
use agent_runtime::questions_prompts::QuestionsPrompts;
use agent_runtime::run::{EngineParts, RunEngine, RunTimings};
use agent_runtime::settings::Settings;
use agent_runtime::store::StateStore;
use agent_runtime::team_prompts::TeamPrompts;
use rand::{SeedableRng, rngs::StdRng};
use tokio::sync::{broadcast, mpsc};
use tokio_util::sync::CancellationToken;

use super::rig::{PARALLEL, Rig, investigator, normal};
use crate::daemon::owner::teams::Look;
use crate::daemon::owner::{MAX_RUNS, Owner, OwnerParts, Pulls};
use agent_runtime::autopilot_prompts::AutopilotPrompts;
use agent_runtime::checks_prompts::ChecksPrompts;

/// Holds channel consumers while the test drives the same owner methods as the daemon loop.
struct Stepped {
    /// The production owner, with real engine, store and history.
    owner: Owner,
    /// The history writer to drain before restarting.
    writer: Option<Writer>,
    /// Notices are retained until settlement completes.
    _notices: mpsc::Receiver<crate::daemon::notifier::Delivery>,
    /// Events are retained until settlement completes.
    _events: broadcast::Receiver<agent_core::protocol::Event>,
}

/// Open a production owner without its scheduling loop, so the crash boundary requires no timing race.
async fn stepped(rig: &Rig) -> Stepped {
    let store = Arc::new(StateStore::open(rig.home.clone()).await.unwrap());
    let settings = Arc::new(Settings::load(&rig.home).await.unwrap());
    let prompts = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../prompts");
    let language = settings.config.language();
    let texts = Arc::new(HarnessPrompts::load(&prompts, language).await.unwrap());
    let opened = crate::daemon::history::open(&settings, &rig.home).await;
    let engine = RunEngine::new(EngineParts {
        store: Arc::clone(&store),
        texts: Arc::clone(&texts),
        config: Arc::new(settings.config.clone()),
        secrets: Arc::new(settings.secrets.clone()),
        supervisor: Supervisor::new(MAX_RUNS),
        clock: rig.history_test_clock(),
        local_offset: time::UtcOffset::UTC,
        timings: RunTimings {
            poll: Duration::from_millis(20),
            hang: Duration::from_secs(20),
            stop_grace: Duration::from_secs(5),
        },
        agent_binary_dir: agent_core::config::AbsolutePath::from_str("/opt/bin").unwrap(),
    })
    .with_history(opened.history.clone());
    let (notices, deliveries) = mpsc::channel(64);
    let (events, kept) = broadcast::channel(64);
    let pulls = Pulls::new(
        Arc::new(AutopilotPrompts::load(&prompts, language).await.unwrap()),
        Arc::new(ChecksPrompts::load(&prompts, language).await.unwrap()),
        &settings,
    )
    .unwrap()
    .unwrap();
    let parts = OwnerParts {
        state: store.load().await.unwrap(),
        team_texts: Arc::new(TeamPrompts::load(&prompts, language).await.unwrap()),
        question_texts: Arc::new(QuestionsPrompts::load(&prompts, language).await.unwrap()),
        rules: Arc::new(PromptLoader::load(&prompts, language).await.unwrap()),
        bus: AgentBus::new(
            settings.config.binaries().agentbus.clone(),
            rig.home.absolute_root().clone(),
        ),
        opens: None,
        engine,
        clock: rig.history_test_clock(),
        rng: StdRng::seed_from_u64(88),
        notices,
        events,
        local_offset: time::UtcOffset::UTC,
        tick: Duration::from_millis(25),
        store,
        notifier: agent_runtime::notify::Notifier::from_config(
            settings.config.notifications(),
            &rig.home,
            &settings.secrets,
        ),
        free_space: agent_runtime::disk::SharedFreeSpace::system(),
        settings,
        texts,
    };
    Stepped {
        owner: Owner::new(parts, CancellationToken::new())
            .with_history(opened.history)
            .with_pulls(pulls),
        writer: opened.writer,
        _notices: deliveries,
        _events: kept,
    }
}

/// The two persisted crash boundaries of recovery.
#[derive(Clone, Copy, Debug)]
enum Boundary {
    /// The close was saved; a failed bus send prevented opening the fresh round.
    Observed,
    /// The fresh round and queued member were saved; no CLI has started yet.
    Recovered,
}

/// Starts a prompt-driven accepted claim and stops its daemon before any closure is observed.
async fn accepted_claim() -> (
    Rig,
    agent_core::ids::TeamId,
    Vec<agent_core::ids::TaskId>,
    agent_core::ids::RoundId,
) {
    let mut rig = Rig::new(
        &[
            investigator(
                "inv-lead",
                "sleep,integrate,delivery-from-prompt,delivery-return-lead,sleep",
            ),
            normal("claude-1", "commit-clean"),
            normal("codex-1", "delivery-held,delivery-return-work"),
            investigator("inv-r1", "done:VERDICT: APPROVE\\nSUMMARY: fine"),
            investigator("inv-r2", "done:VERDICT: APPROVE\\nSUMMARY: fine"),
        ],
        5,
        true,
    )
    .await;
    rig.start().await;
    let (team, tasks) = rig
        .run(
            &["inv-lead", "claude-1", "codex-1"],
            PARALLEL,
            MergeMode::NoMerge,
            Some("inv-lead"),
        )
        .await;
    rig.item_where("1.1", "the first PR to be approved", |phase| {
        matches!(phase, ItemPhase::Approved { .. })
    })
    .await;
    std::fs::write(rig.home.root().join("delivery-release"), "release").unwrap();
    let accepted = rig
        .state_where("the prompt-driven claim to be accepted", |state| {
            state
                .team(&team)
                .unwrap()
                .round_of("2.1".parse().unwrap())
                .unwrap()
                .delivered_by()
                .is_some()
        })
        .await;
    let old = accepted
        .team(&team)
        .unwrap()
        .round_of("2.1".parse().unwrap())
        .unwrap()
        .id()
        .clone();
    rig.stop().await.unwrap();
    (rig, team, tasks, old)
}

#[tokio::test]
async fn delivery_restart_at_each_recovery_boundary_publishes_and_merges_the_items_own_pr() {
    for boundary in [Boundary::Observed, Boundary::Recovered] {
        let (mut rig, team, tasks, old) = accepted_claim().await;
        rig.github.closed_by_a_human(1);
        let mut step = stepped(&rig).await;
        let pulls = step.owner.pulls.clone().unwrap();
        if matches!(boundary, Boundary::Observed) {
            step.owner.parts.bus = AgentBus::new(
                rig.home
                    .root()
                    .join("missing-agentbus")
                    .to_str()
                    .unwrap()
                    .parse()
                    .unwrap(),
                rig.home.absolute_root().clone(),
            );
        }
        let mut look = Look::default();
        let result = step.owner.poll_deliveries(&pulls, &team, &mut look).await;
        match boundary {
            Boundary::Observed => assert!(matches!(
                result,
                Err(crate::daemon::owner::OwnerError::Delivery(
                    super::DeliveryDaemonError::Bus(_)
                ))
            )),
            Boundary::Recovered => result.unwrap(),
        }
        let saved = rig.state();
        let found = saved.team(&team).unwrap();
        assert!(found.round(&old).unwrap().delivery_closed_at().is_some());
        assert_eq!(
            found
                .rounds()
                .iter()
                .filter(|r| r.item() == Some("2.1".parse().unwrap()))
                .count(),
            match boundary {
                Boundary::Observed => 1,
                Boundary::Recovered => 2,
            }
        );
        assert_eq!(rig.runs(&tasks[2]), 1);
        drop(step.owner);
        crate::daemon::history::close(step.writer).await;
        // A reopening after observation never restores the claim.
        rig.github.reopened_by_a_human(1);
        rig.github.merged_by_a_human(1);
        rig.start().await;
        rig.item_where("2.1", "the returned item's own PR to be approved", |phase| {
            matches!(phase, ItemPhase::Approved { .. })
        })
        .await;
        let state = rig.state();
        let found = state.team(&team).unwrap();
        let fresh = found.round_of("2.1".parse().unwrap()).unwrap();
        assert_ne!(fresh.id(), &old);
        assert_eq!(fresh.delivered_by(), None);
        assert_eq!(
            found.find(&tasks[2]).unwrap().1.runs().last().unwrap().rounds(),
            &[fresh.id().clone()],
            "{boundary:?}"
        );
        assert_eq!(rig.github.pulls().len(), 2);
        rig.github.merged_by_a_human(2);
        rig.state_where("the team to finish both items", |state| {
            state.team(&team).unwrap().find(&tasks[0]).unwrap().1.status() == TaskStatus::Done
        })
        .await;
        let state = rig.state();
        rig.stop().await.unwrap();
        assert!(
            state
                .team(&team)
                .unwrap()
                .items()
                .iter()
                .all(|item| matches!(item.phase(), ItemPhase::Done { .. }))
        );
        assert!(state.team(&team).unwrap().rounds().iter().all(|round| !round.is_open()));
        assert_eq!(rig.kept_runs(&tasks[2]), 2);
    }
}

#[tokio::test]
async fn delivery_reopened_before_observation_keeps_the_claim_and_completes_on_merge() {
    let mut rig = Rig::new(
        &[
            investigator("inv-lead", "sleep,integrate,delivery-from-prompt,sleep"),
            normal("claude-1", "commit-clean"),
            normal("codex-1", "delivery-held"),
            investigator("inv-r1", "done:VERDICT: APPROVE\\nSUMMARY: fine"),
            investigator("inv-r2", "done:VERDICT: APPROVE\\nSUMMARY: fine"),
        ],
        5,
        true,
    )
    .await;
    rig.start().await;
    let (team, tasks) = rig
        .run(
            &["inv-lead", "claude-1", "codex-1"],
            PARALLEL,
            MergeMode::NoMerge,
            Some("inv-lead"),
        )
        .await;
    rig.item_where("1.1", "the first PR to be approved", |phase| {
        matches!(phase, ItemPhase::Approved { .. })
    })
    .await;
    std::fs::write(rig.home.root().join("delivery-release"), "release").unwrap();
    let accepted = rig
        .state_where("the delivery to be accepted", |state| {
            state
                .team(&team)
                .unwrap()
                .round_of("2.1".parse().unwrap())
                .unwrap()
                .delivered_by()
                .is_some()
        })
        .await;
    let old = accepted
        .team(&team)
        .unwrap()
        .round_of("2.1".parse().unwrap())
        .unwrap()
        .clone();
    rig.stop().await.unwrap();
    rig.github.closed_by_a_human(1);
    rig.github.reopened_by_a_human(1);
    let mut step = stepped(&rig).await;
    let pulls = step.owner.pulls.clone().unwrap();
    let mut look = Look::default();
    step.owner.poll_deliveries(&pulls, &team, &mut look).await.unwrap();
    assert_eq!(
        step.owner
            .parts
            .state
            .team(&team)
            .unwrap()
            .round_of("2.1".parse().unwrap()),
        Some(&old)
    );
    assert!(!look.changed);
    drop(step.owner);
    crate::daemon::history::close(step.writer).await;
    rig.github.merged_by_a_human(1);
    rig.start().await;
    let done = rig
        .state_where("the unchanged claim to complete at its merge", |state| {
            state.team(&team).unwrap().find(&tasks[0]).unwrap().1.status() == TaskStatus::Done
        })
        .await;
    rig.stop().await.unwrap();
    let found = done.team(&team).unwrap();
    let round = found.round(old.id()).unwrap();
    assert_eq!(round.delivery_closed_at(), None);
    assert_eq!(round.ended(), Some(agent_core::state::RoundEnd::Merged));
    assert!(
        found
            .items()
            .iter()
            .all(|item| matches!(item.phase(), ItemPhase::Done { .. }))
    );
    assert_eq!(found.find(&tasks[2]).unwrap().1.runs().len(), 1);
    assert_eq!(rig.github.pulls().len(), 1);
}
