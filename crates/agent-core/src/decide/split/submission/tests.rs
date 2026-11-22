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
        (&["1.1"], "SPLIT: 7.7\nSPLIT: 1.1", already("1.1")),
        (&[], "SPLIT: item one\nSPLIT: 9.9", unknown("9.9")),
    ];

    for (split, message, want) in cases {
        assert_eq!(judged(split, Some(message)), want, "{message:?}");
    }
}

// Rule: only a line that has the form of a proposal — `SPLIT:` and an item number such as `1.1` — is a submission; a word
// that merely starts with `SPLIT:` in a report (`SPLIT: none`, `SPLIT: 1.1a`) asks nothing and is left alone.
#[test]
fn a_line_that_has_not_the_form_of_a_proposal_submits_nothing() {
    let cases = [
        "SPLIT: none",
        "SPLIT: 1.1a",
        "SPLIT:",
        "SPLIT: 1.1 and more",
        "SPLIT: item one",
        "SPLIT: 1.",
        "SPLIT: .1",
        "SPLIT: 1.1.1",
        "SPLIT: -1.1",
    ];

    for line in cases {
        assert_eq!(
            judged(&[], Some(&format!("{line}\nSTATUS: DONE"))),
            Submission::NotSubmitted,
            "{line}"
        );
        assert_eq!(judged(&["1.1"], Some(line)), Submission::NotSubmitted, "{line}");
    }
}

// Rule: an item that has the form of one, and that the team does not have, is refused by its name.
#[test]
fn a_well_formed_item_the_team_does_not_have_is_refused_by_name() {
    assert_eq!(judged(&[], Some("SPLIT: 9.9\nSTATUS: SLEEPING")), unknown("9.9"));
    assert_eq!(judged(&["1.1"], Some("SPLIT: 12.34")), unknown("12.34"));
}
