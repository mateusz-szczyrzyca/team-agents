//! Production notice mapping tests for [`AutopilotNotice`].
//!
//! Verifies that `AutopilotNotice::render` exercises the full production pipeline:
//! prompt resolution -> template rendering -> `layout_notification` with `notice.kind()`.

use std::path::Path;

use agent_core::config::Language;
use agent_core::ids::TeamId;
use agent_core::kinds::CliKind;
use agent_core::notice_status::NoticeStatus;
use agent_core::prompt::Variables;
use agent_runtime::autopilot_prompts::AutopilotPrompts;

use super::AutopilotNotice;
use crate::daemon::notice::NoticeContext;
use crate::fixtures::{at, task};

async fn texts() -> AutopilotPrompts {
    AutopilotPrompts::load(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("../../prompts"),
        &Language::default(),
    )
    .await
    .unwrap()
}

async fn assert_autopilot_notices(cases: &[(AutopilotNotice, NoticeStatus, Variables, &'static str)]) {
    let task_id = task(1);
    let team_id: TeamId = "team-20261111-191500-1111".parse().unwrap();
    let context = NoticeContext {
        repo: "example",
        agent: "claude-1",
        cli: Some(CliKind::Claude),
        model: None,
        time: at(0),
        offset: time::UtcOffset::UTC,
        task: &task_id,
        team: Some(&team_id),
        pr: Some("https://github.example.com/octo/example/pull/1"),
    };
    let texts = texts().await;

    for (notice, expected_status, variables, expected_note) in cases {
        let notification = notice
            .render(&texts, variables, &context)
            .unwrap_or_else(|e| panic!("failed to render {notice:?}: {e}"));

        assert_eq!(
            notification.title(),
            format!("{} example", expected_status.icon()),
            "title mismatch for {notice:?}"
        );
        let expected_status_line = format!("Status: {} {}", expected_status.icon(), expected_status.label());
        assert!(
            notification.body().contains(&expected_status_line),
            "status line {expected_status_line:?} not found in {notice:?} body:\n{}",
            notification.body()
        );
        assert!(
            notification.body().contains(expected_note),
            "expected note {expected_note:?} not found in {notice:?} body:\n{}",
            notification.body()
        );
    }
}

#[tokio::test]
async fn autopilot_lifecycle_notices_render_expected_notes_and_statuses() {
    let cases = [
        (
            AutopilotNotice::TeamAsleep,
            NoticeStatus::Asleep,
            Variables::new()
                .with("team", "team-1")
                .unwrap()
                .with("items", "1.1")
                .unwrap()
                .with("minutes", "15")
                .unwrap()
                .with("tasks", "task-1")
                .unwrap()
                .with("woken", "1")
                .unwrap()
                .with("lead", "claude-1")
                .unwrap(),
            "Note: Team team-1 sleeps with item 1.1 unfinished",
        ),
        (
            AutopilotNotice::Approved,
            NoticeStatus::Approved,
            Variables::new()
                .with("team", "team-1")
                .unwrap()
                .with("repo", "example")
                .unwrap()
                .with("pr", "1")
                .unwrap()
                .with("url", "https://github.example.com/octo/example/pull/1")
                .unwrap(),
            "Note: Pull request 1 of team-1 is approved",
        ),
        (
            AutopilotNotice::PublishFailed,
            NoticeStatus::NotPublished,
            Variables::new()
                .with("team", "team-1")
                .unwrap()
                .with("items", "1.1")
                .unwrap()
                .with("reason", "git push rejected")
                .unwrap(),
            "Note: The pull request of team-1 could not be published",
        ),
        (
            AutopilotNotice::MergeFailed,
            NoticeStatus::NotMerged,
            Variables::new()
                .with("team", "team-1")
                .unwrap()
                .with("pr", "1")
                .unwrap()
                .with("url", "https://github.example.com/octo/example/pull/1")
                .unwrap()
                .with("reason", "conflict")
                .unwrap(),
            "Note: Pull request 1 of team-1 could not be merged",
        ),
        (
            AutopilotNotice::Closed,
            NoticeStatus::Closed,
            Variables::new()
                .with("team", "team-1")
                .unwrap()
                .with("pr", "1")
                .unwrap()
                .with("url", "https://github.example.com/octo/example/pull/1")
                .unwrap(),
            "Note: Pull request 1 of team-1 was closed",
        ),
        (
            AutopilotNotice::Merged,
            NoticeStatus::Merged,
            Variables::new()
                .with("team", "team-1")
                .unwrap()
                .with("pr", "1")
                .unwrap()
                .with("url", "https://github.example.com/octo/example/pull/1")
                .unwrap()
                .with("items", "1.1")
                .unwrap(),
            "Note: Pull request 1 of team-1 is merged",
        ),
    ];

    assert_autopilot_notices(&cases).await;
}

#[tokio::test]
async fn autopilot_review_notices_render_expected_notes_and_statuses() {
    let cases = [
        (
            AutopilotNotice::NoVerdict,
            NoticeStatus::NoVerdict,
            Variables::new()
                .with("reviewer", "claude-2")
                .unwrap()
                .with("pr", "1")
                .unwrap()
                .with("task", "task-2")
                .unwrap()
                .with("why", "it gave none.")
                .unwrap(),
            "Note: claude-2 gave no verdict on pull request 1",
        ),
        (
            AutopilotNotice::Decision,
            NoticeStatus::Decision,
            Variables::new()
                .with("team", "team-1")
                .unwrap()
                .with("pr", "1")
                .unwrap()
                .with("passes", "3")
                .unwrap()
                .with("additional_passes", "5")
                .unwrap()
                .with("findings", "")
                .unwrap(),
            "Note: Pull request 1 of team-1 needs your decision",
        ),
    ];

    assert_autopilot_notices(&cases).await;
}

#[tokio::test]
async fn unanswered_opinion_has_its_own_notice_kind_and_requests_a_human_decision() {
    assert_ne!(AutopilotNotice::Opinion.kind(), AutopilotNotice::NoVerdict.kind());
    assert_autopilot_notices(&[(
        AutopilotNotice::Opinion,
        NoticeStatus::Decision,
        Variables::new()
            .with("round", "round-1")
            .unwrap()
            .with("number", "5")
            .unwrap()
            .with("task", "task-2")
            .unwrap(),
        "Note: Second opinion needs attention",
    )])
    .await;
}
