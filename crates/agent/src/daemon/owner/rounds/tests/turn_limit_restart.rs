//! Stop the real owner precisely between settling and scheduling, then boot the daemon from its saved state.

use std::path::Path;
use std::str::FromStr;
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
        agent_binary_dir: agent_core::config::AbsolutePath::from_str("/opt/bin").unwrap(),
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
        local_offset: time::UtcOffset::UTC,
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
    let now = "2026-11-16T00:00:00Z".parse().unwrap();
    {
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
    let context = step.owner.notice_context(&tasks[0], now, None).unwrap();
    let rendered = notice.unwrap().render(&step.owner.parts.texts, &context).unwrap();
    assert_eq!(rendered.title(), format!("❌ {}", context.repo));
    assert!(
        rendered
            .body()
            .contains("Note: claude-1: turn limit reached without a session")
    );
    assert!(rendered.body().contains("no session ID to resume"));
    assert!(!rendered.body().contains("consecutive turn-limit endings"));
}

#[tokio::test]
async fn one_explanation_event_links_every_item_round_of_the_pull_request() {
    use crate::daemon::owner::rounds::Pulls;
    use crate::daemon::owner::teams::Look;
    use agent_core::history::{HistoryEvent, Record};
    use agent_core::state::State;
    use agent_runtime::autopilot_prompts::AutopilotPrompts;
    use agent_runtime::checks_prompts::ChecksPrompts;

    let mut rig = Rig::new(&[
        normal("claude-1", "work,sleep"),
        investigator("inv-r1", "done:VERDICT: REQUEST_CHANGES\\nFINDINGS\\n1. [MAJOR] TYP-3 work-1.txt:1 — units → document units\\n   CHECK: read work-1.txt"),
        investigator("inv-r2", "done:VERDICT: APPROVE"),
    ], 5, false).await;
    rig.start().await;
    let (team, tasks) = rig
        .run(&["claude-1"], super::rig::PARALLEL, MergeMode::Autopilot, None)
        .await;
    rig.prompt(&tasks[0], 2).await;
    rig.status_is(&tasks[0], TaskStatus::Waiting).await;
    rig.stop().await.unwrap();
    // Reproduce two items sharing a PR, as the coordinator's integration can produce.
    let mut saved: serde_json::Value = serde_json::from_str(&rig.state().to_json().unwrap()).unwrap();
    let found = &mut saved["teams"][0];
    let mut joined = found["items"][0].clone();
    joined["id"] = "2.1".into();
    found["items"][1] = joined;
    let second = found["rounds"]
        .as_array()
        .unwrap()
        .iter()
        .find(|round| round["item"] == "2.1")
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap();
    let member = &mut found["crew"]["member"];
    member["status"] = "done".into();
    member["runs"].as_array_mut().unwrap().last_mut().unwrap()["state"]["end"] =
        serde_json::json!({"kind": "reported", "outcome": "DONE"});
    let run = member["runs"].as_array_mut().unwrap().last_mut().unwrap();
    run["started_at"] = run["state"]["at"].clone();
    for item in found["items"].as_array_mut().unwrap() {
        item["findings"]["verification"]["lead_message"] =
            "FINDING 1: EXPLAINED — the units are explicit\nSTATUS: DONE".into();
    }
    let state = State::from_json(&saved.to_string()).unwrap();
    let mut step = stepped(&rig).await;
    step.owner.parts.state = state;
    step.owner.record_round(&team, &second);
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../prompts");
    let language = step.owner.parts.settings.config.language();
    let pulls = Pulls::new(
        Arc::new(AutopilotPrompts::load(&root, language).await.unwrap()),
        Arc::new(ChecksPrompts::load(&root, language).await.unwrap()),
        &step.owner.parts.settings,
    )
    .unwrap()
    .unwrap();
    step.owner
        .check_fix_responses(&pulls, &team, &mut Look::default())
        .unwrap();
    let rounds = step
        .owner
        .rounds_of_items(&team, &["1.1".parse().unwrap(), "2.1".parse().unwrap()]);
    drop(step.owner);
    crate::daemon::history::close(step.writer).await;
    let reader = Reader::open(&rig.home.history_file()).unwrap();
    for round in &rounds {
        let explained: Vec<_> = reader
            .round_records(round)
            .unwrap()
            .into_iter()
            .filter_map(|row| match row.record {
                Record::Event(event) if matches!(event.event, HistoryEvent::FindingExplained { .. }) => Some(event),
                _ => None,
            })
            .collect();
        assert_eq!(explained.len(), 1, "round {round}");
        assert_eq!(explained[0].rounds, rounds);
    }
}

#[tokio::test]
async fn verification_refreshes_accepted_explanations_from_the_actual_git_changes() {
    use super::rig::{ONE, git};
    use crate::daemon::owner::teams::Look;
    use agent_core::review_findings::Severity;
    use agent_core::state::{ItemPhase, State};
    use agent_core::status::ReviewVerdict;

    for (file, may_raise) in [("work-1.txt", true), ("unrelated.txt", false)] {
        let mut rig = Rig::new(&[
            normal("claude-1", "work,done:FINDING 1: EXPLAINED — units are explicit"),
            investigator("inv-r1", "done:VERDICT: REQUEST_CHANGES\\nFINDINGS\\n1. [MAJOR] TYP-3 work-1.txt:1 — units → document units\\n   CHECK: read work-1.txt,done:VERDICT: APPROVE\\nFINDING 1: ACCEPTED — explicit units"),
            investigator("inv-r2", "done:VERDICT: APPROVE,done:VERDICT: APPROVE\\nFINDING 1: ACCEPTED — explicit units"),
        ], 5, false).await;
        rig.start().await;
        let (team, tasks) = rig.run(&["claude-1"], ONE, MergeMode::NoMerge, None).await;
        rig.item_where("1.1", "accepted explanation", |phase| {
            matches!(phase, ItemPhase::Approved { .. })
        })
        .await;
        rig.stop().await.unwrap();
        let state = rig.state();
        let item = state.team(&team).unwrap().item("1.1".parse().unwrap()).unwrap();
        let ItemPhase::Approved { pr, .. } = item.phase() else {
            panic!("approved PR")
        };
        let pr = serde_json::to_value(pr).unwrap();
        let ledger = item.findings().unwrap();
        assert_eq!(ledger.entries()[0].finding.severity, Severity::Minor);
        assert_eq!(ledger.disputes().unwrap().accepted_heads.len(), 1);
        let worktree = rig.worktree(&tasks[0]);
        std::fs::write(worktree.join(file), "changed unit contract\n").unwrap();
        git(&worktree, &["add", file]);
        git(&worktree, &["commit", "-m", "Update the next verification input"]);
        let head = git(&worktree, &["rev-parse", "HEAD"]);
        // Resume at the scheduling boundary after a later fix: the new head still needs verification.
        let mut saved: serde_json::Value = serde_json::from_str(&state.to_json().unwrap()).unwrap();
        saved["teams"][0]["items"][0]["phase"] = serde_json::json!({"phase":"in_review", "pr":pr, "reviews":[]});
        saved["teams"][0]["rounds"][0]["state"] = serde_json::json!({"status":"open"});
        let mut step = stepped(&rig).await;
        step.owner.parts.state = State::from_json(&saved.to_string()).unwrap();
        let mut look = Look::default();
        step.owner.ask_for_reviews(&team, &mut look).await.unwrap();
        assert!(look.changed, "verification must be scheduled");
        let item = step
            .owner
            .parts
            .state
            .team(&team)
            .unwrap()
            .item("1.1".parse().unwrap())
            .unwrap();
        let ledger = item.findings().unwrap();
        let ItemPhase::InReview { reviews, .. } = item.phase() else {
            panic!("verification")
        };
        assert_eq!(reviews.len(), 2);
        assert_eq!(ledger.entries()[0].finding.severity, Severity::Minor);
        let raised = ledger.parse_report(
            "FINDING 1: RAISED [MAJOR] — changed unit contract",
            ReviewVerdict::RequestChanges,
        );
        match (may_raise, raised) {
            (true, Ok(report)) => assert_eq!(report.answers.len(), 1),
            (false, Err(error)) => assert_eq!(error.line, 1),
            (expected, outcome) => panic!("{file}: may_raise={expected}, got {outcome:?} at {head}"),
        }
        drop(step.owner);
        crate::daemon::history::close(step.writer).await;
    }
}

#[tokio::test]
async fn fix_responses_do_not_read_head_until_a_new_done_response_is_selected() {
    use crate::daemon::owner::rounds::Pulls;
    use crate::daemon::owner::teams::Look;
    use agent_core::state::State;
    use agent_runtime::autopilot_prompts::AutopilotPrompts;
    use agent_runtime::checks_prompts::ChecksPrompts;
    let mut rig = Rig::new(&[
        normal("claude-1", "work,sleep"),
        investigator("inv-r1", "done:VERDICT: REQUEST_CHANGES\\nFINDINGS\\n1. [MAJOR] TYP-3 work-1.txt:1 — units → document units\\n   CHECK: read work-1.txt"),
        investigator("inv-r2", "done:VERDICT: APPROVE"),
    ], 5, false).await;
    rig.start().await;
    let (team, tasks) = rig
        .run(&["claude-1"], super::rig::ONE, MergeMode::Autopilot, None)
        .await;
    rig.prompt(&tasks[0], 2).await;
    rig.status_is(&tasks[0], TaskStatus::Waiting).await;
    rig.stop().await.unwrap();
    let worktree = rig.worktree(&tasks[0]);
    std::fs::rename(worktree.join(".git"), worktree.join(".git-held")).unwrap();
    let original: serde_json::Value = serde_json::from_str(&rig.state().to_json().unwrap()).unwrap();
    for mode in ["unfinished", "answered", "checked"] {
        let mut saved = original.clone();
        let found = &mut saved["teams"][0];
        if mode != "unfinished" {
            let run = found["crew"]["member"]["runs"]
                .as_array_mut()
                .unwrap()
                .last_mut()
                .unwrap();
            run["state"]["end"] = serde_json::json!({"kind":"reported", "outcome":"DONE"});
            let run_id = run["id"].clone();
            let disputes = &mut found["items"][0]["findings"]["disputes"];
            if mode == "answered" {
                disputes["answered_pass"] = 1.into();
            } else {
                disputes["checked_run"] = run_id;
            }
        }
        let mut step = stepped(&rig).await;
        step.owner.parts.state = State::from_json(&saved.to_string()).unwrap();
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../prompts");
        let language = step.owner.parts.settings.config.language();
        let pulls = Pulls::new(
            Arc::new(AutopilotPrompts::load(&root, language).await.unwrap()),
            Arc::new(ChecksPrompts::load(&root, language).await.unwrap()),
            &step.owner.parts.settings,
        )
        .unwrap()
        .unwrap();
        let before = step.owner.parts.state.clone();
        let mut look = Look::default();
        let result = step.owner.read_fix_responses(&pulls, &team, &mut look).await;
        assert!(matches!(result, Ok(())), "{mode}: {result:?}");
        assert_eq!(step.owner.parts.state, before);
        assert!(!look.changed);
        drop(step.owner);
        crate::daemon::history::close(step.writer).await;
    }
}

#[tokio::test]
async fn opinion_inactivity_uses_review_patience_and_resets_on_output() {
    use agent_core::state::{OpinionFailure, OpinionStage, Timestamp};
    let mut rig = Rig::new(
        &[
            normal("claude-1", "work,opinion-ask"),
            investigator("inv-r1", "opinion-failure:HOLD"),
            investigator("inv-r2", "opinion-failure:HOLD"),
            investigator("inv-r3", "opinion-failure:HOLD"),
            investigator("inv-r4", "opinion-failure:HOLD"),
        ],
        1,
        false,
    )
    .await;
    let config = std::fs::read_to_string(rig.home.config_file()).unwrap();
    std::fs::write(
        rig.home.config_file(),
        config.replace("[review]", "[review]\nreplace_after_minutes = 1"),
    )
    .unwrap();
    rig.start().await;
    let (team, _) = rig
        .run(
            &["claude-1"],
            super::rig::ONE,
            agent_core::state::MergeMode::Autopilot,
            None,
        )
        .await;
    let running = rig
        .state_where("opinions running", |state| {
            let team = state.team(&team).unwrap();
            team.items()[0].findings().is_some_and(|ledger| {
                ledger.opinions().len() == 2
                    && ledger.opinions().values().all(|opinion| {
                        let OpinionStage::Assigned { task, .. } = &opinion.stage else {
                            return false;
                        };
                        team.find(task).unwrap().1.current_run().is_some()
                    })
            })
        })
        .await;
    rig.stop().await.unwrap();
    let mut step = stepped(&rig).await;
    step.owner.parts.state = running;
    let pulls = opinion_pulls(&step.owner).await;
    let now = Timestamp::now(&step.owner.parts.clock).unwrap();
    step.owner.replace_silent_opinions(&pulls, &team, now).await.unwrap();
    let tasks: Vec<_> = step.owner.parts.state.team(&team).unwrap().items()[0]
        .findings()
        .unwrap()
        .opinions()
        .values()
        .map(|opinion| {
            let OpinionStage::Assigned { task, .. } = &opinion.stage else {
                panic!("assigned")
            };
            task.clone()
        })
        .collect();
    for task in &tasks {
        let run = step
            .owner
            .parts
            .state
            .find_task(task)
            .unwrap()
            .member
            .current_run()
            .unwrap();
        let file = rig.home.run_dir(task, run.id()).join("out.jsonl");
        let mut text = std::fs::read_to_string(&file).unwrap();
        text.push_str("activity\n");
        std::fs::write(file, text).unwrap();
    }
    let later = Timestamp::from_utc(now.as_utc() + time::Duration::minutes(2)).unwrap();
    step.owner.replace_silent_opinions(&pulls, &team, later).await.unwrap();
    assert!(
        step.owner.parts.state.team(&team).unwrap().items()[0]
            .findings()
            .unwrap()
            .opinions()
            .values()
            .all(|o| matches!(o.stage, OpinionStage::Assigned { .. }))
    );
    let quiet = Timestamp::from_utc(later.as_utc() + time::Duration::minutes(1)).unwrap();
    step.owner.replace_silent_opinions(&pulls, &team, quiet).await.unwrap();
    let ledger = step.owner.parts.state.team(&team).unwrap().items()[0]
        .findings()
        .unwrap();
    for number in ledger.opinions().keys() {
        assert_eq!(ledger.opinions()[number].stage, OpinionStage::Waiting);
        assert_eq!(ledger.opinion_attempts(*number).len(), 1);
        assert_eq!(
            ledger.opinion_attempts(*number)[0].failure,
            Some(OpinionFailure::Silent)
        );
    }
    drop(step.owner);
    crate::daemon::history::close(step.writer).await;
}

/// Loads the real opinion catalog for deterministic owner stepping.
async fn opinion_pulls(owner: &Owner) -> crate::daemon::owner::rounds::Pulls {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../prompts");
    let language = owner.parts.settings.config.language();
    crate::daemon::owner::rounds::Pulls::new(
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
