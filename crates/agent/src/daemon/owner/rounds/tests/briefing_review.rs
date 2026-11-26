//! The briefing of a new session of a task in review tells the pull request, where its review stands and the findings: a
//! task renewed by the human in a fix pass, and a task handed over because its agent was busy.

use agent_core::protocol::{Client, Command, ControlRequest, Reply, Target};
use agent_core::state::{FindingStatus, ItemPhase, MergeMode};
use agent_core::status::TaskStatus;

use super::rig::{ONE, Rig, investigator, normal};
use crate::client::{ClientError, Connection};

/// Two findings in the first pass; the verification of the fix resolves the first and leaves the second open.
const TWO_FINDINGS: &str = "verify:VERDICT: REQUEST_CHANGES\\nFINDINGS\\n1. [MINOR] DOC-1 src/lib.rs:7 — the wording is unclear → clarify it\\n   CHECK: read src/lib.rs:7\\n2. [MAJOR] TDD-2 src/lib.rs:8 — the case is untested → add coverage\\n   CHECK: read src/lib.rs:8,verify-partial:VERDICT: REQUEST_CHANGES\\nSUMMARY: one is still open";
/// A reviewer with nothing to add, then a verification that leaves the second finding open too.
const PARTLY: &str =
    "verify:VERDICT: APPROVE\\nSUMMARY: fine,verify-partial:VERDICT: REQUEST_CHANGES\\nSUMMARY: one is still open";

/// Two findings in the first pass, given once the test lets the reviewer go.
const HELD_FINDINGS: &str = "held-verify:VERDICT: REQUEST_CHANGES\\nFINDINGS\\n1. [MINOR] DOC-1 src/lib.rs:7 — the wording is unclear → clarify it\\n   CHECK: read src/lib.rs:7\\n2. [MAJOR] TDD-2 src/lib.rs:8 — the case is untested → add coverage\\n   CHECK: read src/lib.rs:8";
/// A reviewer with nothing to add, given once the test lets the reviewer go.
const HELD_APPROVAL: &str = "held-verify:VERDICT: APPROVE\\nSUMMARY: fine";

/// Sends `command` as the command line does and returns the reply.
async fn ask(rig: &Rig, command: Command) -> Result<Reply, ClientError> {
    Connection::connect(&rig.home.socket_file())
        .await
        .unwrap()
        .request(Client::Cli, command)
        .await
}

// Rule: a solo task that waits in a fix pass is renewed by the human; the prompt of its next run is the briefing with the
// section of the pull request: its number and branch, the fix pass, the open finding #2 and the resolved finding #1.
#[tokio::test]
async fn a_renewed_task_in_a_fix_pass_is_told_its_pull_request_and_which_findings_are_open() {
    let mut rig = Rig::new(
        &[
            normal("claude-1", "work,fix-work,fix-done"),
            investigator("inv-r1", TWO_FINDINGS),
            investigator("inv-r2", PARTLY),
        ],
        5,
        true,
    )
    .await;
    rig.start().await;
    let (team, tasks) = rig.run(&["claude-1"], ONE, MergeMode::Autopilot, None).await;
    let task = tasks[0].clone();
    rig.notification_with("FIXED needs a commit").await;
    rig.status_is(&task, TaskStatus::Waiting).await;
    let runs_before = rig.runs(&task);

    let renewed = ask(
        &rig,
        Command::Renew(ControlRequest {
            target: Target::Task(task.clone()),
        }),
    )
    .await;

    assert!(matches!(&renewed, Ok(Reply::Applied { .. })), "{renewed:?}");
    let briefing = rig.prompt(&task, runs_before + 1).await;
    let state = rig.state();
    rig.stop().await.unwrap();
    let item = state.team(&team).unwrap().item("1.1".parse().unwrap()).unwrap();
    assert!(matches!(item.phase(), ItemPhase::Fixing { .. }), "{:?}", item.phase());
    let ledger = item.findings().unwrap().entries();
    assert_eq!(
        ledger
            .iter()
            .map(|entry| (entry.number.get(), entry.status))
            .collect::<Vec<_>>(),
        [(1, FindingStatus::Resolved), (2, FindingStatus::Open)]
    );
    let pass = item.fix_passes().len();
    assert!(briefing.contains("## The pull request and its review"), "{briefing}");
    assert!(briefing.contains("Pull request #1 on branch "), "{briefing}");
    assert!(briefing.contains(&format!("fix pass {pass} of 5")), "{briefing}");
    assert!(
        briefing.contains("- #2 [MAJOR] src/lib.rs:8 — the case is untested → add coverage (open;"),
        "{briefing}"
    );
    assert!(
        briefing.contains("- #1 [MINOR] src/lib.rs:7 — the wording is unclear → clarify it (resolved;"),
        "{briefing}"
    );
}

// Rule: the briefing is the briefing's, whoever starts the new session: a task in a fix pass whose agent is busy in another
// team is handed over to a free agent, and the prompt of the new holder tells the pull request, the fix pass and the open
// findings, as it does after `agent renew`.
#[tokio::test]
async fn a_task_handed_over_in_a_fix_pass_because_its_agent_is_busy_is_told_its_pull_request_and_its_findings() {
    let mut rig = Rig::new(
        &[
            normal("claude-2", "work-or-hold"),
            normal("claude-3", "work,fix-done"),
            investigator("inv-r1", HELD_FINDINGS),
            investigator("inv-r2", HELD_APPROVAL),
        ],
        5,
        false,
    )
    .await;
    let config = std::fs::read_to_string(rig.home.config_file()).unwrap();
    std::fs::write(
        rig.home.config_file(),
        config + "\n[replacement]\nagents = [\"claude-3\"]\nbusy_after = 3600\n\n[limits]\nidle_team_minutes = 0\n",
    )
    .unwrap();
    rig.start().await;
    let (team, tasks) = rig.run(&["claude-2"], ONE, MergeMode::Autopilot, None).await;
    let task = tasks[0].clone();
    let item_id = "1.1".parse().unwrap();
    let state = rig
        .state_where("the reviewers to hold the review of the item", |state| {
            state.team(&team).and_then(|team| team.item(item_id)).is_some_and(
                |progress| matches!(progress.phase(), ItemPhase::InReview { reviews, .. } if reviews.len() == 2),
            )
        })
        .await;
    let ItemPhase::InReview { reviews, .. } = state.team(&team).unwrap().item(item_id).unwrap().phase().clone() else {
        panic!("the item is not in review");
    };
    for review in &reviews {
        rig.wait_for("a reviewer to start", || rig.runs(&review.task) >= 1)
            .await;
    }
    // The agent of the item is busy with a run of its own when the reviews send the item back to it.
    let holder_list = ONE.replace("The first item", "Holds the agent");
    let (_, holder) = rig.run(&["claude-2"], &holder_list, MergeMode::NoMerge, None).await;
    rig.status_is(&holder[0], TaskStatus::Running).await;
    for review in &reviews {
        std::fs::write(rig.worktree(&review.task).join(".fake-continue"), "").unwrap();
    }
    rig.state_where("the item to wait for its busy agent in the fix pass", |state| {
        state
            .find_task(&task)
            .is_some_and(|found| found.member.waiting_since().is_some())
    })
    .await;
    rig.advance_history_days(1);
    rig.state_where("the task to be handed over to claude-3", |state| {
        state
            .find_task(&task)
            .is_some_and(|found| found.member.agent().name() == "claude-3")
    })
    .await;

    let briefing = rig.prompt(&task, 2).await;
    let state = rig.state();
    std::fs::write(rig.worktree(&holder[0]).join(".fake-continue"), "").unwrap();
    rig.stop().await.unwrap();

    let item = state.team(&team).unwrap().item(item_id).unwrap();
    assert!(matches!(item.phase(), ItemPhase::Fixing { .. }), "{:?}", item.phase());
    assert!(briefing.contains("`busy`"), "{briefing}");
    assert!(briefing.contains("Pull request #1 on branch "), "{briefing}");
    assert!(briefing.contains("fix pass 1 of 5"), "{briefing}");
    assert!(
        briefing.contains("- #2 [MAJOR] src/lib.rs:8 — the case is untested → add coverage (open;"),
        "{briefing}"
    );
    assert!(
        briefing.contains("- #1 [MINOR] src/lib.rs:7 — the wording is unclear → clarify it (open;"),
        "{briefing}"
    );
}
