//! The numbering of an item's requirements and the text of their numbers.

use super::*;
use crate::task_list::TaskList;

/// The only item of the list `text` makes, under a feature.
fn item_of(body: &str) -> Item {
    let text = format!("1. Feature\n[ ] 1.1 The title\n{body}");
    TaskList::parse(&text).unwrap().items().next().unwrap().clone()
}

/// The requirements of `item` as `(number, text)` pairs.
fn pairs(item: &Item) -> Vec<(String, String)> {
    item.requirements()
        .into_iter()
        .map(|requirement| (requirement.id.to_string(), requirement.text))
        .collect()
}

/// `(number, text)` pairs written as literals.
fn expected(rows: &[(&str, &str)]) -> Vec<(String, String)> {
    rows.iter()
        .map(|(id, text)| ((*id).to_owned(), (*text).to_owned()))
        .collect()
}

#[test]
fn three_bullets_and_two_checks_are_r1_to_r3_and_k1_k2() {
    let item = item_of(
        "    Expected result:\n    - first\n    - second\n    - third\n    Checks:\n    - K1. one\n    - K2. two\n",
    );
    assert_eq!(
        pairs(&item),
        expected(&[
            ("R1", "first"),
            ("R2", "second"),
            ("R3", "third"),
            ("K1", "one"),
            ("K2", "two"),
        ])
    );
}

#[test]
fn nested_bullets_and_continuation_lines_stay_inside_their_requirement() {
    let item = item_of("    Expected result:\n    - first\n      - nested\n      more of it\n    - second\n");
    assert_eq!(
        pairs(&item),
        expected(&[("R1", "first\n  - nested\n  more of it"), ("R2", "second")])
    );
}

#[test]
fn a_check_line_without_a_number_gets_the_next_free_one() {
    let item = item_of("    Checks:\n    - K1. one\n    - two\n    - K3: three\n    - four\n");
    assert_eq!(
        pairs(&item),
        expected(&[
            ("R1", "The title"),
            ("K1", "one"),
            ("K2", "two"),
            ("K3", "three"),
            ("K4", "four")
        ])
    );
}

#[test]
fn an_unnumbered_check_does_not_take_a_number_a_later_line_names() {
    let item = item_of("    Checks:\n    - one\n    - K1. two\n");
    assert_eq!(
        pairs(&item),
        expected(&[("R1", "The title"), ("K2", "one"), ("K1", "two")])
    );
}

#[test]
fn a_number_named_twice_is_kept_by_the_first_line_only() {
    let item = item_of("    Checks:\n    - K1. one\n    - K1. two\n");
    assert_eq!(
        pairs(&item),
        expected(&[("R1", "The title"), ("K1", "one"), ("K2", "two")])
    );
}

#[test]
fn checks_that_are_not_bullets_are_one_line_each() {
    let item = item_of("    Checks:\n    run it\n    K2. run that\n");
    assert_eq!(
        pairs(&item),
        expected(&[("R1", "The title"), ("K1", "run it"), ("K2", "run that")])
    );
}

#[test]
fn an_item_with_no_parts_has_its_title_as_r1() {
    let item = item_of("    Only a description.\n");
    assert_eq!(pairs(&item), expected(&[("R1", "The title")]));
}

#[test]
fn an_expected_result_with_no_bullets_is_one_requirement() {
    let item = item_of("    Expected result:\n    It works.\n    All of it.\n    Checks:\n    - K1. run\n");
    assert_eq!(
        pairs(&item),
        expected(&[("R1", "It works.\nAll of it."), ("K1", "run")])
    );
}

#[test]
fn an_item_with_checks_only_still_has_its_title_as_r1() {
    let item = item_of("    Checks:\n    - K1. run\n");
    assert_eq!(pairs(&item), expected(&[("R1", "The title"), ("K1", "run")]));
}

#[test]
fn a_number_is_read_from_its_text_and_written_back() {
    for text in ["R1", "R12", "K2", "K65535"] {
        let id: RequirementId = text.parse().unwrap();
        assert_eq!(id.to_string(), text);
    }
    let id: RequirementId = "K2".parse().unwrap();
    assert_eq!((id.kind(), id.number().get()), (RequirementKind::Check, 2));
}

#[test]
fn a_text_that_is_not_a_number_of_a_requirement_is_refused() {
    for text in [
        "", "R", "R0", "R01", "r1", "X1", "R1 ", " R1", "R-1", "R4294967296", "RR1", "1", "R1.2",
    ] {
        assert_eq!(
            text.parse::<RequirementId>(),
            Err(InvalidRequirementId { value: text.to_owned() }),
            "{text:?}"
        );
    }
}

#[test]
fn the_expected_result_is_written_with_its_numbers_and_keeps_its_other_lines() {
    let item = item_of("    Expected result:\n    Intro line.\n    - first\n      - nested\n    - second\n");
    assert_eq!(
        item.expected_numbered().as_deref(),
        Some("Intro line.\nR1. first\n  - nested\nR2. second")
    );
}

#[test]
fn an_expected_result_with_no_bullets_is_numbered_as_one_requirement() {
    let item = item_of("    Expected result:\n    It works.\n    All of it.\n");
    assert_eq!(item.expected_numbered().as_deref(), Some("R1. It works.\nAll of it."));
}

#[test]
fn the_checks_are_written_with_their_numbers_whether_they_have_them_or_not() {
    let item = item_of("    Checks:\n    - K1. one\n    - two\n    - K5: three\n");
    assert_eq!(item.checks_numbered().as_deref(), Some("K1. one\nK2. two\nK5. three"));
    let lines = item_of("    Checks:\n    run it\n    K2. run that\n");
    assert_eq!(lines.checks_numbered().as_deref(), Some("K1. run it\nK2. run that"));
}

#[test]
fn an_item_without_the_parts_has_nothing_numbered() {
    let item = item_of("    Only a description.\n");
    assert_eq!((item.expected_numbered(), item.checks_numbered()), (None, None));
}

/// The number of lines one more than a 16-bit number can hold.
const PAST_SIXTEEN_BITS: usize = 65_536;

// Rule: no bullet of an expected result is dropped, however many an item has.
#[test]
fn every_bullet_of_the_expected_result_is_a_requirement_past_65535() {
    let body = format!("    Expected result:\n{}", "    - x\n".repeat(PAST_SIXTEEN_BITS));
    let item = item_of(&body);

    let requirements = item.requirements();

    assert_eq!(requirements.len(), PAST_SIXTEEN_BITS);
    assert_eq!(
        requirements
            .last()
            .map(|requirement| requirement.id.to_string())
            .as_deref(),
        Some("R65536")
    );
    let numbered = item.expected_numbered().unwrap();
    assert_eq!(numbered.lines().count(), PAST_SIXTEEN_BITS);
    assert!(numbered.ends_with("R65536. x"), "{}", &numbered[numbered.len() - 20..]);
}

// Rule: no check line is dropped either, and the unnumbered ones go on from the last number.
#[test]
fn every_check_line_is_a_requirement_past_65535() {
    let body = format!("    Checks:\n{}", "    - x\n".repeat(PAST_SIXTEEN_BITS));
    let item = item_of(&body);

    let requirements = item.requirements();

    let checks: Vec<String> = requirements
        .iter()
        .filter(|requirement| requirement.id.kind() == RequirementKind::Check)
        .map(|requirement| requirement.id.to_string())
        .collect();
    assert_eq!(checks.len(), PAST_SIXTEEN_BITS);
    assert_eq!(checks.last().map(String::as_str), Some("K65536"));
}
