//! The words of a task in a notification: the description, the item's line and the brief's excerpt, and the head and
//! tail every notification gets from them.

use agent_core::ids::{TaskId, TeamId};
use agent_core::kinds::CliKind;
use agent_core::state::split::{PartDefinition, PartKey};
use agent_core::task_title::{CUT_MARK, MAX_EXCERPT_CHARS};

use super::*;
use crate::daemon::notice::sample;
use crate::daemon::notice::{NoticeContext, NoticeKind, layout_notification};
use crate::fixtures::{at, task};

/// The list `text`, which must parse.
fn list(text: &str) -> TaskList {
    TaskList::parse(text).unwrap()
}

/// The list used by most cases: a header line, a brief of two sentences, a feature with two items.
const LIST: &str = "\
Short desc: Notifications name the task

The harness tells identifiers only. The human wants words.

1. Notifications
[ ] 1.1 The daemon names the task in every notification
    Do it.
[ ] 1.2 Show the words in `agent status`
    Do that.
";

/// A split of item 1.1 into two parts.
fn split() -> SplitPlan {
    let part = |title: &str| PartDefinition {
        title: title.to_owned(),
        delivers: "Something".to_owned(),
        checks: "Something".to_owned(),
        depends_on: Vec::new(),
    };
    SplitPlan::new(
        "1.1".parse().unwrap(),
        vec![part("Read the line"), part("Show the line")],
    )
    .unwrap()
}

#[test]
fn the_words_carry_the_given_description_and_the_item_line() {
    let words = sample::words_of(&list(LIST), Some("1.1 Title".to_owned()));

    assert_eq!(words.short_desc(), "Notifications name the task");
    assert_eq!(words.item(), Some("1.1 Title"));
}

#[test]
fn the_words_derive_the_description_of_a_task_that_has_none() {
    let words = sample::words_of(&list("Fix the typo. Then stop.\n"), None);

    assert_eq!(words.short_desc(), "Fix the typo.");
    assert_eq!(words.item(), None);
}

#[test]
fn the_excerpt_is_the_first_sentences_of_the_brief_not_the_header_or_the_items() {
    let words = sample::words_of(&list(LIST), None);

    assert_eq!(
        words.excerpt(),
        "The harness tells identifiers only. The human wants words."
    );
}

#[test]
fn a_brief_longer_than_the_limit_gives_a_cut_excerpt() {
    let brief = "Sentence of the brief. ".repeat(40);

    let words = sample::words_of(&list(&brief), None);

    assert!(words.excerpt().chars().count() <= MAX_EXCERPT_CHARS);
    assert!(words.excerpt().ends_with(CUT_MARK), "{:?}", words.excerpt());
}

#[test]
fn a_list_of_features_without_a_brief_has_an_empty_excerpt() {
    let words = sample::words_of(&list("1. Feature\n[ ] 1.1 Item\n    Do it.\n"), None);

    assert_eq!(words.excerpt(), "");
}

#[test]
fn the_item_line_is_its_number_and_its_full_title() {
    let list = list(LIST);

    assert_eq!(
        item_line(&list, &[], "1.1".parse().unwrap(), None).as_deref(),
        Some("1.1 The daemon names the task in every notification")
    );
    assert_eq!(
        item_line(&list, &[], "1.2".parse().unwrap(), None).as_deref(),
        Some("1.2 Show the words in `agent status`")
    );
}

#[test]
fn an_item_that_the_list_does_not_have_has_no_line() {
    assert_eq!(item_line(&list(LIST), &[], "3.1".parse().unwrap(), None), None);
}

#[test]
fn the_line_of_a_part_is_the_number_of_the_part_and_its_title() {
    let list = list(LIST);
    let plan = [split()];

    assert_eq!(
        item_line(&list, &plan, "1.1".parse().unwrap(), PartKey::at_position(0)).as_deref(),
        Some("1.1a Read the line")
    );
    assert_eq!(
        item_line(&list, &plan, "1.1".parse().unwrap(), PartKey::at_position(1)).as_deref(),
        Some("1.1b Show the line")
    );
}

#[test]
fn a_part_that_the_plan_does_not_have_falls_back_to_the_item_line() {
    let list = list(LIST);

    assert_eq!(
        item_line(&list, &[split()], "1.1".parse().unwrap(), PartKey::at_position(5)).as_deref(),
        Some("1.1 The daemon names the task in every notification")
    );
    assert_eq!(
        item_line(&list, &[], "1.1".parse().unwrap(), PartKey::at_position(0)).as_deref(),
        Some("1.1 The daemon names the task in every notification")
    );
}

/// The context of a notification about task 1 of `claude-1`, with `words`.
fn context<'a>(task_id: &'a TaskId, team: Option<&'a TeamId>, words: TaskWords) -> NoticeContext<'a> {
    NoticeContext {
        repo: "team-agents",
        agent: "claude-1",
        cli: Some(CliKind::Claude),
        model: Some("sonnet"),
        time: at(0),
        offset: agent_core::local_time::Zone::Fixed(time::UtcOffset::UTC),
        task: task_id,
        team,
        pr: None,
        words,
    }
}

#[test]
fn a_notification_starts_with_repo_time_and_task_and_ends_with_the_first_sentences_of_the_task() {
    let task_id = task(1);
    let team_id: TeamId = "team-20261111-153001-t5m8".parse().unwrap();
    let words = sample::words_of(&list(LIST), Some("1.1 The daemon names the task".to_owned()));

    let notification = layout_notification(
        NoticeKind::Questions,
        &context(&task_id, Some(&team_id), words),
        "claude-1 has questions\nQuestions number 1 wait in a file.",
    )
    .unwrap();

    let expected = "\
Repo: team-agents
Time: 2026-11-11 19:15 +00:00
Task: Notifications name the task
Agent: claude-1 (claude, sonnet)
Task ID: task-20261111-153001-k3x9
Team: team-20261111-153001-t5m8
Status: ❓ waiting for answers
Note: claude-1 has questions

1.1 The daemon names the task

Questions number 1 wait in a file.

TASK (task-20261111-153001-k3x9):
The harness tells identifiers only. The human wants words.";
    assert_eq!(notification.body(), expected);
}

#[test]
fn a_notification_without_an_item_a_team_or_a_message_has_no_empty_sections() {
    let task_id = task(1);
    let words = sample::words_of(&list("Fix the typo.\n"), None);

    let notification = layout_notification(NoticeKind::Failed, &context(&task_id, None, words), "It failed").unwrap();

    let expected = "\
Repo: team-agents
Time: 2026-11-11 19:15 +00:00
Task: Fix the typo.
Agent: claude-1 (claude, sonnet)
Task ID: task-20261111-153001-k3x9
Status: ❌ failed
Note: It failed

TASK (task-20261111-153001-k3x9):
Fix the typo.";
    assert_eq!(notification.body(), expected);
}

#[test]
fn a_task_without_a_brief_has_no_task_block() {
    let task_id = task(1);
    let words = sample::words_of(&list("1. Feature\n[ ] 1.1 Item\n    Do it.\n"), None);

    let notification = layout_notification(NoticeKind::Failed, &context(&task_id, None, words), "It failed").unwrap();

    assert!(!notification.body().contains("TASK ("), "{}", notification.body());
}

/// Every kind of notification, one of each: the match forces a new kind to be listed.
fn every_kind() -> Vec<NoticeKind> {
    let kinds = [
        NoticeKind::Questions,
        NoticeKind::QuestionsMissing,
        NoticeKind::Hung,
        NoticeKind::TurnLimitExhausted,
        NoticeKind::TurnLimitNoSession,
        NoticeKind::Paused,
        NoticeKind::Failed,
        NoticeKind::FailedNoTool,
        NoticeKind::NoStatus,
        NoticeKind::NoStatusSignal,
        NoticeKind::StartFailed,
        NoticeKind::RunFailed,
        NoticeKind::Incomplete,
        NoticeKind::Decision,
        NoticeKind::NotAsked,
        NoticeKind::Approved,
        NoticeKind::MergeFailed,
        NoticeKind::Closed,
        NoticeKind::PublishFailed,
        NoticeKind::NoVerdict,
        NoticeKind::Opinion,
        NoticeKind::Merged,
        NoticeKind::TeamAsleep,
        NoticeKind::TeamDone,
    ];
    for kind in kinds {
        match kind {
            NoticeKind::Questions
            | NoticeKind::QuestionsMissing
            | NoticeKind::Hung
            | NoticeKind::TurnLimitExhausted
            | NoticeKind::TurnLimitNoSession
            | NoticeKind::Paused
            | NoticeKind::Failed
            | NoticeKind::FailedNoTool
            | NoticeKind::NoStatus
            | NoticeKind::NoStatusSignal
            | NoticeKind::StartFailed
            | NoticeKind::RunFailed
            | NoticeKind::Incomplete
            | NoticeKind::Decision
            | NoticeKind::NotAsked
            | NoticeKind::Approved
            | NoticeKind::MergeFailed
            | NoticeKind::Closed
            | NoticeKind::PublishFailed
            | NoticeKind::NoVerdict
            | NoticeKind::Opinion
            | NoticeKind::Merged
            | NoticeKind::TeamAsleep
            | NoticeKind::TeamDone => {}
        }
    }
    kinds.to_vec()
}

#[test]
fn every_kind_of_notification_has_the_same_head_and_tail() {
    let task_id = task(1);
    let team_id: TeamId = "team-20261111-153001-t5m8".parse().unwrap();

    for kind in every_kind() {
        let words = sample::words_of(&list(LIST), Some("1.1 The daemon names the task".to_owned()));
        let notification = layout_notification(
            kind,
            &context(&task_id, Some(&team_id), words),
            "The note\nThe message.",
        )
        .unwrap();
        let lines: Vec<&str> = notification.body().lines().collect();

        assert_eq!(lines.first().copied(), Some("Repo: team-agents"), "{kind:?}");
        assert_eq!(lines.get(1).copied(), Some("Time: 2026-11-11 19:15 +00:00"), "{kind:?}");
        assert_eq!(
            lines.get(2).copied(),
            Some("Task: Notifications name the task"),
            "{kind:?}"
        );
        assert!(lines.contains(&"Task ID: task-20261111-153001-k3x9"), "{kind:?}");
        assert!(lines.contains(&"1.1 The daemon names the task"), "{kind:?}");
        assert!(
            notification.body().ends_with(
                "\n\nTASK (task-20261111-153001-k3x9):\nThe harness tells identifiers only. The human wants words."
            ),
            "{kind:?}: {}",
            notification.body()
        );
    }
}
