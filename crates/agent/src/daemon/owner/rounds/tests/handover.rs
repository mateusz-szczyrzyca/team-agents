//! Regression coverage for completed fix passes that supplied no commit.

use super::rig::{ONE, Rig, investigator, normal};
use agent_core::state::{ItemPhase, MergeMode};

/// A first-pass finding followed by acceptance of the committed fix.
const CHANGES: &str = "verify:VERDICT: REQUEST_CHANGES\\nFINDINGS\\n1. [MAJOR] TDD-2 src/lib.rs:7 — the case is untested → add coverage\\n   CHECK: read src/lib.rs:7,verify:VERDICT: APPROVE\\nSUMMARY: fixed";
/// A reviewer with no additional findings.
const APPROVE: &str = "verify:VERDICT: APPROVE\\nSUMMARY: fine";

#[tokio::test]
async fn a_claimed_fix_without_a_commit_is_returned_and_the_corrected_commit_is_verified() {
    let mut rig = Rig::new(
        &[
            normal("claude-1", "work,fix-done,fix-work"),
            investigator("inv-r1", CHANGES),
            investigator("inv-r2", APPROVE),
        ],
        5,
        false,
    )
    .await;
    rig.start().await;
    let (_, tasks) = rig.run(&["claude-1"], ONE, MergeMode::Autopilot, None).await;
    let correction = rig.prompt(&tasks[0], 3).await;
    rig.item_where("1.1", "merged after the correction", |phase| {
        matches!(phase, ItemPhase::Done { .. })
    })
    .await;
    rig.stop().await.unwrap();

    assert!(correction.contains("FIXED needs a commit"), "{correction}");
    assert!(correction.contains("agent gate"), "{correction}");
    assert_eq!(rig.runs(&tasks[0]), 3);
    assert_eq!(rig.github.reviews(1).len(), 4);
}

#[tokio::test]
async fn repeated_uncommitted_fixes_send_one_notice_naming_the_round_and_recovery_command() {
    let mut rig = Rig::new(
        &[
            normal("claude-1", "work,fix-done"),
            investigator("inv-r1", CHANGES),
            investigator("inv-r2", APPROVE),
        ],
        5,
        false,
    )
    .await;
    rig.start().await;
    let (team, tasks) = rig.run(&["claude-1"], ONE, MergeMode::Autopilot, None).await;
    let notice = rig.notification_with("FIXED needs a commit").await;
    for _ in 0..3 {
        let reply = crate::client::Connection::connect(&rig.home.socket_file())
            .await
            .unwrap()
            .request(
                agent_core::protocol::Client::Cli,
                agent_core::protocol::Command::Status(agent_core::protocol::StatusRequest { target: None }),
            )
            .await
            .unwrap();
        assert!(matches!(reply, agent_core::protocol::Reply::Teams { .. }));
    }
    rig.stop().await.unwrap();
    let state = rig.state();
    let round = state.team(&team).unwrap().rounds().last().unwrap();
    assert!(notice.contains(round.id().as_str()), "{notice}");
    assert!(notice.contains(&format!("agent say {}", tasks[0])), "{notice}");
    assert_eq!(
        rig.notifications()
            .iter()
            .filter(|n| n.contains("FIXED needs a commit"))
            .count(),
        1
    );
    assert_eq!(rig.runs(&tasks[0]), 5);
    assert_eq!(rig.github.reviews(1).len(), 2);
}
