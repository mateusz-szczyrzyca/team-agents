//! Tests for `agent swap`: changing the agent holding a task on the human's word.

use agent_core::log::{LogEntry, LogEvent, LogLevel};
use agent_core::protocol::{Client, Command, FailureKind, Reply, SwapRequest};
use agent_core::state::{ItemPhase, MergeMode};
use agent_core::status::TaskStatus;

use super::rig::{ONE, Rig, investigator, normal};
use crate::client::{ClientError, Connection};

/// A reviewer that approves.
const APPROVE: &str = "verify:VERDICT: APPROVE\\nSUMMARY: fine";

/// Adds `table` to the configuration of `rig`, before the daemon starts.
fn configure(rig: &Rig, table: &str) {
    let mut config = std::fs::read_to_string(rig.home.config_file()).unwrap();
    config.push_str(table);
    std::fs::write(rig.home.config_file(), config).unwrap();
}

/// Sends `command` as the CLI does and returns the reply.
async fn ask(rig: &Rig, command: Command) -> Result<Reply, ClientError> {
    Connection::connect(&rig.home.socket_file())
        .await
        .unwrap()
        .request(Client::Cli, command)
        .await
}

/// The failure of a refused request.
fn refusal(result: Result<Reply, ClientError>) -> (FailureKind, String) {
    match result {
        Err(ClientError::Refused(failure)) => (failure.kind, failure.message),
        other => panic!("expected refused request, got {other:?}"),
    }
}

// Rule: a running member task of claude-1 swapped to glm-1 of tier low: the run is ended, the task is queued, its next
// run is glm-1's in a new session whose prompt is the briefing with reason swap, in the same worktree and branch;
// past_holders = [claude-1].
#[tokio::test]
async fn running_member_swapped_to_lower_tier() {
    let mut rig = Rig::new(
        &[
            investigator("claude-1", "work,busy"),
            normal("glm-1", "work"),
            investigator("inv-r1", APPROVE),
            investigator("inv-r2", APPROVE),
        ],
        5,
        false,
    )
    .await;
    rig.start().await;
    let (_, tasks) = rig.run(&["claude-1"], ONE, MergeMode::Autopilot, None).await;
    let member_task = tasks[0].clone();

    rig.status_is(&member_task, TaskStatus::Running).await;

    let reply = ask(
        &rig,
        Command::Swap(SwapRequest {
            task: member_task.clone(),
            agent: "glm-1".parse().unwrap(),
        }),
    )
    .await
    .unwrap();

    assert_eq!(
        reply,
        Reply::Applied {
            tasks: vec![member_task.clone()]
        }
    );

    let state = rig
        .item_where("1.1", "merged after swap to lower tier", |phase| {
            matches!(phase, ItemPhase::Done { .. })
        })
        .await;

    let member = state.find_task(&member_task).unwrap().member;
    assert_eq!(member.agent().name(), "glm-1");
    assert_eq!(member.past_holders().len(), 1);
    assert_eq!(member.past_holders()[0].agent.name(), "claude-1");

    let takeover = rig.prompt(&member_task, 2).await;
    assert!(
        takeover.starts_with("# A NEW SESSION ON A TASK THAT WAS ALREADY STARTED"),
        "{takeover}"
    );
    assert!(takeover.contains("`swap`:"), "{takeover}");
    assert!(takeover.contains("You are agent **glm-1**"), "{takeover}");

    let log = std::fs::read_to_string(rig.worktree(&member_task).join(".fake-log")).unwrap();
    let branches: Vec<&str> = log.lines().filter_map(|line| line.split_whitespace().nth(2)).collect();
    assert!(
        branches.iter().all(|branch| *branch == branches[0]) && branches[0].starts_with("agents/claude-1/"),
        "every run on the task's original branch: {branches:?}"
    );

    rig.stop().await.unwrap();
}

// Rule: a task waiting for answers keeps its status: status stays questions, holder changed, no run started.
#[tokio::test]
async fn task_waiting_for_answers_keeps_its_status() {
    let mut rig = Rig::new(
        &[
            normal("claude-1", "question:1"),
            normal("claude-2", "work"),
            investigator("inv-r1", APPROVE),
            investigator("inv-r2", APPROVE),
        ],
        5,
        false,
    )
    .await;
    rig.start().await;
    let (_, tasks) = rig.run(&["claude-1"], ONE, MergeMode::Autopilot, None).await;
    let member_task = tasks[0].clone();

    rig.status_is(&member_task, TaskStatus::Questions).await;

    let reply = ask(
        &rig,
        Command::Swap(SwapRequest {
            task: member_task.clone(),
            agent: "claude-2".parse().unwrap(),
        }),
    )
    .await
    .unwrap();

    assert_eq!(
        reply,
        Reply::Applied {
            tasks: vec![member_task.clone()]
        }
    );

    let state = rig.state();
    let member = state.find_task(&member_task).unwrap().member;
    assert_eq!(member.status(), TaskStatus::Questions);
    assert_eq!(member.agent().name(), "claude-2");
    assert_eq!(member.runs().len(), 1);

    rig.stop().await.unwrap();
}

// Rule: an unknown agent is refused with its exact reply line.
#[tokio::test]
async fn unknown_agent_is_refused() {
    let mut rig = Rig::new(
        &[
            normal("claude-1", "busy"),
            investigator("inv-r1", APPROVE),
            investigator("inv-r2", APPROVE),
        ],
        5,
        false,
    )
    .await;
    rig.start().await;
    let (_, tasks) = rig.run(&["claude-1"], ONE, MergeMode::Autopilot, None).await;

    let (kind, message) = refusal(
        ask(
            &rig,
            Command::Swap(SwapRequest {
                task: tasks[0].clone(),
                agent: "glm-9".parse().unwrap(),
            }),
        )
        .await,
    );

    assert_eq!(kind, FailureKind::Invalid);
    assert_eq!(message, "unknown agent glm-9: it is not in agents.toml");

    rig.stop().await.unwrap();
}

// Rule: the task's own agent is refused with its exact reply line.
#[tokio::test]
async fn same_holder_is_refused() {
    let mut rig = Rig::new(
        &[
            normal("claude-1", "busy"),
            investigator("inv-r1", APPROVE),
            investigator("inv-r2", APPROVE),
        ],
        5,
        false,
    )
    .await;
    rig.start().await;
    let (_, tasks) = rig.run(&["claude-1"], ONE, MergeMode::Autopilot, None).await;

    let (kind, message) = refusal(
        ask(
            &rig,
            Command::Swap(SwapRequest {
                task: tasks[0].clone(),
                agent: "claude-1".parse().unwrap(),
            }),
        )
        .await,
    );

    assert_eq!(kind, FailureKind::Conflict);
    assert_eq!(message, format!("{} is already held by claude-1", tasks[0]));

    rig.stop().await.unwrap();
}

// Rule: an agent of the same team is refused with its exact reply line.
#[tokio::test]
async fn in_team_is_refused() {
    let mut rig = Rig::new(
        &[
            normal("claude-1", "busy"),
            normal("claude-2", "busy"),
            investigator("inv-r1", APPROVE),
            investigator("inv-r2", APPROVE),
        ],
        5,
        false,
    )
    .await;
    rig.start().await;
    let (team, tasks) = rig
        .run(&["claude-1", "claude-2"], ONE, MergeMode::Autopilot, None)
        .await;

    let (kind, message) = refusal(
        ask(
            &rig,
            Command::Swap(SwapRequest {
                task: tasks[0].clone(),
                agent: "claude-2".parse().unwrap(),
            }),
        )
        .await,
    );

    assert_eq!(kind, FailureKind::Conflict);
    assert_eq!(message, format!("claude-2 is already in {team} as {}", tasks[1]));

    rig.stop().await.unwrap();
}

// Rule: an agent with a run going in another task is refused with its exact reply line.
#[tokio::test]
async fn in_run_is_refused() {
    let mut rig = Rig::new(
        &[
            normal("claude-1", "busy"),
            normal("claude-2", "busy"),
            investigator("inv-r1", APPROVE),
            investigator("inv-r2", APPROVE),
        ],
        5,
        false,
    )
    .await;
    rig.start().await;
    let (_, team_one_tasks) = rig.run(&["claude-1"], ONE, MergeMode::Autopilot, None).await;
    let (_, team_two_tasks) = rig.run(&["claude-2"], ONE, MergeMode::Autopilot, None).await;

    rig.status_is(&team_two_tasks[0], TaskStatus::Running).await;

    let (kind, message) = refusal(
        ask(
            &rig,
            Command::Swap(SwapRequest {
                task: team_one_tasks[0].clone(),
                agent: "claude-2".parse().unwrap(),
            }),
        )
        .await,
    );

    assert_eq!(kind, FailureKind::Conflict);
    assert_eq!(
        message,
        format!("claude-2 runs {} now; swap when it is free", team_two_tasks[0])
    );

    rig.stop().await.unwrap();
}

// Rule: a worker of [tiers] workers is refused with its exact reply line.
#[tokio::test]
async fn worker_is_refused() {
    let mut rig = Rig::new(
        &[
            normal("claude-1", "busy"),
            normal("glm-1", "work"),
            investigator("inv-r1", APPROVE),
            investigator("inv-r2", APPROVE),
        ],
        5,
        false,
    )
    .await;
    let text = std::fs::read_to_string(rig.home.config_file()).unwrap();
    let with_worker = text.replace("[tiers]\nnormal = [", "[tiers]\nworkers = [\"glm-1\"]\nnormal = [");
    std::fs::write(rig.home.config_file(), with_worker).unwrap();

    rig.start().await;
    let (_, tasks) = rig.run(&["claude-1"], ONE, MergeMode::Autopilot, None).await;

    let (kind, message) = refusal(
        ask(
            &rig,
            Command::Swap(SwapRequest {
                task: tasks[0].clone(),
                agent: "glm-1".parse().unwrap(),
            }),
        )
        .await,
    );

    assert_eq!(kind, FailureKind::Invalid);
    assert_eq!(
        message,
        "glm-1 is a worker of [tiers] workers: give it no task of a team"
    );

    rig.stop().await.unwrap();
}

// Rule: a task of a worker or a reviewer is refused with its exact reply line.
#[tokio::test]
async fn role_is_refused() {
    let mut rig = Rig::new(
        &[
            normal("claude-1", "work"),
            normal("claude-2", "work"),
            investigator("inv-r1", APPROVE),
            investigator("inv-r2", APPROVE),
        ],
        5,
        false,
    )
    .await;
    rig.start().await;
    let (team_id, _) = rig.run(&["claude-1"], ONE, MergeMode::Autopilot, None).await;

    rig.item_where("1.1", "pull request in review", |phase| {
        matches!(phase, ItemPhase::InReview { .. })
    })
    .await;

    let state = rig.state();
    let team = state.team(&team_id).unwrap();
    let reviewer_task = team.reviewers().first().unwrap().task().clone();

    let (kind, message) = refusal(
        ask(
            &rig,
            Command::Swap(SwapRequest {
                task: reviewer_task.clone(),
                agent: "claude-2".parse().unwrap(),
            }),
        )
        .await,
    );

    assert_eq!(kind, FailureKind::Conflict);
    assert_eq!(
        message,
        format!("{reviewer_task} is a reviewer's task: swap takes a solo, coordinator or member task")
    );

    rig.stop().await.unwrap();
}

// Rule: a task in a closed team, or done, is refused with its exact reply line.
#[tokio::test]
async fn done_task_is_refused() {
    let mut rig = Rig::new(
        &[
            normal("claude-1", "work"),
            normal("claude-2", "work"),
            investigator("inv-r1", APPROVE),
            investigator("inv-r2", APPROVE),
        ],
        5,
        false,
    )
    .await;
    rig.start().await;
    let (_, tasks) = rig.run(&["claude-1"], ONE, MergeMode::Autopilot, None).await;

    rig.item_where("1.1", "team finished and item done", |phase| {
        matches!(phase, ItemPhase::Done { .. })
    })
    .await;

    let (kind, message) = refusal(
        ask(
            &rig,
            Command::Swap(SwapRequest {
                task: tasks[0].clone(),
                agent: "claude-2".parse().unwrap(),
            }),
        )
        .await,
    );

    assert_eq!(kind, FailureKind::Conflict);
    assert_eq!(message, format!("{} is done: nothing to swap", tasks[0]));

    rig.stop().await.unwrap();
}

// Rule: [replacement] replacements = 0: the swap is still applied.
#[tokio::test]
async fn swap_ignores_replacements_zero() {
    let mut rig = Rig::new(
        &[
            normal("claude-1", "busy"),
            normal("claude-2", "work"),
            investigator("inv-r1", APPROVE),
            investigator("inv-r2", APPROVE),
        ],
        5,
        false,
    )
    .await;
    configure(&rig, "[replacement]\nreplacements = 0\n");
    rig.start().await;
    let (_, tasks) = rig.run(&["claude-1"], ONE, MergeMode::Autopilot, None).await;
    let member_task = tasks[0].clone();

    let reply = ask(
        &rig,
        Command::Swap(SwapRequest {
            task: member_task.clone(),
            agent: "claude-2".parse().unwrap(),
        }),
    )
    .await
    .unwrap();

    assert_eq!(
        reply,
        Reply::Applied {
            tasks: vec![member_task.clone()]
        }
    );

    rig.stop().await.unwrap();
}

// Rule: swap is recorded in history as decision_taken swap and in log as task_handed_over reason=swap.
#[tokio::test]
async fn swap_is_recorded() {
    let mut rig = Rig::new(
        &[
            normal("claude-1", "busy"),
            normal("claude-2", "work"),
            investigator("inv-r1", APPROVE),
            investigator("inv-r2", APPROVE),
        ],
        5,
        false,
    )
    .await;
    rig.start_with_log_layer(agent_runtime::history::HistoryLayer::deferred(
        LogLevel::Debug,
        rig.history_test_clock(),
        agent_core::config::Secrets::default(),
    ))
    .await;
    let (_, tasks) = rig.run(&["claude-1"], ONE, MergeMode::Autopilot, None).await;
    let member_task = tasks[0].clone();

    let reply = ask(
        &rig,
        Command::Swap(SwapRequest {
            task: member_task.clone(),
            agent: "claude-2".parse().unwrap(),
        }),
    )
    .await
    .unwrap();

    assert_eq!(
        reply,
        Reply::Applied {
            tasks: vec![member_task.clone()]
        }
    );

    rig.wait_for("the swap records in history", || {
        let logs: Vec<LogEntry> = agent_runtime::history::Reader::open(&rig.home.history_file())
            .unwrap()
            .daemon_logs()
            .unwrap();
        let has_handover = logs.iter().any(|entry| {
            entry.event() == LogEvent::TaskHandedOver && entry.fields().get("reason").is_some_and(|r| r == "swap")
        });
        let has_decision = logs.iter().any(|entry| {
            entry.event() == LogEvent::DecisionTaken && entry.fields().get("decision").is_some_and(|d| d == "swap")
        });
        has_handover && has_decision
    })
    .await;

    rig.stop().await.unwrap();
}
