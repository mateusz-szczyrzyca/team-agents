//! The notifications of a whole team run name the task in words: its description, its item and its first sentences.

use agent_core::state::{ItemPhase, MergeMode};
use agent_core::status::TaskStatus;

use super::rig::{ONE, PARALLEL, Rig, investigator, normal};

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

/// What the daemon answers `agent status` with for `target`: the one team it lists.
async fn listed_team(rig: &Rig, target: Option<agent_core::protocol::Target>) -> agent_core::protocol::TeamSummary {
    use agent_core::protocol::{Client, Command, Reply, StatusRequest};

    let reply = crate::client::Connection::connect(&rig.home.socket_file())
        .await
        .unwrap()
        .request(Client::Cli, Command::Status(StatusRequest { target }))
        .await
        .unwrap();
    let (Reply::Teams { mut teams } | Reply::Overview { mut teams, .. }) = reply else {
        panic!("the status answers with teams: {reply:?}");
    };
    assert_eq!(teams.len(), 1, "{teams:?}");
    teams.remove(0)
}

#[tokio::test]
async fn the_status_of_a_team_and_of_its_task_say_the_description_also_after_a_restart() {
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
    let of_the_team = listed_team(&rig, None).await;
    let of_the_task = listed_team(&rig, Some(agent_core::protocol::Target::Task(tasks[0].clone()))).await;
    rig.stop().await.unwrap();
    rig.start().await;
    let after_restart = listed_team(&rig, None).await;
    rig.stop().await.unwrap();

    for summary in [&of_the_team, &of_the_task, &after_restart] {
        assert_eq!(summary.short_desc.as_deref(), Some("Build the thing in words"));
    }
    assert!(
        crate::commands::status::render(std::slice::from_ref(&of_the_team))
            .lines()
            .next()
            .is_some_and(|line| line.ends_with("\"Build the thing in words\""))
    );
}

#[tokio::test]
async fn the_status_of_a_team_whose_list_names_nothing_says_the_derived_description() {
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
    let summary = listed_team(&rig, None).await;
    rig.stop().await.unwrap();

    assert_eq!(summary.short_desc.as_deref(), Some("Build the thing."));
}

#[tokio::test]
async fn a_notice_about_one_item_of_two_names_that_item_and_not_the_other() {
    let mut rig = Rig::new(
        &[
            investigator("inv-lead", "sleep,work"),
            normal("claude-1", "fail"),
            normal("claude-2", "fail"),
            investigator("inv-r1", "done"),
            investigator("inv-r2", "done"),
        ],
        5,
        true,
    )
    .await;
    rig.wake_sleeping_teams();
    rig.start().await;
    let (_, tasks) = rig
        .run(
            &["inv-lead", "claude-1", "claude-2"],
            PARALLEL,
            MergeMode::Autopilot,
            Some("inv-lead"),
        )
        .await;
    rig.status_is(&tasks[1], TaskStatus::Failed).await;
    rig.status_is(&tasks[2], TaskStatus::Failed).await;
    rig.status_is(&tasks[0], TaskStatus::Waiting).await;
    let first = rig.notification_with("item 1.1 behind failed member").await;
    let second = rig.notification_with("item 2.1 behind failed member").await;
    rig.stop().await.unwrap();

    assert!(first.contains("\n\n1.1 One\n\n"), "{first}");
    assert!(!first.contains("2.1 Two"), "{first}");
    assert!(second.contains("\n\n2.1 Two\n\n"), "{second}");
    assert!(!second.contains("1.1 One"), "{second}");
}

#[tokio::test]
async fn the_notification_that_a_team_is_done_names_no_item() {
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
    rig.run(&["claude-1"], ONE, MergeMode::Autopilot, None).await;
    rig.item_where("1.1", "merged", |phase| matches!(phase, ItemPhase::Done { .. }))
        .await;
    let notification = rig.notification_with("is finished").await;
    rig.stop().await.unwrap();

    assert!(!notification.contains("1.1 The first item"), "{notification}");
    assert!(notification.contains("\nTask: Build the thing.\n"), "{notification}");
}
