//! The plan of a task through the daemon: what a run is told of it, and a plan committed by mistake.

use agent_core::ids::ItemId;
use agent_core::state::{PlanReason, State};
use agent_core::status::TaskStatus;

use super::super::fixture::Daemon;
use crate::fixtures::{solo_state, task};

/// The first lines of the rules a run is told of the plan.
const RULES: &str = "## Your plan\n\nKeep the plan of your task in `.agent/plan.md`";

/// [`solo_state`] whose task 1 was sent back for `reason`, which no run has been told yet.
fn sent_back(reason: PlanReason) -> State {
    let mut state = solo_state();
    let (_, mut member) = state.find_task_mut(&task(1)).unwrap();
    member.send_back(reason);
    drop(member);
    state
}

/// A daemon of `state` whose configuration holds runs to their plan `bounces` times.
fn with_bounces(state: &State, bounces: u32) -> Daemon {
    let daemon = Daemon::new(state, None);
    let config = std::fs::read_to_string(daemon.home.config_file()).unwrap();
    std::fs::write(
        daemon.home.config_file(),
        config.replace("[plan]\nbounces = 0\n", &format!("[plan]\nbounces = {bounces}\n")),
    )
    .unwrap();
    daemon
}

#[tokio::test]
async fn a_run_sent_back_is_told_its_reason_and_no_later_run_is() {
    let reason = PlanReason::Unfinished {
        steps: vec!["1.2".parse::<ItemId>().unwrap(), "2.1".parse::<ItemId>().unwrap()],
    };
    let mut daemon = with_bounces(&sent_back(reason), 3);
    daemon.scenario(1, "done");

    daemon.start().await;
    let state = daemon.status_is(1, TaskStatus::Done).await;
    daemon.stop().await.unwrap();

    let prompt = daemon.recorded(1, ".fake-args");
    assert!(
        prompt.contains("are still to do or in hand: 1.2, 2.1. Finish each of them"),
        "{prompt}"
    );
    let keeping = state.find_task(&task(1)).unwrap().member.plan().unwrap().clone();
    assert_eq!((keeping.sent_back(), keeping.pending()), (1, None));
}

#[tokio::test]
async fn a_plan_that_does_not_parse_is_told_with_its_line() {
    let reason = PlanReason::Unreadable {
        line: Some(std::num::NonZeroU32::new(7).unwrap()),
        detail: "line 7: the step is not in a stage — zażółć".to_owned(),
    };
    let mut daemon = with_bounces(&sent_back(reason), 3);
    daemon.scenario(1, "done");

    daemon.start().await;
    daemon.status_is(1, TaskStatus::Done).await;
    daemon.stop().await.unwrap();

    let prompt = daemon.recorded(1, ".fake-args");
    assert!(
        prompt.contains("is not in the form of a plan: line 7: the step is not in a stage — zażółć."),
        "{prompt}"
    );
}

#[tokio::test]
async fn a_first_prompt_has_the_rules_of_the_plan_where_runs_are_held_to_it() {
    let mut daemon = with_bounces(&solo_state(), 3);
    daemon.scenario(1, "done");

    daemon.start().await;
    daemon.status_is(1, TaskStatus::Done).await;
    daemon.stop().await.unwrap();

    let prompt = daemon.recorded(1, ".fake-args");
    assert!(prompt.contains(RULES), "{prompt}");
}

#[tokio::test]
async fn no_rules_are_told_when_no_run_is_sent_back_for_its_plan() {
    let mut daemon = with_bounces(&solo_state(), 0);
    daemon.scenario(1, "done");

    daemon.start().await;
    daemon.status_is(1, TaskStatus::Done).await;
    daemon.stop().await.unwrap();

    let prompt = daemon.recorded(1, ".fake-args");
    assert!(!prompt.contains(RULES), "{prompt}");
}
