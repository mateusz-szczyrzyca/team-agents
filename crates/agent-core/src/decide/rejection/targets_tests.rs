//! What a `RETURNED:` line names — an item or a part of a split item — and which run of the member a hand-over was made
//! after.

use super::*;
use crate::state::RunEnd;
use crate::state::fixtures::{at, member, normal, run};
use crate::state::split::PartKey;
use crate::status::RunOutcome;

/// The targets of `message` as `(item, part)` text, in order.
fn targets(message: &str) -> Vec<(String, Option<String>)> {
    returned_targets(message)
        .iter()
        .map(|target| (target.item().to_string(), target.part().map(|part| part.to_string())))
        .collect()
}

#[test]
fn a_line_names_an_item_or_a_part_of_one() {
    let table = [
        ("RETURNED: 1.1\nSTATUS: SLEEPING", vec![("1.1", None)]),
        ("RETURNED: 1.1a\nSTATUS: SLEEPING", vec![("1.1", Some("a"))]),
        ("**RETURNED: 2.3c**\nSTATUS: DONE", vec![("2.3", Some("c"))]),
        (
            "RETURNED: 1.1a\nRETURNED: 1.1b\nRETURNED: 1.2",
            vec![("1.1", Some("a")), ("1.1", Some("b")), ("1.2", None)],
        ),
        // The same target twice is one, a part and its item are two.
        (
            "RETURNED: 1.1a\nRETURNED: 1.1a\nRETURNED: 1.1",
            vec![("1.1", Some("a")), ("1.1", None)],
        ),
        // Only the first word after the colon is the target.
        ("RETURNED: 1.1a because the tests are missing", vec![("1.1", Some("a"))]),
    ];
    for (message, expected) in table {
        let expected: Vec<(String, Option<String>)> = expected
            .into_iter()
            .map(|(item, part)| (item.to_owned(), part.map(str::to_owned)))
            .collect();
        assert_eq!(targets(message), expected, "{message:?}");
    }
}

#[test]
fn a_line_that_names_neither_an_item_nor_a_part_is_skipped() {
    for message in [
        "RETURNED: 1.1ab",
        "RETURNED: 1.1A",
        "RETURNED: a1.1",
        "RETURNED: 1.1-a",
        "RETURNED: none",
        "RETURNED:",
        "The work of 1.1a is not accepted.",
    ] {
        assert_eq!(targets(message), Vec::new(), "{message:?}");
    }
}

#[test]
fn a_target_knows_its_part_key() {
    let found = returned_targets("RETURNED: 1.1b");

    assert_eq!(found.len(), 1);
    assert_eq!(found[0].part(), PartKey::at_position(1));
}

/// A member whose run `n` starts at second `10 * n` and ends with `ends[n - 1]`.
fn runs(ends: &[RunEnd]) -> Vec<crate::state::Run> {
    let mut task = member(1, normal("claude-1"));
    for (index, end) in ends.iter().enumerate() {
        let n = u32::try_from(index).unwrap() + 1;
        task.begin_run(run(n), at(10 * n), None).unwrap();
        task.end_run(at(10 * n + 1), end.clone()).unwrap();
    }
    task.runs().to_vec()
}

#[test]
fn the_hand_over_was_made_after_the_last_run_that_ended_done() {
    let done = RunEnd::Reported {
        outcome: RunOutcome::Done,
    };
    let asked = RunEnd::Reported {
        outcome: RunOutcome::Questions,
    };
    let table = [
        (vec![], None, "no run"),
        (vec![asked.clone()], None, "no run ended DONE"),
        (vec![done.clone()], Some(1), "the only run"),
        (vec![done.clone(), done.clone()], Some(2), "the last of two"),
        (
            vec![done.clone(), asked, RunEnd::Stopped],
            Some(1),
            "the DONE one before later ones",
        ),
    ];
    for (ends, expected, what) in table {
        let runs = runs(&ends);
        assert_eq!(handed_over_run(&runs), expected.map(run).as_ref(), "{what}");
    }
}
