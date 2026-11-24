//! A run that reports `STATUS: FAILED` after it used a tool is not the end of its task: the ladder starts the task in a
//! new session before anything fails it.

use agent_core::state::MergeMode;
use agent_core::status::TaskStatus;

use super::rig::{ONE, Rig, investigator, normal};

/// A reviewer that approves.
const APPROVE: &str = "verify:VERDICT: APPROVE\\nSUMMARY: fine";

// Rule: a member that reports FAILED after a tool call is started once more in a new session; its task is not failed.
#[tokio::test]
async fn a_declared_failure_after_a_tool_call_goes_on_in_a_new_session() {
    let mut rig = Rig::new(
        &[
            normal("claude-1", "fail-first-tool:the second session says done"),
            investigator("inv-r1", APPROVE),
            investigator("inv-r2", APPROVE),
        ],
        5,
        false,
    )
    .await;
    rig.start().await;
    let (_, tasks) = rig.run(&["claude-1"], ONE, MergeMode::Autopilot, None).await;
    rig.wait_for("the second run to start", || rig.runs(&tasks[0]) >= 2)
        .await;
    let state = rig.state();
    rig.stop().await.unwrap();

    let member = state.find_task(&tasks[0]).unwrap().member;
    assert_ne!(
        member.status(),
        TaskStatus::Failed,
        "a declared failure is not the end of the task"
    );
    assert_ne!(
        member.runs()[1].session(),
        member.runs()[0].session(),
        "the second run is a new session"
    );
    assert_eq!(member.renewals().map(agent_core::state::Renewals::used), Some(1));
}
