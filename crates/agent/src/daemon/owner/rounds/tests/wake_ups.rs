//! A message that reaches a lead while it works on a fix pass neither loses the fix response nor wakes the lead without end.

use agent_core::ids::TaskId;
use agent_core::protocol::{Client, Command, SayRequest, SayText, Target};
use agent_core::state::{ItemPhase, MergeMode};

use super::rig::{ONE, Rig, investigator, normal};
use crate::client::Connection;

/// A first review that asks for one change, then approves the fix.
const CHANGES_ONCE: &str = "verify:VERDICT: REQUEST_CHANGES\\nFINDINGS\\n1. [MAJOR] TDD-2 src/lib.rs:7 — the case is untested → add coverage\\n   CHECK: read src/lib.rs:7,verify:VERDICT: APPROVE\\nSUMMARY: fixed";
/// A review with nothing to ask for.
const APPROVING: &str = "verify:VERDICT: APPROVE\\nSUMMARY: fine";

/// Says `text` to the agent of `task`, as `agent say` does.
async fn say(rig: &Rig, task: &TaskId, text: &str) {
    let request = SayRequest {
        target: Target::Task(task.clone()),
        text: SayText::try_from(text.to_owned()).unwrap(),
    };
    Connection::connect(&rig.home.socket_file())
        .await
        .unwrap()
        .request(Client::Cli, Command::Say(request))
        .await
        .unwrap();
}

/// A team whose coordinator integrates, then does its fix pass in its third run, which waits until the test releases it,
/// and then ignores its messages in every run.
async fn held_fix_pass() -> Rig {
    Rig::new(
        &[
            investigator("inv-lead", "sleep,integrate,deaf-fix-held:1,deaf-answer:1"),
            normal("claude-1", "work"),
            investigator("inv-r1", CHANGES_ONCE),
            investigator("inv-r2", APPROVING),
        ],
        5,
        true,
    )
    .await
}

#[tokio::test]
async fn a_message_during_a_fix_pass_does_not_lose_the_fix_response() {
    let mut rig = held_fix_pass().await;
    rig.start().await;
    let (_, tasks) = rig
        .run(&["inv-lead", "claude-1"], ONE, MergeMode::Autopilot, Some("inv-lead"))
        .await;
    let worktree = rig.worktree(&tasks[0]);
    rig.wait_for("the fix pass to start", || worktree.join(".fake-started-3").exists())
        .await;
    say(&rig, &tasks[0], "please also look at the docs").await;
    std::fs::write(worktree.join(".fake-release-3"), "").unwrap();

    rig.item_where("1.1", "done", |phase| matches!(phase, ItemPhase::Done { .. }))
        .await;
    rig.stop().await.unwrap();
}
