//! The briefing the daemon renders from its facts: the note on unread messages left out follows the bus's reader.

use std::num::NonZeroU64;
use std::path::Path;

use agent_core::config::Language;
use agent_core::questions::Verdict;
use agent_core::state::RunEnd;
use agent_core::status::RunOutcome;

use super::*;

/// The facts of a briefing whose unread messages are `completeness`.
fn facts(completeness: Completeness) -> BriefingFacts {
    BriefingFacts {
        reason: Trouble::NoStatus,
        base: "main".parse().unwrap(),
        branch: "agents/claude-1/task-20261120-144101-r7q2".parse().unwrap(),
        commits: Vec::new(),
        uncommitted: Vec::new(),
        plan: None,
        messages: vec![UnreadMessage {
            id: NonZeroU64::new(1508).unwrap(),
            from: "inv-opus".to_owned(),
            text: "Work on item 1.1 only.".to_owned(),
        }],
        completeness,
        snapshot: "refs/agents/snapshots/round-1/task-20261120-144101-r7q2/1".to_owned(),
    }
}

/// The briefing of `facts`, rendered through the catalog of the repository.
async fn render(facts: &BriefingFacts) -> String {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../prompts");
    let texts = HarnessPrompts::load(&dir, &Language::default()).await.unwrap();
    facts
        .render(&texts, &"task-20261120-144101-r7q2".parse().unwrap(), "Fix the parser.")
        .unwrap()
}

/// The note of the catalog on unread messages left out.
const NOTE: &str = "Older unread messages are left out of this list: read them all with `read_messages`";

#[tokio::test]
async fn a_briefing_whose_reader_gave_only_the_newest_messages_says_older_ones_are_left_out() {
    let text = render(&facts(Completeness::Newest)).await;

    assert!(text.contains(NOTE), "{text}");
    assert!(text.contains("- #1508 inv-opus:"), "{text}");
}

#[tokio::test]
async fn a_briefing_with_every_unread_message_has_no_note() {
    let text = render(&facts(Completeness::All)).await;

    assert!(!text.contains(NOTE), "{text}");
    assert!(text.contains("- #1508 inv-opus:"), "{text}");
}

#[test]
fn a_run_whose_end_was_never_recorded_is_told_as_cut() {
    assert_eq!(run_end_word(None), "cut before its end was recorded");
}

#[test]
fn each_way_a_run_ends_has_its_word_in_the_briefing() {
    let cases = [
        (
            RunEnd::Reported {
                outcome: RunOutcome::Done,
            },
            "DONE",
        ),
        (
            RunEnd::Reported {
                outcome: RunOutcome::Questions,
            },
            "QUESTIONS",
        ),
        (
            RunEnd::Reported {
                outcome: RunOutcome::Failed,
            },
            "FAILED",
        ),
        (RunEnd::NoStatus { exit_code: Some(1) }, "no status"),
        (RunEnd::Hung, "hang"),
        (RunEnd::TurnLimit, "turn limit"),
        (RunEnd::Stopped, "stopped"),
        (RunEnd::RateLimited, "failed"),
        (RunEnd::QuotaExhausted { resets_at: None }, "failed"),
    ];
    for (end, word) in cases {
        assert_eq!(run_end_word(Some(&end)), word, "{end:?}");
    }
}

#[test]
fn only_the_newest_items_are_shown_and_a_flag_says_older_ones_are_left_out() {
    assert_eq!(newest_shown(vec![1, 2, 3], 10), (vec![1, 2, 3], false));
    assert_eq!(newest_shown(vec![1, 2, 3, 4], 2), (vec![3, 4], true));
    assert_eq!(newest_shown(Vec::<u8>::new(), 2), (Vec::new(), false));
}

#[test]
fn an_accepted_verdict_is_ok_and_an_own_answer_is_its_text() {
    assert_eq!(verdict_word(&Verdict::Accept), "ok");
    let own = Verdict::from_text("No, drop it.").unwrap().unwrap();
    assert_eq!(verdict_word(&own), "No, drop it.");
}
