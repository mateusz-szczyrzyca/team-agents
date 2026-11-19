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
        HistoryEvent::SecondOpinionEnded {
            attempt: std::num::NonZeroU32::MIN,
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

#[test]
fn failed_opinion_history_distinguishes_replacement_from_exhaustion() {
    for (attempt, action) in [(1, "replaced"), (3, "exhausted")] {
        let event = serde_json::from_value::<HistoryEvent>(serde_json::json!({
            "event": "second_opinion_ended", "pr": 1, "number": 2, "pass": 1,
            "by": "inv-first", "why": "the run failed", "attempt": attempt
        }));
        assert_eq!(
            event
                .as_ref()
                .map(|event| event.kind().to_string())
                .map_err(ToString::to_string),
            Ok("second_opinion_ended".to_owned())
        );
        let event = event.unwrap();
        let mut page = Page::default();
        pull_request_event(&mut page, "2026-11-11T19:50:00Z".parse().unwrap(), &event);
        assert_eq!(
            page.text,
            format!(
                "2026-11-11T19:50:00Z  second opinion {action}: pull request #1 finding 2, pass 1, by inv-first; the run failed (second_opinion_ended)\n  ----- evidence (14 bytes) -----\nthe run failed\n  ----- end of evidence -----\n"
            )
        );
        assert_eq!(event.identities(), ["inv-first"]);
        assert_eq!(event.key(), Some("pr 1 finding 2 pass 1 by inv-first".to_owned()));
    }
}
