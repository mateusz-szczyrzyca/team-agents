//! A round that goes on past `[limits] round_minutes` without its pull request in review: the daemon cuts the run of the
//! agent that works on it and hands the task over through `[replacement]`; with nobody to take it over, the task fails
//! and the human is told.

use agent_core::decide::ladder::Trouble;
use agent_core::ids::TaskId;
use agent_core::state::{ItemPhase, MergeMode, RunEnd, RunState, State};
use agent_core::status::TaskStatus;

use super::rig::{ONE, Rig, investigator, normal};

/// A reviewer that approves.
const APPROVE: &str = "verify:VERDICT: APPROVE\\nSUMMARY: fine";

/// Adds `table` to the configuration of `rig`, before the daemon starts.
fn configure(rig: &Rig, table: &str) {
    let mut config = std::fs::read_to_string(rig.home.config_file()).unwrap();
    config.push_str(table);
    std::fs::write(rig.home.config_file(), config).unwrap();
}

/// How each run of `task` in `state` ended, oldest first; `None` for a run that is going.
fn ends(state: &State, task: &TaskId) -> Vec<Option<RunEnd>> {
    state
        .find_task(task)
        .unwrap()
        .member
        .runs()
        .iter()
        .map(|run| match run.state() {
            RunState::Ended { end, .. } => Some(end.clone()),
            RunState::Running => None,
        })
        .collect()
}

/// A rig of `claude-1`, which holds its run until it is stopped, `claude-2`, which does the work, and two reviewers that
/// approve, under `table`.
async fn rig_under(table: &str) -> Rig {
    let rig = Rig::new(
        &[
            normal("claude-1", "hold"),
            normal("claude-2", "work"),
            investigator("inv-r1", APPROVE),
            investigator("inv-r2", APPROVE),
        ],
        5,
        false,
    )
    .await;
    configure(&rig, table);
    rig
}

/// Starts `rig`, runs the one item with `claude-1` and waits until its run works on the item.
async fn holding(rig: &mut Rig) -> TaskId {
    rig.start().await;
    let (_, tasks) = rig.run(&["claude-1"], ONE, MergeMode::Autopilot, None).await;
    rig.item_where("1.1", "claude-1 to work on the item", |phase| {
        matches!(phase, ItemPhase::InProgress { .. })
    })
    .await;
    rig.status_is(&tasks[0], TaskStatus::Running).await;
    tasks[0].clone()
}

// Rule: the run of an agent whose round went on past `[limits] round_minutes` without its pull request in review is cut,
// and the task is handed over to the first fitting agent of `[replacement] agents`, which does the work; the cut run ended
// as a stop, and the state records the hand-over for the overdue round.
#[tokio::test]
async fn an_overdue_round_is_cut_and_its_task_handed_over_to_a_fitting_agent() {
    let mut rig = rig_under("[replacement]\nagents = [\"claude-2\"]\n[limits]\nround_minutes = 120\n").await;
    let task = holding(&mut rig).await;

    rig.advance_history_days(1);
    let state = rig
        .item_where("1.1", "merged after the hand-over", |phase| {
            matches!(phase, ItemPhase::Done { .. })
        })
        .await;
    rig.stop().await.unwrap();

    let member = state.find_task(&task).unwrap().member;
    assert_eq!(member.agent().name(), "claude-2", "the fitting agent holds the task");
    assert_eq!(member.past_holders().len(), 1);
    assert_eq!(member.past_holders()[0].agent.name(), "claude-1");
    assert_eq!(
        member.renewals().map(agent_core::state::Renewals::reason),
        Some(Trouble::RoundOverdue)
    );
    let ended = ends(&state, &task);
    assert_eq!(ended.first(), Some(&Some(RunEnd::Stopped)), "{ended:?}");
    assert_eq!(member.bad_runs_since_renewal(), 0, "the cut is no bad run");
}

// Rule: with nobody to take the task over, an overdue round fails its task and the human is told why.
#[tokio::test]
async fn an_overdue_round_with_nobody_to_take_it_over_fails_its_task_and_tells_the_human() {
    let mut rig =
        rig_under("[replacement]\nagents = [\"claude-2\"]\nmode = \"off\"\n[limits]\nround_minutes = 120\n").await;
    let task = holding(&mut rig).await;

    rig.advance_history_days(1);
    let state = rig.status_is(&task, TaskStatus::Failed).await;
    let notice = rig.notification_with("round_minutes").await;
    rig.stop().await.unwrap();

    assert_eq!(state.find_task(&task).unwrap().member.agent().name(), "claude-1");
    assert_eq!(ends(&state, &task), vec![Some(RunEnd::Stopped)]);
    assert!(notice.contains("item 1.1"), "{notice}");
    assert!(notice.contains("without its pull request in review"), "{notice}");
}

/// A solo task that asks once and holds its next run, with its first question answered a day later.
async fn answered_late() -> (Rig, TaskId) {
    use agent_core::protocol::{AnswerInput, AnswerList, AnswerRequest, Client, Command, Reply};
    use agent_core::questions::Verdict;
    let mut rig = Rig::new(
        &[
            normal("claude-1", "ask-here,hold"),
            normal("claude-2", "work"),
            investigator("inv-r1", APPROVE),
            investigator("inv-r2", APPROVE),
        ],
        5,
        false,
    )
    .await;
    rig.pin_turn_limit_clock();
    configure(
        &rig,
        "[replacement]\nagents = [\"claude-2\"]\n[limits]\nround_minutes = 120\n",
    );
    rig.start_with_log_layer(agent_runtime::history::HistoryLayer::deferred(
        agent_core::log::LogLevel::Info,
        rig.history_test_clock(),
        agent_core::config::Secrets::default(),
    ))
    .await;
    let (_, tasks) = rig.run(&["claude-1"], ONE, MergeMode::Autopilot, None).await;
    let task = tasks[0].clone();
    rig.status_is(&task, TaskStatus::Questions).await;
    rig.advance_history_days(1);
    let reply = crate::client::Connection::connect(&rig.home.socket_file())
        .await
        .unwrap()
        .request(
            Client::Cli,
            Command::Answer(AnswerRequest {
                task: task.clone(),
                number: std::num::NonZeroU32::MIN,
                answers: AnswerList::try_from(vec![AnswerInput {
                    question: std::num::NonZeroU32::MIN,
                    verdict: Verdict::from_text("ok").unwrap().unwrap(),
                }])
                .unwrap(),
            }),
        )
        .await
        .unwrap();
    assert!(matches!(reply, Reply::Applied { .. }), "{reply:?}");
    rig.state_where("the answered run starts or is handed over", |state| {
        let member = state.find_task(&task).unwrap().member;
        member.runs().len() >= 2 && (member.current_run().is_some() || member.agent().name() != "claude-1")
    })
    .await;
    (rig, task)
}

/// The persisted hand-over log rows whose reason is the round limit.
fn overdue_handovers(rig: &Rig) -> Vec<agent_core::log::LogEntry> {
    agent_runtime::history::Reader::open(&rig.home.history_file())
        .unwrap()
        .daemon_logs()
        .unwrap()
        .into_iter()
        .filter(|row| {
            row.event() == agent_core::log::LogEvent::TaskHandedOver
                && row.fields().get("reason") == Some(&serde_json::json!("round_overdue"))
        })
        .collect()
}

#[tokio::test]
async fn a_round_answered_after_the_limit_is_not_cut_in_the_first_run_after_the_answer() {
    let (mut rig, task) = answered_late().await;
    std::fs::write(rig.worktree(&task).join(".fake-continue"), "go").unwrap();
    let state = rig
        .state_where("the answered run finishes or is handed over", |state| {
            let member = state.find_task(&task).unwrap().member;
            member.status() == TaskStatus::Waiting || member.agent().name() != "claude-1"
        })
        .await;
    rig.stop().await.unwrap();

    let member = state.find_task(&task).unwrap().member;
    assert_eq!(member.agent().name(), "claude-1");
    assert_eq!(member.past_holders().len(), 0);
    assert_eq!(overdue_handovers(&rig), vec![]);
}

#[tokio::test]
async fn a_round_still_overdue_without_its_waits_is_cut() {
    let (mut rig, task) = answered_late().await;
    rig.advance_history_days(1);
    let state = rig
        .item_where("1.1", "the overdue round is handed over and completed", |phase| {
            matches!(phase, ItemPhase::Done { .. })
        })
        .await;
    rig.stop().await.unwrap();

    let member = state.find_task(&task).unwrap().member;
    assert_eq!(member.agent().name(), "claude-2");
    assert_eq!(member.past_holders().len(), 1);
    assert_eq!(
        member.renewals().map(agent_core::state::Renewals::reason),
        Some(Trouble::RoundOverdue)
    );
    assert_eq!(overdue_handovers(&rig).len(), 1);
}
