//! The notifications of a whole team run name the task in words: its description, its item and its first sentences.

use agent_core::state::{ItemPhase, MergeMode};
use agent_core::status::TaskStatus;

use super::rig::{ONE, Rig, investigator, normal};

/// The list `ONE` with a header line before its brief.
fn one_with_a_description() -> String {
    format!("Short desc: Build the thing in words\n\n{ONE}")
}

/// The lines of `notification` below its title.
fn body_lines(notification: &str) -> Vec<&str> {
    notification.lines().skip(1).collect()
}

#[tokio::test]
async fn the_notification_of_a_question_names_the_task_its_item_and_its_first_sentences() {
    let mut rig = Rig::new(
        &[
            normal("claude-1", "ask"),
            investigator("inv-r1", "done"),
            investigator("inv-r2", "done"),
        ],
        5,
        false,
    )
    .await;
    rig.start().await;
    let (_, tasks) = rig
        .run(&["claude-1"], &one_with_a_description(), MergeMode::Autopilot, None)
        .await;
    rig.status_is(&tasks[0], TaskStatus::Questions).await;
    let notification = rig.notification_with("waiting for answers").await;
    rig.stop().await.unwrap();

    let lines = body_lines(&notification);

    assert_eq!(lines.first().copied(), Some("Repo: example"), "{notification}");
    assert!(
        lines.get(1).is_some_and(|line| line.starts_with("Time: ")),
        "{notification}"
    );
    assert_eq!(
        lines.get(2).copied(),
        Some("Task: Build the thing in words"),
        "{notification}"
    );
    assert!(
        lines.contains(&format!("Task ID: {}", tasks[0]).as_str()),
        "{notification}"
    );
    assert!(notification.contains("\n\n1.1 The first item\n\n"), "{notification}");
    assert!(
        notification.ends_with(&format!("\n\nTASK ({}):\nBuild the thing.", tasks[0])),
        "{notification}"
    );
}

#[tokio::test]
async fn a_task_list_without_the_line_has_its_description_derived_from_its_brief() {
    let mut rig = Rig::new(
        &[
            normal("claude-1", "ask"),
            investigator("inv-r1", "done"),
            investigator("inv-r2", "done"),
        ],
        5,
        false,
    )
    .await;
    rig.start().await;
    let (_, tasks) = rig.run(&["claude-1"], ONE, MergeMode::Autopilot, None).await;
    rig.status_is(&tasks[0], TaskStatus::Questions).await;
    let notification = rig.notification_with("waiting for answers").await;
    rig.stop().await.unwrap();

    let lines = body_lines(&notification);

    assert_eq!(lines.get(2).copied(), Some("Task: Build the thing."), "{notification}");
}

#[tokio::test]
async fn the_notification_of_a_merge_names_the_item_of_the_round() {
    let mut rig = Rig::new(
        &[
            normal("claude-1", "work"),
            investigator("inv-r1", "done:VERDICT: APPROVE\\nSUMMARY: fine"),
            investigator("inv-r2", "done:VERDICT: APPROVE\\nSUMMARY: fine"),
        ],
        5,
        false,
    )
    .await;
    rig.start().await;
    rig.run(&["claude-1"], &one_with_a_description(), MergeMode::Autopilot, None)
        .await;
    rig.item_where("1.1", "merged", |phase| matches!(phase, ItemPhase::Done { .. }))
        .await;
    let notification = rig.notification_with("is merged").await;
    rig.stop().await.unwrap();

    assert!(notification.contains("\n\n1.1 The first item\n\n"), "{notification}");
    assert!(
        notification.contains("\nTask: Build the thing in words\n"),
        "{notification}"
    );
}
