//! What a notification says of a task in words, and of its brief.
//!
//! The table of cases covers the first sentence of a brief (short, long, cut at its limit) and every kind of opening a
//! brief may have (a heading, a list item, an indented paragraph), and a description cut at its limit.

use super::{CUT_MARK, MAX_EXCERPT_CHARS, MAX_SHORT_DESC_CHARS, NO_BRIEF_DESC, brief_excerpt, derived_short_desc};

/// The byte order mark some editors put at the start of a file.
const BYTE_ORDER_MARK: char = '\u{feff}';

#[test]
fn a_short_first_sentence_is_the_description() {
    let cases = [
        ("Fix the flaky test in foo.", "Fix the flaky test in foo."),
        ("Fix the flaky test in foo.\nIt fails on CI.", "Fix the flaky test in foo."),
        ("# Fix the flaky test.\n\nMore.", "Fix the flaky test."),
        ("- Fix the flaky test.\n- Then stop.", "Fix the flaky test."),
        ("* Fix the flaky test\n* Then stop.", "Fix the flaky test"),
        ("1. Fix the flaky test.\n2. Then stop.", "Fix the flaky test."),
        ("1) Fix the flaky test.\n2) Then stop.", "Fix the flaky test."),
        ("  Fix the flaky test  ", "Fix the flaky test"),
        ("\n\nFix the flaky test.", "Fix the flaky test."),
        (
            "Fix the flaky test in `foo` at v1.2 (not 1.3)!\nNext.",
            "Fix the flaky test in `foo` at v1.2 (not 1.3)!",
        ),
        ("Żółć w powiadomieniach. Potem nic.", "Żółć w powiadomieniach."),
        ("One sentence? Yes! So it is.", "One sentence? Yes! So it is."),
    ];

    for (brief, expected) in cases {
        assert_eq!(derived_short_desc(brief), expected, "{brief:?}");
    }
}

#[test]
fn a_sentence_that_fits_the_limit_exactly_is_kept_whole() {
    let sentence = format!("{}.", "a".repeat(MAX_SHORT_DESC_CHARS - 1));

    assert_eq!(derived_short_desc(&sentence), sentence);
    assert_eq!(derived_short_desc(&sentence).chars().count(), MAX_SHORT_DESC_CHARS);
}

#[test]
fn a_long_first_sentence_is_cut_at_the_limit_with_a_mark() {
    let sentence = format!("{}. Then stop.", "a".repeat(MAX_SHORT_DESC_CHARS));
    let cutting = format!("{}.", "b".repeat(MAX_SHORT_DESC_CHARS - 1));
    let words = format!("{} words that go on.", "c".repeat(MAX_SHORT_DESC_CHARS - 12));

    let derived = derived_short_desc(&sentence);
    let cut = derived_short_desc(&cutting);
    let at_word = derived_short_desc(&words);

    assert_eq!(derived.chars().count(), MAX_SHORT_DESC_CHARS);
    assert_eq!(derived, format!("{}{CUT_MARK}", "a".repeat(MAX_SHORT_DESC_CHARS - 1)));
    assert_eq!(cut.chars().count(), MAX_SHORT_DESC_CHARS);
    assert_eq!(cut, format!("{}{CUT_MARK}", "b".repeat(MAX_SHORT_DESC_CHARS - 1)));
    assert_eq!(at_word.chars().count(), MAX_SHORT_DESC_CHARS);
    assert_eq!(at_word, format!("{} {CUT_MARK}", "c".repeat(MAX_SHORT_DESC_CHARS - 12)));
}

#[test]
fn a_brief_that_opens_with_a_heading_or_a_list_falls_back_to_its_first_word() {
    for (brief, expected) in [("## ", "##"), ("- \n- x", "-"), ("1. \n", "1."), ("   \n#", "#")] {
        assert_eq!(derived_short_desc(brief), expected, "{brief:?}");
    }
    let cases = [
        ("## A heading", "A heading"),
        ("- an item", "an item"),
        ("+ an item", "an item"),
        ("-an item, not a list", "-an item, not a list"),
        ("1. an item", "an item"),
        ("12) an item", "an item"),
        ("1.5 is a number, not a list", "1.5 is a number, not a list"),
        (&format!("{BYTE_ORDER_MARK}A brief."), "A brief."),
    ];
    for (brief, expected) in cases {
        assert_eq!(derived_short_desc(brief), expected, "{brief:?}");
    }
}

#[test]
fn a_brief_without_words_gives_the_text_of_a_task_without_a_description() {
    for brief in ["", " ", "\n\t\n", "\u{feff}", "\u{feff}\n"] {
        assert_eq!(derived_short_desc(brief), NO_BRIEF_DESC, "{brief:?}");
    }
}

#[test]
fn an_excerpt_shorter_than_the_limit_is_the_brief_trimmed() {
    for brief in ["Fix it.", "One.\nTwo.", "No full stop at the end", "Żółć. Potem nic."] {
        assert_eq!(brief_excerpt(brief), brief.trim(), "{brief:?}");
    }
}

#[test]
fn an_excerpt_of_a_long_brief_stops_after_the_last_sentence_that_fits() {
    let sentence = "Sentence text that goes on for a while.";
    let brief = format!("{} And then more.", sentence.repeat(12));

    let excerpt = brief_excerpt(&brief);

    assert!(
        excerpt.chars().count() <= MAX_EXCERPT_CHARS,
        "{}",
        excerpt.chars().count()
    );
    assert_eq!(excerpt, sentence.repeat(10));
    assert!(brief.starts_with(&excerpt));
}

#[test]
fn an_excerpt_of_a_brief_cut_inside_its_first_sentence_is_marked() {
    let brief = "word ".repeat(MAX_EXCERPT_CHARS);
    let unbroken = "x".repeat(MAX_EXCERPT_CHARS + 50);

    let excerpt = brief_excerpt(&brief);
    let cut = brief_excerpt(&unbroken);

    assert!(
        excerpt.chars().count() <= MAX_EXCERPT_CHARS,
        "{}",
        excerpt.chars().count()
    );
    assert_eq!(excerpt, format!("{} {CUT_MARK}", "word ".repeat(79)), "{excerpt:?}");
    // Cut inside the brief's first sentence, which has no end to stop at.
    assert_eq!(cut.chars().count(), MAX_EXCERPT_CHARS);
    assert!(cut.ends_with(CUT_MARK), "{cut:?}");
}

#[test]
fn an_excerpt_that_ends_with_a_sentence_on_the_limit_is_not_marked() {
    let padding = "x".repeat(MAX_EXCERPT_CHARS - 5);
    let brief = format!("{padding} end.");

    let excerpt = brief_excerpt(&brief);

    assert_eq!(excerpt.chars().count(), MAX_EXCERPT_CHARS - 5);
    assert!(excerpt.ends_with("end."), "{excerpt:?}");
}

#[test]
fn an_excerpt_counts_its_characters_not_its_bytes() {
    let brief = format!("{}. Then more.", "Ż".repeat(MAX_EXCERPT_CHARS));

    let excerpt = brief_excerpt(&brief);

    assert_eq!(excerpt.chars().count(), MAX_EXCERPT_CHARS);
    assert!(excerpt.ends_with(CUT_MARK), "{excerpt:?}");
}

#[test]
fn a_blank_brief_gives_no_excerpt() {
    assert_eq!(brief_excerpt(""), "");
    assert_eq!(brief_excerpt(" \n\t\n"), "");
}
