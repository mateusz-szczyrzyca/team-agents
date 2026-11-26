//! Regression tests for the bus membership replacement before a replacement run.

use agent_core::state::MergeMode;
use agent_core::status::TaskStatus;

use crate::daemon::owner::rounds::tests::rig::{ONE, Rig, investigator, normal};

/// A rig with a team whose member fails and is replaced.
async fn replacement_rig() -> Rig {
    Rig::new(
        &[
            normal("claude-1", "work"),
            investigator("inv-r1", "sleep"),
            investigator("inv-r2", "sleep"),
        ],
        5,
        true,
    )
    .await
}

// RED: Overdue member replacement updates real bus before run history
#[tokio::test]
async fn overdue_member_replacement_updates_real_bus_before_run_history() {
    let mut rig = replacement_rig().await;
    rig.start().await;

    // Run a team with a member that will fail
    let (team, tasks) = rig.run(&["claude-1"], ONE, MergeMode::Autopilot, None).await;

    // Simulate member failure by manipulating the worktree
    let member_worktree = rig.worktree(&tasks[0]);
    std::fs::write(member_worktree.join(".fake-action"), "fail").unwrap();

    // Wait for the member to fail
    rig.status_is(&tasks[0], TaskStatus::Failed).await;

    // Stop the daemon to check the bus state
    rig.stop().await.unwrap();

    // Check that the bus was updated with the replacement information before run history
    let bus_dir = rig.home.root().join("bus");
    assert!(bus_dir.exists(), "bus directory should exist");

    // TODO: Add actual assertion about bus state once we implement the production code
    // This is a RED test - it should fail until we implement the feature
    panic!("Overdue member replacement should update real bus before run history");
}

// RED: Lead message reaches new member
#[tokio::test]
async fn lead_message_reaches_new_member() {
    let mut rig = replacement_rig().await;
    rig.start().await;

    // Run a team with a member that will fail
    let (team, tasks) = rig.run(&["claude-1"], ONE, MergeMode::Autopilot, None).await;

    // Simulate member failure
    let member_worktree = rig.worktree(&tasks[0]);
    std::fs::write(member_worktree.join(".fake-action"), "fail").unwrap();

    // Wait for the member to fail
    rig.status_is(&tasks[0], TaskStatus::Failed).await;

    // Stop the daemon to check messaging
    rig.stop().await.unwrap();

    // TODO: Add actual assertion about message delivery once we implement the feature
    // This is a RED test - it should fail until we implement the feature
    panic!("Lead message should reach new member after replacement");
}

// RED: Serena reminder succeeds
#[tokio::test]
async fn serena_reminder_succeeds() {
    let mut rig = replacement_rig().await;
    rig.start().await;

    // Run a team with a member that will fail
    let (team, tasks) = rig.run(&["claude-1"], ONE, MergeMode::Autopilot, None).await;

    // Simulate member failure
    let member_worktree = rig.worktree(&tasks[0]);
    std::fs::write(member_worktree.join(".fake-action"), "fail").unwrap();

    // Wait for the member to fail
    rig.status_is(&tasks[0], TaskStatus::Failed).await;

    // Stop the daemon
    rig.stop().await.unwrap();

    // TODO: Add actual assertion about Serena reminder once we implement the feature
    // This is a RED test - it should fail until we implement the feature
    panic!("Serena reminder should succeed after member replacement");
}

// RED: Storage failure blocks run then next pass retries
#[tokio::test]
async fn storage_failure_blocks_run_then_next_pass_retries() {
    let mut rig = replacement_rig().await;
    rig.start().await;

    // Run a team with a member that will fail
    let (team, tasks) = rig.run(&["claude-1"], ONE, MergeMode::Autopilot, None).await;

    // Simulate storage failure
    let member_worktree = rig.worktree(&tasks[0]);
    std::fs::write(member_worktree.join(".fake-action"), "storage-fail").unwrap();

    // Wait for the member to be blocked
    // TODO: Add actual assertion about storage failure blocking once we implement the feature
    // This is a RED test - it should fail until we implement the feature
    panic!("Storage failure should block run then next pass should retry");
}

// RED: NotFound is complete
#[tokio::test]
async fn not_found_is_complete() {
    let mut rig = replacement_rig().await;
    rig.start().await;

    // Run a team with a member that will fail
    let (team, tasks) = rig.run(&["claude-1"], ONE, MergeMode::Autopilot, None).await;

    // Simulate NotFound error
    let member_worktree = rig.worktree(&tasks[0]);
    std::fs::write(member_worktree.join(".fake-action"), "not-found").unwrap();

    // Wait for the NotFound handling
    // TODO: Add actual assertion about NotFound handling once we implement the feature
    // This is a RED test - it should fail until we implement the feature
    panic!("NotFound should be handled completely");
}

// RED: Busy member handover updates bus
#[tokio::test]
async fn busy_member_handover_updates_bus() {
    let mut rig = replacement_rig().await;
    rig.start().await;

    // Run a team with a member that will become busy
    let (team, tasks) = rig.run(&["claude-1"], ONE, MergeMode::Autopilot, None).await;

    // Simulate busy member
    let member_worktree = rig.worktree(&tasks[0]);
    std::fs::write(member_worktree.join(".fake-action"), "busy").unwrap();

    // Wait for the busy handling
    // TODO: Add actual assertion about bus updates once we implement the feature
    // This is a RED test - it should fail until we implement the feature
    panic!("Busy member handover should update bus");
}

// RED: Solo handover makes no team-replace call
#[tokio::test]
async fn solo_handover_makes_no_team_replace_call() {
    let mut rig = Rig::new(&[normal("claude-1", "work")], 5, true).await;
    rig.start().await;

    // Run a solo agent that will fail
    let (team, tasks) = rig.run(&["claude-1"], ONE, MergeMode::Autopilot, None).await;

    // Simulate solo agent failure
    let member_worktree = rig.worktree(&tasks[0]);
    std::fs::write(member_worktree.join(".fake-action"), "fail").unwrap();

    // Wait for the failure
    rig.status_is(&tasks[0], TaskStatus::Failed).await;

    // Stop the daemon
    rig.stop().await.unwrap();

    // TODO: Add actual assertion about no team-replace call once we implement the feature
    // This is a RED test - it should fail until we implement the feature
    panic!("Solo handover should make no team-replace call");
}
