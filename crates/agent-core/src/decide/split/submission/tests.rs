//! The `SPLIT:` lines of a message: the proposal that is taken, and every refusal.

use super::*;

fn item(text: &str) -> ItemId {
    text.parse().unwrap()
}

fn judged(split: &[&str], message: Option<&str>) -> Submission {
    let items = [item("1.1"), item("1.2"), item("2.1")];
    let split: BTreeSet<ItemId> = split.iter().map(|text| item(text)).collect();
    judge_submission(&items, &split, message)
}

fn unknown(text: &str) -> Submission {
    Submission::Refused(Refusal::UnknownItem { text: text.to_owned() })
}

fn already(text: &str) -> Submission {
    Submission::Refused(Refusal::AlreadySplit { item: item(text) })
}

// Rule: a message without a `SPLIT:` line submits nothing.
#[test]
fn a_message_without_a_split_line_submits_nothing() {
    let cases = [
        (None, "no message"),
        (Some(""), "an empty message"),
        (Some("Done.\nSTATUS: SLEEPING"), "a message with no line"),
        (
            Some("I may split 1.1 later.\nSTATUS: DONE"),
            "a message that only mentions a split",
        ),
    ];

    for (message, name) in cases {
        assert_eq!(judged(&[], message), Submission::NotSubmitted, "{name}");
    }
}

// Rule: a line that names an item that is not split is the proposal that is taken.
#[test]
fn a_line_naming_an_item_that_is_not_split_is_taken() {
    let cases = [
        (&[][..], "Proposal.\nSPLIT: 1.1\nSTATUS: SLEEPING", item("1.1")),
        (&[], "  SPLIT: 1.2  \nSTATUS: SLEEPING", item("1.2")),
        (&["1.1"], "SPLIT: 1.1\nSPLIT: 2.1\nSTATUS: SLEEPING", item("2.1")),
        (&[], "SPLIT: 2.1\nSPLIT: 1.2", item("1.2")),
    ];

    for (split, message, want) in cases {
        assert_eq!(judged(split, Some(message)), Submission::Taken(want), "{message:?}");
    }
}

// Rule: a line the harness cannot take is refused with its reason: an item that is split already, an item the team does
// not have, a line with no item.
#[test]
fn a_line_the_harness_cannot_take_is_refused_with_its_reason() {
    let cases = [
        (&["1.1"][..], "SPLIT: 1.1\nSTATUS: SLEEPING", already("1.1")),
        (&["1.1", "1.2"], "SPLIT: 1.2\nSPLIT: 1.1", already("1.2")),
        (&[], "SPLIT: 9.9\nSTATUS: SLEEPING", unknown("9.9")),
        (&[], "SPLIT:\nSTATUS: SLEEPING", unknown("")),
        (&[], "SPLIT: 1.1 and more\n", unknown("1.1 and more")),
        (&["1.1"], "SPLIT: 7.7\nSPLIT: 1.1", already("1.1")),
        (&[], "SPLIT: item one\nSPLIT: 9.9", unknown("item one")),
    ];

    for (split, message, want) in cases {
        assert_eq!(judged(split, Some(message)), want, "{message:?}");
    }
}
