//! The opinion part of the story consumed by agent log.
use super::*;

#[test]
fn opinions_are_told_in_words_with_the_first_line_of_the_evidence() {
    let mut page = Page::default();
    let at = "2026-11-11T19:50:00Z".parse().unwrap();
    let pr = crate::state::PrNumber::new(std::num::NonZeroU32::MIN);
    let number = std::num::NonZeroU32::MIN;
    for event in [
        HistoryEvent::SecondOpinionAsked {
            pr,
            number,
            pass: 1,
            by: "claude-1".parse().unwrap(),
            why: "café path is unreachable\nCheck the boundary.".into(),
        },
        HistoryEvent::SecondOpinionAsked {
            pr,
            number,
            pass: 1,
            by: "inv-first".parse().unwrap(),
            why: "- inv-first (task-20261111-193001-k3x9): the run failed.\n".into(),
        },
        HistoryEvent::FindingConfirmed {
            pr,
            number,
            pass: 1,
            by: "inv-second".parse().unwrap(),
            why: "café path fails\nReproduced the defect.".into(),
        },
        HistoryEvent::FindingNotConfirmed {
            pr,
            number,
            pass: 1,
            by: "inv-third".parse().unwrap(),
            why: "café boundary is guarded\nChecked every caller.".into(),
        },
    ] {
        pull_request_event(&mut page, at, &event);
    }
    assert_eq!(page.text, include_str!("opinions.golden"));
}
