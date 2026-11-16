use std::path::Path;

use agent_core::config::Language;
use agent_core::decide::outcome::PauseReason;

use super::*;
use crate::fixtures::{at, task};

async fn texts() -> HarnessPrompts {
    HarnessPrompts::load(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("../../prompts"),
        &Language::default(),
    )
    .await
    .unwrap()
}

/// The title and the body of the notification of `notice` about task 1 of `claude-1`.
async fn rendered(notice: &Notice) -> (String, String) {
    let task_id = task(1);
    let context = NoticeContext {
        repo: "team-agents",
        agent: "claude-1",
        cli: Some(agent_core::kinds::CliKind::Claude),
        model: None,
        time: at(0),
        offset: time::UtcOffset::UTC,
        task: &task_id,
        team: None,
        pr: None,
    };
    let notification = notice.render(&texts().await, &context).unwrap();
    (notification.title().to_owned(), notification.body().to_owned())
}

#[tokio::test]
async fn every_notice_has_a_title_that_names_the_agent_and_the_task() {
    let notices = [
        Notice::Questions {
            number: NonZeroU32::new(2).unwrap(),
            file: PathBuf::from("/h/tasks/t/questions-2.md"),
        },
        Notice::QuestionsMissing {
            file: PathBuf::from("/h/tasks/t/questions-1.md"),
            reason: "no such file".to_owned(),
        },
        Notice::Hung {
            minutes: 30,
            strikes: 3,
        },
        Notice::Paused {
            until: at(3600),
            reason: PauseReason::Quota,
            evidence: "usage limit".to_owned(),
        },
        Notice::Failed {
            message: "It broke.".to_owned(),
        },
        Notice::NoStatus { exit: Some(3) },
        Notice::StartFailed {
            reason: "no such program".to_owned(),
        },
    ];

    for notice in notices {
        let (title, body) = rendered(&notice).await;

        assert!(title.ends_with("team-agents"), "{notice:?}: {title}");
        assert!(body.contains("claude-1"), "{notice:?}: {body}");
        assert!(body.contains(task(1).as_str()), "{notice:?}: {body}");
        assert!(!body.trim().is_empty(), "{notice:?}");
    }
}

#[tokio::test]
async fn the_body_of_a_notice_has_the_facts_of_it() {
    let (_, questions) = rendered(&Notice::Questions {
        number: NonZeroU32::new(2).unwrap(),
        file: PathBuf::from("/h/tasks/t/questions-2.md"),
    })
    .await;
    let (_, hung) = rendered(&Notice::Hung {
        minutes: 30,
        strikes: 3,
    })
    .await;
    let (_, paused) = rendered(&Notice::Paused {
        until: at(3600),
        reason: PauseReason::RateLimit,
        evidence: "HTTP 429".to_owned(),
    })
    .await;
    let (_, none) = rendered(&Notice::NoStatus { exit: None }).await;
    let (_, code) = rendered(&Notice::NoStatus { exit: Some(3) }).await;

    assert!(
        questions.contains("Questions number 2") && questions.contains("/h/tasks/t/questions-2.md"),
        "{questions}"
    );
    assert!(hung.contains("30 minutes") && hung.contains("3 times"), "{hung}");
    assert!(paused.contains("2026-11-11T20:15:00Z"), "{paused}");
    assert!(paused.contains("rate_limit") && paused.contains("HTTP 429"), "{paused}");
    assert!(none.contains("exit: signal"), "{none}");
    assert!(code.contains("exit: 3"), "{code}");
}

#[tokio::test]
async fn a_value_that_looks_like_a_placeholder_is_shown_as_it_is() {
    let (_, body) = rendered(&Notice::Failed {
        message: "{{task}} {{".to_owned(),
    })
    .await;

    assert!(body.contains("{{task}} {{"), "{body}");
}

#[tokio::test]
async fn a_run_that_failed_after_its_start_is_not_told_as_a_start() {
    let (_, body) = rendered(&Notice::RunFailed {
        reason: "cannot write the activity".to_owned(),
    })
    .await;

    assert!(body.contains("failed while it ran"), "{body}");
    assert!(!body.contains("could not be started"), "{body}");
    assert!(body.contains("cannot write the activity"), "{body}");
}

#[test]
fn layout_as_a_table_every_field_present() {
    let task_id: TaskId = "task-20261111-191500-1111".parse().unwrap();
    let team_id: TeamId = "team-20261111-153001-t5m8".parse().unwrap();
    let time = at(0);
    let offset = time::UtcOffset::from_hms(2, 0, 0).unwrap();

    let context = NoticeContext {
        repo: "team-agents",
        agent: "codex-1",
        cli: Some(CliKind::Codex),
        model: Some("o3-mini"),
        time,
        offset,
        task: &task_id,
        team: Some(&team_id),
        pr: Some("https://github.com/org/team-agents/pull/42"),
    };

    let rendered = "Pull request 42 of team-agents is approved\nBoth reviewers approved pull request #42.";
    let notification = layout_notification(NoticeKind::Approved, &context, rendered).unwrap();

    assert_eq!(notification.title(), "✅✅ team-agents");
    let expected_body = "\
Repo: team-agents
Agent: codex-1 (codex, o3-mini)
Time: 2026-11-11 21:15
Task: task-20261111-191500-1111
Team: team-20261111-153001-t5m8
Status: ✅✅ approved: ready to merge
Note: Pull request 42 of team-agents is approved
PR: https://github.com/org/team-agents/pull/42

Both reviewers approved pull request #42.";

    assert_eq!(notification.body(), expected_body);
}

#[test]
fn layout_optional_fields_absent() {
    let task_id: TaskId = "task-20261111-191500-1111".parse().unwrap();
    let time = at(0);
    let offset = time::UtcOffset::UTC;

    let context = NoticeContext {
        repo: "team-agents",
        agent: "codex-1",
        cli: Some(CliKind::Codex),
        model: None,
        time,
        offset,
        task: &task_id,
        team: None,
        pr: None,
    };

    let rendered = "codex-1 ended without a status (task-20261111-191500-1111)\nThe run ended without a status line.";
    let notification = layout_notification(NoticeKind::NoStatus, &context, rendered).unwrap();

    assert_eq!(notification.title(), "❌ team-agents");
    let expected_body = "\
Repo: team-agents
Agent: codex-1 (codex)
Time: 2026-11-11 19:15
Task: task-20261111-191500-1111
Status: ❌ failed
Note: codex-1 ended without a status (task-20261111-191500-1111)

The run ended without a status line.";

    assert_eq!(notification.body(), expected_body);
}

#[test]
fn layout_empty_body() {
    let task_id: TaskId = "task-20261111-191500-1111".parse().unwrap();
    let time = at(0);
    let offset = time::UtcOffset::UTC;

    let context = NoticeContext {
        repo: "team-agents",
        agent: "codex-1",
        cli: Some(CliKind::Codex),
        model: None,
        time,
        offset,
        task: &task_id,
        team: None,
        pr: None,
    };

    let rendered = "Single line notice title with no body";
    let notification = layout_notification(NoticeKind::NoStatus, &context, rendered).unwrap();

    assert_eq!(notification.title(), "❌ team-agents");
    let expected_body = "\
Repo: team-agents
Agent: codex-1 (codex)
Time: 2026-11-11 19:15
Task: task-20261111-191500-1111
Status: ❌ failed
Note: Single line notice title with no body";

    assert_eq!(notification.body(), expected_body);
}

#[test]
fn notice_kind_maps_to_expected_status() {
    let expected = [
        (NoticeKind::Questions, NoticeStatus::Questions),
        (NoticeKind::QuestionsMissing, NoticeStatus::Failed),
        (NoticeKind::Incomplete, NoticeStatus::Questions),
        (NoticeKind::Decision, NoticeStatus::Decision),
        (NoticeKind::NotAsked, NoticeStatus::Decision),
        (NoticeKind::Hung, NoticeStatus::Hung),
        (NoticeKind::Paused, NoticeStatus::Paused),
        (NoticeKind::Approved, NoticeStatus::Approved),
        (NoticeKind::MergeFailed, NoticeStatus::NotMerged),
        (NoticeKind::Closed, NoticeStatus::Closed),
        (NoticeKind::PublishFailed, NoticeStatus::NotPublished),
        (NoticeKind::NoVerdict, NoticeStatus::NoVerdict),
        (NoticeKind::Merged, NoticeStatus::Merged),
        (NoticeKind::TeamAsleep, NoticeStatus::Asleep),
        (NoticeKind::TeamDone, NoticeStatus::Done),
        (NoticeKind::Failed, NoticeStatus::Failed),
        (NoticeKind::NoStatus, NoticeStatus::Failed),
        (NoticeKind::NoStatusSignal, NoticeStatus::Failed),
        (NoticeKind::StartFailed, NoticeStatus::Failed),
        (NoticeKind::RunFailed, NoticeStatus::Failed),
        (NoticeKind::TurnLimitExhausted, NoticeStatus::Failed),
        (NoticeKind::TurnLimitNoSession, NoticeStatus::Failed),
    ];
    for (kind, expected_status) in expected {
        assert_eq!(kind.status(), expected_status, "{kind:?} status mismatch");
    }
}

#[tokio::test]
async fn actual_notice_templates_render_expected_notes_and_statuses() {
    let task_id = task(1);
    let context = NoticeContext {
        repo: "team-agents",
        agent: "claude-1",
        cli: Some(CliKind::Claude),
        model: None,
        time: at(0),
        offset: time::UtcOffset::UTC,
        task: &task_id,
        team: None,
        pr: None,
    };
    let texts = texts().await;

    // Notice::Questions
    let notice = Notice::Questions {
        number: NonZeroU32::new(1).unwrap(),
        file: PathBuf::from("/tasks/t/questions-1.md"),
    };
    let rendered = notice.render(&texts, &context).unwrap();
    assert_eq!(rendered.title(), "❓ team-agents");
    assert!(rendered.body().contains("Status: ❓ waiting for answers"));
    assert!(
        rendered
            .body()
            .contains(&format!("Note: claude-1 has questions ({task_id})"))
    );

    // Notice::Hung
    let notice = Notice::Hung {
        minutes: 30,
        strikes: 3,
    };
    let rendered = notice.render(&texts, &context).unwrap();
    assert_eq!(rendered.title(), "⌛ team-agents");
    assert!(rendered.body().contains("Status: ⌛ hung"));
    assert!(rendered.body().contains(&format!("Note: claude-1 is hung ({task_id})")));

    // Notice::Paused
    let notice = Notice::Paused {
        until: at(3600),
        reason: PauseReason::Quota,
        evidence: "quota exceeded".to_owned(),
    };
    let rendered = notice.render(&texts, &context).unwrap();
    assert_eq!(rendered.title(), "⏸ team-agents");
    assert!(rendered.body().contains("Status: ⏸ paused"));
    assert!(rendered.body().contains("Note: claude-1 is paused until "));
    assert!(rendered.body().contains(&format!("({task_id})")));

    // Notice::Failed
    let notice = Notice::Failed {
        message: "run failed".to_owned(),
    };
    let rendered = notice.render(&texts, &context).unwrap();
    assert_eq!(rendered.title(), "❌ team-agents");
    assert!(rendered.body().contains("Status: ❌ failed"));
    assert!(rendered.body().contains(&format!("Note: claude-1 failed ({task_id})")));

    // Notice::TurnLimitExhausted
    let notice = Notice::TurnLimitExhausted { count: 4, limit: 3 };
    let rendered = notice.render(&texts, &context).unwrap();
    assert_eq!(rendered.title(), "❌ team-agents");
    assert!(rendered.body().contains("Status: ❌ failed"));
    assert!(rendered.body().contains("Note: claude-1: turn limit resumes exhausted"));
}

#[test]
fn longest_allowed_repository_name_stays_within_notification_limit() {
    let task_id: TaskId = "task-20261111-191500-1111".parse().unwrap();
    let time = at(0);
    let offset = time::UtcOffset::UTC;
    let long_repo = "a".repeat(100);

    let context = NoticeContext {
        repo: &long_repo,
        agent: "agent-1",
        cli: Some(CliKind::Claude),
        model: None,
        time,
        offset,
        task: &task_id,
        team: None,
        pr: None,
    };

    let notification = layout_notification(NoticeKind::Approved, &context, "Title\nBody").unwrap();
    assert!(notification.title().len() <= agent_runtime::notify::MAX_TITLE_BYTES);
    assert_eq!(notification.title(), format!("✅✅ {long_repo}"));
}
