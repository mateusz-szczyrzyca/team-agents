use std::path::Path;

use agent_core::config::Language;
use agent_core::decide::outcome::PauseReason;

use super::*;
use crate::fixtures::{at, task};

async fn texts() -> HarnessPrompts {
    HarnessPrompts::load(&Path::new(env!("CARGO_MANIFEST_DIR")).join("../../prompts"), &Language::default())
        .await
        .unwrap()
}

/// The title and the body of the notification of `notice` about task 1 of `claude-1`.
async fn rendered(notice: &Notice) -> (String, String) {
    let notification = notice.render(&texts().await, "claude-1", &task(1)).unwrap();
    (notification.title().to_owned(), notification.body().to_owned())
}

#[tokio::test]
async fn every_notice_has_a_title_that_names_the_agent_and_the_task() {
    let notices = [
        Notice::Questions {
            round: NonZeroU32::new(2).unwrap(),
            file: PathBuf::from("/h/tasks/t/questions-2.md"),
        },
        Notice::QuestionsMissing {
            file: PathBuf::from("/h/tasks/t/questions-1.md"),
            reason: "no such file".to_owned(),
        },
        Notice::Hung { minutes: 30, strikes: 3 },
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

        assert!(title.contains("claude-1"), "{notice:?}: {title}");
        assert!(title.contains(task(1).as_str()), "{notice:?}: {title}");
        assert!(!body.trim().is_empty(), "{notice:?}");
    }
}

#[tokio::test]
async fn the_body_of_a_notice_has_the_facts_of_it() {
    let (_, questions) = rendered(&Notice::Questions {
        round: NonZeroU32::new(2).unwrap(),
        file: PathBuf::from("/h/tasks/t/questions-2.md"),
    })
    .await;
    let (_, hung) = rendered(&Notice::Hung { minutes: 30, strikes: 3 }).await;
    let (title, paused) = rendered(&Notice::Paused {
        until: at(3600),
        reason: PauseReason::RateLimit,
        evidence: "HTTP 429".to_owned(),
    })
    .await;
    let (_, none) = rendered(&Notice::NoStatus { exit: None }).await;
    let (_, code) = rendered(&Notice::NoStatus { exit: Some(3) }).await;

    assert!(questions.contains("Round 2") && questions.contains("/h/tasks/t/questions-2.md"), "{questions}");
    assert!(hung.contains("30 minutes") && hung.contains("3 times"), "{hung}");
    assert!(title.contains("2026-11-11T20:15:00Z"), "{title}");
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
