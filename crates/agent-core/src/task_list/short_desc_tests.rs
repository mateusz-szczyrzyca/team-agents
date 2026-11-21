//! The `Short desc:` line of a task list's header: what it accepts, what it refuses and where it is only text.
use super::*;
use crate::task_title::MAX_SHORT_DESC_CHARS;

/// The error of a text that must be refused.
fn refused(text: &str) -> TaskListError {
    TaskList::parse(text).unwrap_err()
}

#[test]
fn a_header_line_gives_the_short_description_and_is_not_part_of_the_brief() {
    let list = TaskList::parse("Short desc: Fix the typo\n\nThe README has a typo.\n").unwrap();

    assert_eq!(list.short_desc(), Some("Fix the typo"));
    assert_eq!(list.brief(), "The README has a typo.");
}

#[test]
fn a_list_without_the_line_has_no_explicit_short_description() {
    let list = TaskList::parse("The README has a typo.\n").unwrap();

    assert_eq!(list.short_desc(), None);
    assert_eq!(list.brief(), "The README has a typo.");
}

#[test]
fn the_line_is_read_beside_the_coordinator_and_allow_lines_in_either_order() {
    let texts = [
        "Coordinator: inv-opus\nShort desc: Fix it\nAllow: Cargo.toml\nBrief.\n",
        "Coordinator: inv-opus\nAllow: Cargo.toml\nShort desc: Fix it\nBrief.\n",
        "Short desc: Fix it\nAllow: Cargo.toml\nBrief.\n",
        "Allow: Cargo.toml\n\nShort desc: Fix it\n\nBrief.\n",
    ];

    for text in texts {
        let list = TaskList::parse(text).unwrap();
        assert_eq!(list.short_desc(), Some("Fix it"), "{text:?}");
        assert_eq!(list.brief(), "Brief.", "{text:?}");
        assert_eq!(list.allowances().len(), 1, "{text:?}");
    }
}

#[test]
fn the_text_after_the_label_is_trimmed() {
    let list = TaskList::parse("Short desc:    Fix it   \nBrief.\n").unwrap();

    assert_eq!(list.short_desc(), Some("Fix it"));
}

#[test]
fn seventy_characters_are_accepted_whatever_their_width() {
    for ch in ["a", "ż", "日"] {
        let longest = ch.repeat(MAX_SHORT_DESC_CHARS);

        let list = TaskList::parse(&format!("Short desc: {longest}\nBrief.")).unwrap();

        assert_eq!(list.short_desc(), Some(longest.as_str()), "{ch}");
    }
}

#[test]
fn seventy_one_characters_are_refused_with_their_number_and_the_line() {
    for ch in ["a", "ż", "日"] {
        let too_long = ch.repeat(MAX_SHORT_DESC_CHARS + 1);

        let error = refused(&format!("Coordinator: inv-opus\nShort desc: {too_long}\nBrief."));

        assert!(
            matches!(&error, TaskListError::ShortDescTooLong { line, len, max }
                if line.get() == 2 && *len == MAX_SHORT_DESC_CHARS + 1 && *max == MAX_SHORT_DESC_CHARS),
            "{ch}: {error:?}"
        );
    }
}

#[test]
fn an_empty_line_is_refused_with_its_number() {
    for (text, expected_line) in [
        ("Short desc:\nBrief.", 1),
        ("Short desc:    \nBrief.", 1),
        ("Coordinator: inv-opus\nShort desc: \t\nBrief.", 2),
        ("Allow: Cargo.toml\n\nShort desc:\nBrief.", 3),
    ] {
        let error = refused(text);

        assert!(
            matches!(&error, TaskListError::ShortDescEmpty { line } if line.get() == expected_line),
            "{text:?}: {error:?}"
        );
    }
}

#[test]
fn a_second_line_is_refused_with_both_line_numbers() {
    let error = refused("Short desc: One\nAllow: Cargo.toml\nShort desc: Two\nBrief.");

    assert!(
        matches!(&error, TaskListError::ShortDescTwice { line, first_line } if line.get() == 3 && first_line.get() == 1),
        "{error:?}"
    );
}

#[test]
fn the_messages_say_what_to_do() {
    let texts = [
        ("Short desc:\nBrief.", "give the line some text"),
        (
            &format!("Short desc: {}\nBrief.", "a".repeat(71)),
            "at most 70 characters",
        ),
        ("Short desc: A\nShort desc: B\nBrief.", "keep one"),
    ];

    for (text, advice) in texts {
        let message = refused(text).to_string();
        assert!(message.starts_with("line "), "{message}");
        assert!(message.contains(advice), "{message}");
    }
}

#[test]
fn the_words_further_down_in_the_brief_are_text() {
    let text = "Brief first.\nShort desc: not a header\n\n1. Feature\n[ ] 1.1 Item\n    Short desc: in an item.\n";
    let list = TaskList::parse(text).unwrap();

    assert_eq!(list.short_desc(), None);
    assert_eq!(list.brief(), "Brief first.\nShort desc: not a header");
}

#[test]
fn a_line_that_is_too_long_after_the_brief_began_is_still_text() {
    let long = "x".repeat(200);
    let list = TaskList::parse(&format!("Brief first.\nShort desc: {long}\n")).unwrap();

    assert_eq!(list.short_desc(), None);
    assert!(list.brief().ends_with(&long));
}

#[test]
fn a_list_made_only_of_header_lines_is_empty() {
    assert!(matches!(
        TaskList::parse("Short desc: Fix it\n"),
        Err(TaskListError::Empty)
    ));
}

#[test]
fn the_line_with_a_feature_list_reads_the_list_as_before() {
    let list = TaskList::parse("Short desc: Do it\nBrief.\n\n1. Feature\n[ ] 1.1 Item\n    Work.\n").unwrap();

    assert_eq!(list.short_desc(), Some("Do it"));
    assert_eq!(list.items().count(), 1);
}

#[test]
fn a_list_read_with_allow_lines_as_brief_keeps_the_line_as_brief_text() {
    let list = TaskList::parse_with("Short desc: Fix it\nBrief.", AllowLines::Brief).unwrap();

    assert_eq!(list.short_desc(), None);
    assert_eq!(list.brief(), "Short desc: Fix it\nBrief.");
}

#[test]
fn the_derived_description_is_used_when_the_list_names_none_and_the_given_one_when_it_does() {
    let named = TaskList::parse("Short desc: Given one\nFix the typo. Then rest.").unwrap();
    let unnamed = TaskList::parse("Fix the typo. Then rest.").unwrap();

    assert_eq!(named.display_short_desc(), "Given one");
    assert_eq!(unnamed.display_short_desc(), "Fix the typo.");
}
