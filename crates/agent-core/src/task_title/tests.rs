//! The words that name a task: its derived description and the excerpt of its brief.
use super::*;

#[test]
fn the_derived_description_is_the_first_sentence_of_the_brief() {
    let cases = [
        ("Fix the typo.", "Fix the typo."),
        ("Fix the typo. Then stop.", "Fix the typo."),
        ("Fix the typo.\nThen stop.", "Fix the typo."),
        ("Fix the typo", "Fix the typo"),
        ("Is it flaky? Find out.", "Is it flaky?"),
        ("Stop now! Please.", "Stop now!"),
        ("Fix v1.2 of the tool. Then stop.", "Fix v1.2 of the tool."),
        ("  \n\n  Fix the typo.  \n", "Fix the typo."),
        ("\u{feff}Fix the typo.", "Fix the typo."),
        ("Żółć w powiadomieniach. Potem nic.", "Żółć w powiadomieniach."),
    ];

    for (brief, expected) in cases {
        assert_eq!(derived_short_desc(brief), expected, "{brief:?}");
    }
}

#[test]
fn a_sentence_wrapped_over_lines_is_one_sentence_up_to_its_end() {
    assert_eq!(
        derived_short_desc("Fix the flaky\ntest in foo. More."),
        "Fix the flaky test in foo."
    );
    assert_eq!(derived_short_desc("Fix the flaky\n\ntest in foo."), "Fix the flaky");
}

#[test]
fn a_heading_or_a_list_mark_is_not_part_of_the_description() {
    let cases = [
        ("# Fix the typo.\n\nMore.", "Fix the typo."),
        ("### Fix the typo\n", "Fix the typo"),
        ("- Fix the typo.\n- Then stop.", "Fix the typo."),
        ("* Fix the typo\n* Then stop.", "Fix the typo"),
        ("+ Fix the typo.", "Fix the typo."),
        ("1. Fix the typo.\n2. Then stop.", "Fix the typo."),
        ("12) Fix the typo.", "Fix the typo."),
        ("  - Fix the typo.", "Fix the typo."),
        ("-Fix the typo.", "-Fix the typo."),
        ("1.5 times faster. Yes.", "1.5 times faster."),
    ];

    for (brief, expected) in cases {
        assert_eq!(derived_short_desc(brief), expected, "{brief:?}");
    }
}

#[test]
fn a_brief_without_words_has_the_fixed_description() {
    for brief in ["", " ", "\n\t\n", "\u{feff}", "#", "## ", "- ", "1. \n"] {
        assert_eq!(derived_short_desc(brief), NO_BRIEF_DESC, "{brief:?}");
    }
}

#[test]
fn a_sentence_of_exactly_the_limit_is_kept_and_a_longer_one_is_cut_with_a_mark() {
    for filler in ["a", "ż"] {
        let exact = format!("{}.", filler.repeat(MAX_SHORT_DESC_CHARS - 1));
        let longer = format!("{}.", filler.repeat(MAX_SHORT_DESC_CHARS));

        let kept = derived_short_desc(&exact);
        let cut = derived_short_desc(&format!("{longer} Then more."));

        assert_eq!(kept, exact, "{filler}");
        assert_eq!(
            cut,
            format!("{}{CUT_MARK}", filler.repeat(MAX_SHORT_DESC_CHARS - 1)),
            "{filler}"
        );
        assert_eq!(cut.chars().count(), MAX_SHORT_DESC_CHARS, "{filler}");
    }
}

#[test]
fn a_cut_description_does_not_end_with_a_space_before_the_mark() {
    let brief = format!("{} {}", "word".repeat(17), "next. Then more.");

    let derived = derived_short_desc(&brief);

    assert!(derived.ends_with(&format!("d{CUT_MARK}")), "{derived:?}");
    assert!(derived.chars().count() <= MAX_SHORT_DESC_CHARS);
}

#[test]
fn a_brief_within_the_limit_is_its_own_excerpt() {
    for brief in [
        "Fix it.",
        "One.\nTwo.",
        "No full stop at the end",
        "Żółć. Potem nic.",
        "  Padded.  \n",
    ] {
        assert_eq!(brief_excerpt(brief), brief.trim(), "{brief:?}");
    }
}

#[test]
fn a_blank_brief_has_no_excerpt() {
    for brief in ["", " \n\t\n", "\u{feff}"] {
        assert_eq!(brief_excerpt(brief), "", "{brief:?}");
    }
}

#[test]
fn the_excerpt_of_a_long_brief_ends_at_the_last_sentence_that_fits_and_says_it_was_cut() {
    let sentence = "Sentence text that goes on for a while.";
    let brief = format!("{} And then more.", [sentence; 12].join(" "));

    let excerpt = brief_excerpt(&brief);

    assert_eq!(excerpt, format!("{} {CUT_MARK}", [sentence; 9].join(" ")));
    assert!(excerpt.chars().count() <= MAX_EXCERPT_CHARS);
}

#[test]
fn the_excerpt_of_a_brief_whose_first_sentence_is_too_long_is_cut_inside_it() {
    for brief in [
        "word ".repeat(MAX_EXCERPT_CHARS),
        "x".repeat(MAX_EXCERPT_CHARS + 50),
        "Ż".repeat(MAX_EXCERPT_CHARS + 1),
    ] {
        let excerpt = brief_excerpt(&brief);

        assert_eq!(excerpt.chars().count(), MAX_EXCERPT_CHARS, "{excerpt:?}");
        assert!(excerpt.ends_with(CUT_MARK), "{excerpt:?}");
        assert!(!excerpt.trim_end_matches(CUT_MARK).ends_with(' '), "{excerpt:?}");
    }
}

#[test]
fn a_brief_of_exactly_the_limit_is_not_cut() {
    let brief = format!("{}.", "x".repeat(MAX_EXCERPT_CHARS - 1));

    assert_eq!(brief_excerpt(&brief), brief);
}

#[test]
fn a_full_stop_inside_a_number_or_a_name_does_not_end_the_excerpt() {
    let brief = format!("Raise it to v1.2.3 now. {}", "More text here. ".repeat(40));

    let excerpt = brief_excerpt(&brief);

    assert!(
        excerpt.starts_with("Raise it to v1.2.3 now. More text here."),
        "{excerpt:?}"
    );
    assert!(excerpt.ends_with(&format!("here. {CUT_MARK}")), "{excerpt:?}");
}

#[test]
fn a_brief_that_opens_with_a_short_desc_line_is_described_by_that_line_without_its_label() {
    let cases = [
        (
            "Short desc: Checks ask the human once\n\nThis is an implementation task.",
            "Checks ask the human once",
        ),
        ("Short desc: Fix it\nFix the typo.", "Fix it"),
        ("Short desc: Fix it", "Fix it"),
        ("\n  Short desc:   Fix it  \nFix the typo.", "Fix it"),
        ("\u{feff}Short desc: Fix it\nFix the typo.", "Fix it"),
        ("Short desc: One\nShort desc: Two\nBrief.", "One"),
        (
            "Short desc: Uses `agent log`. Twice.\nBrief.",
            "Uses `agent log`. Twice.",
        ),
    ];

    for (brief, expected) in cases {
        assert_eq!(derived_short_desc(brief), expected, "{brief:?}");
    }
}

#[test]
fn a_short_desc_line_without_text_is_skipped_and_the_brief_after_it_describes_the_task() {
    assert_eq!(derived_short_desc("Short desc:\nFix the typo. More."), "Fix the typo.");
    assert_eq!(derived_short_desc("Short desc:   \n\nFix the typo."), "Fix the typo.");
    assert_eq!(derived_short_desc("Short desc:"), NO_BRIEF_DESC);
}

#[test]
fn a_short_desc_line_longer_than_the_limit_is_cut_with_a_mark_and_keeps_no_label() {
    let brief = format!("Short desc: {}\nBrief.", "x".repeat(MAX_SHORT_DESC_CHARS + 1));

    let desc = derived_short_desc(&brief);

    assert_eq!(desc.chars().count(), MAX_SHORT_DESC_CHARS);
    assert!(desc.starts_with('x') && desc.ends_with(CUT_MARK), "{desc:?}");
}

#[test]
fn only_a_line_that_opens_the_brief_with_the_exact_label_names_the_task() {
    let cases = [
        ("Fix it.\n\nShort desc: later in the brief", "Fix it."),
        (
            "Short description of the change. More.",
            "Short description of the change.",
        ),
        ("short desc: fix it. More.", "short desc: fix it."),
    ];

    for (brief, expected) in cases {
        assert_eq!(derived_short_desc(brief), expected, "{brief:?}");
    }
}

#[test]
fn the_excerpt_of_a_brief_leaves_out_the_short_desc_line_that_opens_it() {
    assert_eq!(
        brief_excerpt("Short desc: Fix it\n\nFix the typo in the README. It has one."),
        "Fix the typo in the README. It has one."
    );
    assert_eq!(brief_excerpt("Short desc: Fix it"), "");
    assert_eq!(
        brief_excerpt("Fix it.\n\nShort desc: later in the brief"),
        "Fix it.\n\nShort desc: later in the brief"
    );
}
