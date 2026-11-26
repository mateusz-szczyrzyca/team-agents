//! What the human reads and what the lead reads when a waiting task is renewed: the recovery sentence of the notice
//! and the reason `human` in the briefing header.

use agent_core::ids::TaskId;
use agent_core::protocol::{Client, Command, ControlRequest, Reply, SayRequest, Target};
use agent_core::state::MergeMode;
use agent_core::status::TaskStatus;

use super::rig::{ONE, Rig, investigator, normal};
use crate::client::{ClientError, Connection};

/// A first-pass finding followed by acceptance of the committed fix.
const CHANGES: &str = "verify:VERDICT: REQUEST_CHANGES\\nFINDINGS\\n1. [MAJOR] TDD-2 src/lib.rs:7 — the case is untested → add coverage\\n   CHECK: read src/lib.rs:7,verify:VERDICT: APPROVE\\nSUMMARY: fixed";
/// A reviewer with no additional findings.
const APPROVE: &str = "verify:VERDICT: APPROVE\\nSUMMARY: fine";
/// What the human says to the lead before they renew its task.
const SAID: &str = "use branch alternative-fix";

/// Sends `command` as the command line does and returns the reply.
async fn ask(rig: &Rig, command: Command) -> Result<Reply, ClientError> {
    Connection::connect(&rig.home.socket_file())
        .await
        .unwrap()
        .request(Client::Cli, command)
        .await
}

// Rule: the notice of a round that waits after "not published" names `agent renew <task>`, and the briefing of the
// renewed task says in its header that the reason of the new session is `human`.
#[tokio::test]
async fn the_notice_names_the_renewal_and_the_briefing_header_names_the_reason_human() {
    let mut rig = Rig::new(
        &[
            normal("claude-1", "work,fix-done"),
            investigator("inv-r1", CHANGES),
            investigator("inv-r2", APPROVE),
        ],
        5,
        true,
    )
    .await;
    rig.start().await;
    let (_team, tasks) = rig.run(&["claude-1"], ONE, MergeMode::Autopilot, None).await;
    let task: TaskId = tasks[0].clone();
    let notice = rig.notification_with("FIXED needs a commit").await;
    rig.status_is(&task, TaskStatus::Waiting).await;
    let next = rig.runs(&task) + 1;
    let target = Target::Task(task.clone());
    ask(
        &rig,
        Command::Say(SayRequest {
            target: target.clone(),
            text: SAID.to_owned().try_into().unwrap(),
        }),
    )
    .await
    .unwrap();

    let renewed = ask(&rig, Command::Renew(ControlRequest { target })).await;

    assert!(matches!(&renewed, Ok(Reply::Applied { .. })), "{renewed:?}");
    let briefing = rig.prompt(&task, next).await;
    rig.stop().await.unwrap();
    assert!(
        notice.contains(&format!("run `agent renew {task}`")),
        "the notice does not name the renewal: {notice}"
    );
    assert!(
        briefing.contains("recorded why as\n`human`:"),
        "the header does not name the reason: {briefing}"
    );
}
