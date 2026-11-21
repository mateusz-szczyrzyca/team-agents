//! The line `Short desc:` of a task list's header: what the task is, in one line of at most
//! [`MAX_SHORT_DESC_CHARS`] characters.

use std::num::NonZeroUsize;

use super::TaskListError;
use crate::task_list::TaskList;
use crate::task_title::MAX_SHORT_DESC_CHARS;

/// The text that starts the header line naming the task in words.
const SHORT_DESC_PREFIX: &str = "Short desc:";

/// A parsed `Short desc:` line.
struct ShortDescLine {
    /// The accepted description, as it is read; the stub keeps no text.
    text: (),
    /// Its length in characters.
    len: usize,
}

/// Reads the description after the label of a `Short desc:` line, the text after `SHORT_DESC_PREFIX` being `rest`.
fn short_desc_line(line: NonZeroUsize, rest: &str) -> Result<ShortDescLine, TaskListError> {
    let _ = (line, rest);
    Err(TaskListError::ShortDescEmpty { line })
}

#[cfg(test)]
mod tests {
    use std::num::NonZeroU16;

    use super::*;
    use crate::task_list::ShortDesc;

    fn line(n: usize) -> NonZeroUsize {
        NonZeroUsize::new(n).unwrap()
    }

    fn parsed(text: &str) -> TaskList {
        TaskList::parse(text).unwrap()
    }

    fn failure(text: &str) -> TaskListError {
        TaskList::parse(text).unwrap_err()
    }

    /// A description of exactly `chars` characters: ASCII `a`s, then `Ż`.
    fn of_chars(chars: usize) -> String {
        "a".repeat(chars - 1) + "Ż"
    }

    #[test]
    fn a_header_line_names_the_task_without_being_part_of_the_brief() {
        let list = parsed("Short desc: Notifications name the task\n\nBuild the thing.\n");

        assert_eq!(
            list.short_desc().map(ShortDesc::as_str),
            Some("Notifications name the task")
        );
        assert_eq!(list.brief(), "Build the thing.");
    }

    #[test]
    fn a_list_without_the_line_has_no_description() {
        let list = parsed("Build the thing.\n\nShort desc: later is brief.\n");

        assert_eq!(list.short_desc(), None);
        assert_eq!(list.brief(), "Build the thing.\n\nShort desc: later is brief.");
    }

    #[test]
    fn the_description_is_read_from_every_header_order() {
        let with_both =
            parsed("Allow: Cargo.toml\nShort desc: Names the task\n\nBuild the thing.\n\n1. Feature\n[ ] 1.1 Item\n");
        let other_order =
            parsed("Allow: Cargo.toml\nCoordinator: inv-opus\nShort desc:   Names the task  \n\nBuild the thing.\n");
        let first = parsed("Short desc: Names the task\nAllow: Cargo.toml\n\nBuild the thing.\n");

        assert_eq!(with_both.short_desc().map(ShortDesc::as_str), Some("Names the task"));
        assert_eq!(with_both.allowances().len(), 1);
        assert_eq!(with_both.brief(), "Build the thing.");
        assert_eq!(with_both.features()[0].number, NonZeroU16::new(1).unwrap());
        assert_eq!(other_order.short_desc().map(ShortDesc::as_str), Some("Names the task"));
        assert_eq!(
            other_order.coordinator().map(crate::ids::AgentName::as_str),
            Some("inv-opus")
        );
        assert_eq!(first.short_desc().map(ShortDesc::as_str), Some("Names the task"));
        assert_eq!(first.allowances().len(), 1);
        assert_eq!(
            failure("Short desc: one\n\nAllow: Cargo.toml\nShort desc: two\n"),
            TaskListError::ShortDescTwice { line: line(4) }
        );
    }

    #[test]
    fn a_description_of_seventy_characters_is_read_and_seventy_one_is_refused() {
        for (chars, accepted) in [
            (1, true),
            (MAX_SHORT_DESC_CHARS, true),
            (MAX_SHORT_DESC_CHARS + 1, false),
        ] {
            let text = of_chars(chars);
            let list = format!("Short desc: {text}\n\nBrief\n");
            if accepted {
                assert_eq!(
                    parsed(&list).short_desc().map(ShortDesc::as_str),
                    Some(text.as_str()),
                    "{chars}"
                );
            } else {
                assert_eq!(
                    failure(&list),
                    TaskListError::ShortDescTooLong {
                        line: line(1),
                        len: MAX_SHORT_DESC_CHARS + 1,
                        max: MAX_SHORT_DESC_CHARS
                    },
                    "{chars}"
                );
            }
        }
    }

    #[test]
    fn an_empty_description_is_refused_with_its_line() {
        for (text, at) in [
            ("Short desc:\n\nBrief\n", 1),
            ("Short desc:   \n\nBrief\n", 1),
            ("Allow: Cargo.toml\n\n\nShort desc:\t\n\nBrief\n", 4),
        ] {
            assert_eq!(
                failure(text),
                TaskListError::ShortDescEmpty { line: line(at) },
                "{text:?}"
            );
        }
    }

    #[test]
    fn a_second_description_is_refused_with_the_line_of_the_second() {
        for text in [
            "Short desc: one\nShort desc: two\n\nBrief\n",
            "Short desc: one\n\nAllow: Cargo.toml\nShort desc: two\n\nBrief\n",
        ] {
            assert_eq!(
                failure(text),
                TaskListError::ShortDescTwice { line: line(2) },
                "{text:?}"
            );
        }
        assert_eq!(
            failure("Short desc: one\nAllow: Cargo.toml\n\nShort desc: two\n\nBrief\n"),
            TaskListError::ShortDescTwice { line: line(4) },
            "the second line counts from the start of the file"
        );
    }

    #[test]
    fn a_description_after_the_brief_began_is_text_of_the_brief() {
        for text in [
            "Brief\nShort desc: late\n",
            "Brief\n\nShort desc: late\n",
            "  Short desc: indented\n",
        ] {
            let list = parsed(text);
            assert_eq!(list.short_desc(), None, "{text:?}");
            assert!(list.brief().contains("Short desc:"), "{text:?}");
        }
    }

    #[test]
    fn a_list_of_the_description_and_header_lines_alone_is_empty() {
        for text in ["Short desc: only\n", "Coordinator: inv-opus\nShort desc: only\n\n"] {
            assert_eq!(failure(text), TaskListError::Empty, "{text:?}");
        }
    }

    #[test]
    fn the_description_may_hold_non_ascii_text_and_is_counted_in_characters() {
        let list = parsed("Short desc: Żółć w powiadomieniach — nazwa zadania\n\nBrief\n");
        let at_the_limit = format!("Short desc: {}\n\nBrief\n", "Ż".repeat(MAX_SHORT_DESC_CHARS));

        assert_eq!(
            list.short_desc().map(ShortDesc::as_str),
            Some("Żółć w powiadomieniach — nazwa zadania")
        );
        assert_eq!(
            parsed(&at_the_limit).short_desc().map(ShortDesc::as_str).map(str::len),
            Some(2 * MAX_SHORT_DESC_CHARS)
        );
    }

    #[test]
    fn the_description_may_hold_placeholder_and_markdown_text() {
        let list = parsed("Short desc: {{task}} and `code`\n\nBrief\n");

        assert_eq!(list.short_desc().map(ShortDesc::as_str), Some("{{task}} and `code`"));
    }

    #[test]
    fn the_line_matches_the_label_of_the_header() {
        for text in [
            "Short desc: Notes\n\nBrief\n",
            "Short desc:Notes\n\nBrief\n",
            "Short desc:   Notes   \n\nBrief\n",
        ] {
            assert_eq!(
                short_desc_line(line(1), text.strip_prefix(SHORT_DESC_PREFIX).unwrap_or(text))
                    .map(|read| (read.text, read.len)),
                Ok(((), 5usize)),
                "{text:?}"
            );
        }
        assert_eq!(
            short_desc_line(line(3), "  ").err(),
            Some(TaskListError::ShortDescEmpty { line: line(3) })
        );
    }
}
