//! Stop the real owner precisely between settling and scheduling, then boot the daemon from its saved state.

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use agent_core::history::RunWhy;
use agent_core::protocol::{AgentList, Client, Command, Reply, RunRequest};
use agent_core::state::{MergeMode, RunEnd, TaskListText};
use agent_core::status::TaskStatus;
use agent_runtime::agentbus::AgentBus;
use agent_runtime::harness_prompts::HarnessPrompts;
use agent_runtime::history::{Reader, Writer};
use agent_runtime::process::Supervisor;
use agent_runtime::prompts::PromptLoader;
use agent_runtime::questions_prompts::QuestionsPrompts;
use agent_runtime::run::{EngineParts, RunEngine, RunTimings};
use agent_runtime::settings::Settings;
use agent_runtime::store::StateStore;
use agent_runtime::team_prompts::TeamPrompts;
use rand::{SeedableRng, rngs::StdRng};
use tokio::sync::{broadcast, mpsc, oneshot};
use tokio_util::sync::CancellationToken;

use super::rig::{PATIENCE, Rig, investigator, normal};
use crate::daemon::owner::{MAX_RUNS, Message, Owner, OwnerParts};

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
    })
    .with_history(opened.history.clone());
    let (notices, deliveries) = mpsc::channel(64);
    let (events, kept) = broadcast::channel(64);
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
        tick: Duration::from_millis(25),
        store,
        settings,
        texts,
    };
    Stepped {
        owner: Owner::new(parts, CancellationToken::new()).with_history(opened.history),
        writer: opened.writer,
        _notices: deliveries,
        _events: kept,
    }
}

#[tokio::test]
async fn restart_after_the_saved_turn_limit_resumes_once_and_keeps_the_budget() {
    for (scenario, expected) in [
        ("turn-limit,done", TaskStatus::Done),
        ("turn-limit", TaskStatus::Failed),
    ] {
        let mut rig = Rig::new(
            &[
                normal("claude-1", scenario),
                investigator("inv-r1", "done"),
                investigator("inv-r2", "done"),
            ],
            5,
            false,
        )
        .await;
        rig.pin_turn_limit_clock();
        let path = rig.home.config_file();
        let config = std::fs::read_to_string(&path).unwrap();
        std::fs::write(path, format!("{config}\n[limits]\nturn_limit_resumes = 1\n")).unwrap();
        let mut step = stepped(&rig).await;
        let (reply, receive) = oneshot::channel();
        step.owner
            .handle(Message::Request {
                client: Client::Cli,
                command: Command::Run(RunRequest {
                    repo: "example".parse().unwrap(),
                    checkout: rig.repo.to_str().unwrap().parse().unwrap(),
                    agents: AgentList::try_from(vec!["claude-1".parse().unwrap()]).unwrap(),
                    task_list: TaskListText::new("Finish the work.".to_owned()).unwrap(),
                    coordinator: None,
                    merge: MergeMode::Autopilot,
                }),
                reply,
            })
            .await
            .unwrap();
        let Reply::Started { team, tasks } = receive.await.unwrap().unwrap() else {
            panic!("team did not start")
        };
        step.owner.start_queued().await.unwrap();
        let joined = tokio::time::timeout(PATIENCE, step.owner.runs.join_next_with_id())
            .await
            .unwrap()
            .unwrap();
        step.owner.settle_joined(joined).await.unwrap();
        let saved = rig.state();
        assert_eq!(saved.find_task(&tasks[0]).unwrap().member.status(), TaskStatus::Queued);
        assert_eq!(rig.runs(&tasks[0]), 1);
        // Drop the owner before it can schedule: only its durable state is handed to the new daemon.
        drop(step.owner);
        crate::daemon::history::close(step.writer).await;
        rig.start().await;
        rig.state_where("resumed terminal status", |s| {
            s.find_task(&tasks[0]).unwrap().member.status() == expected
        })
        .await;
        rig.stop().await.unwrap();
        assert_eq!(rig.runs(&tasks[0]), 2);
        let rows = Reader::open(&rig.home.history_file())
            .unwrap()
            .runs_of_team(&team)
            .unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].end.as_ref().unwrap().value.end, RunEnd::TurnLimit);
        assert_eq!(rows[1].start.value.why, RunWhy::TurnLimit);
        assert_eq!(rows[0].start.value.session, rows[1].start.value.session);
        assert_eq!(
            rig.notifications().iter().filter(|n| n.contains("turn limit")).count(),
            usize::from(expected == TaskStatus::Failed)
        );
    }
}

#[tokio::test]
async fn turn_limit_without_a_session_fails_with_its_own_notice() {
    use crate::daemon::notice::Notice;

    let mut rig = Rig::new(
        &[
            normal("claude-1", "turn-limit"),
            investigator("inv-r1", "done"),
            investigator("inv-r2", "done"),
        ],
        5,
        false,
    )
    .await;
    rig.pin_turn_limit_clock();
    let mut step = stepped(&rig).await;
    let (reply, receive) = oneshot::channel();
    step.owner
        .handle(Message::Request {
            client: Client::Cli,
            command: Command::Run(RunRequest {
                repo: "example".parse().unwrap(),
                checkout: rig.repo.to_str().unwrap().parse().unwrap(),
                agents: AgentList::try_from(vec!["claude-1".parse().unwrap()]).unwrap(),
                task_list: TaskListText::new("Finish the work.".to_owned()).unwrap(),
                coordinator: None,
                merge: MergeMode::Autopilot,
            }),
            reply,
        })
        .await
        .unwrap();
    let Reply::Started { team, tasks } = receive.await.unwrap().unwrap() else {
        panic!("team did not start")
    };
    {
        let now = "2026-11-16T00:00:00Z".parse().unwrap();
        let mut team_mut = step.owner.parts.state.team_mut(&team).unwrap();
        let (_, mut member) = team_mut.find_mut(&tasks[0]).unwrap();
        member
            .begin_run("run-20261116-000000-0001".parse().unwrap(), now, None)
            .unwrap();
        member.transition(TaskStatus::Running).unwrap();
        member.end_run(now, RunEnd::TurnLimit).unwrap();
    }
    let (status, notice) = step.owner.turn_limit(&tasks[0]).unwrap();
    assert_eq!(status, TaskStatus::Failed);
    assert_eq!(notice, Some(Notice::TurnLimitNoSession));
    let rendered = notice
        .unwrap()
        .render(&step.owner.parts.texts, "claude-1", &tasks[0])
        .unwrap();
    assert_eq!(rendered.title(), "claude-1: turn limit reached without a session");
    assert!(rendered.body().contains("no session ID to resume"));
    assert!(!rendered.body().contains("consecutive turn-limit endings"));
}
