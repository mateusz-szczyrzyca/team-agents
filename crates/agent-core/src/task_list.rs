//! The task list: the text a task is started with, parsed (see `docs/task-list.md`).
//!
//! A task list is an optional `Coordinator: NAME` line, a free-text brief, and features with items:
//!
//! ```text
//! Coordinator: inv-opus
//!
//! The brief every member reads.
//!
//! 1. A feature
//! Depends on: none
//! [ ] 1.1 An item
//!     What to do, in indented lines.
//!     Expected result:
//!     - what the reviewers check the pull request against
//!     Checks:
//!     - what anyone can run to see that it works
//! ```
//!
//! [`TaskList::parse`] reads the text of an inline task list and of a `-f` file alike — it takes text, so the two
//! cannot differ — and every fault is a [`TaskListError`] with the number of the line it is on.

use std::fmt;
use std::num::{NonZeroU16, NonZeroUsize};
use std::str::FromStr;

use crate::ids::{AgentName, InvalidItemId, InvalidName, ItemId};

/// Largest task list accepted, in bytes: a bound on untrusted input, far above any real list.
pub const MAX_TASK_LIST_BYTES: usize = 1 << 20;

/// Whether an item is marked as done in the text: `[ ]` or `[x]`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ItemMark {
    /// `[ ]`: the item is to be done.
    Open,
    /// `[x]` (or `[X]`): the author says the item is done.
    Done,
}

/// One item of a feature: a task of one branch and one pull request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item {
    /// The item's number, such as `2.3`: its feature's number and its own.
    pub id: ItemId,
    /// The rest of the item's line.
    pub title: String,
    /// The mark of the item's line.
    pub mark: ItemMark,
    /// The indented lines that tell what to do, without their indentation; empty when there are none.
    pub description: String,
    /// The part that follows `Expected result:`, the reviewers' standard; `None` when the item has no such part.
    pub expected_result: Option<String>,
    /// The part that follows `Checks:`, what anyone can run to see that the item works; `None` when there is none.
    pub checks: Option<String>,
}

impl Item {
    /// Whether the text marks the item as done.
    #[must_use]
    pub fn is_done(&self) -> bool {
        false
    }
}

/// A feature: a numbered group of items, started in order, that may wait for other features.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Feature {
    /// The feature's number, as in `3. Core decisions`.
    pub number: NonZeroU16,
    /// The rest of the feature's line.
    pub title: String,
    /// The features that must be merged before this one starts, in the order written; empty for `Depends on: none`
    /// and without a `Depends on:` line. Each is a feature that comes earlier in the text.
    pub depends_on: Vec<NonZeroU16>,
    /// The items, in the order written.
    pub items: Vec<Item>,
}

/// A parsed task list.
///
/// Made only by [`TaskList::parse`], so its parts are consistent: feature numbers are unique, every item is in the
/// feature its number says, item numbers are unique, and a feature depends only on features before it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskList {
    /// The name from a `Coordinator:` line, if the list starts with one.
    coordinator: Option<AgentName>,
    /// The text before the first feature, without the `Coordinator:` line and the blank lines around it.
    brief: String,
    /// The features, in the order written.
    features: Vec<Feature>,
}

/// An item part that a label starts: `Expected result:` or `Checks:`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ItemPart {
    /// The part after `Expected result:`.
    ExpectedResult,
    /// The part after `Checks:`.
    Checks,
}

impl fmt::Display for ItemPart {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::ExpectedResult => "Expected result:",
            Self::Checks => "Checks:",
        })
    }
}

/// What is wrong with an item line.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ItemProblem {
    /// The line starts with `[` but not with `[ ]`, `[x]` or `[X]`.
    #[error("it must start with [ ] or [x]")]
    BadMark,
    /// The mark is not followed by a space.
    #[error("the mark must be followed by a space")]
    NoSpaceAfterMark,
    /// The text after the mark is not an item number such as `2.3`.
    #[error("the item number is invalid")]
    BadId(#[source] InvalidItemId),
    /// There is no title after the item number.
    #[error("the title is missing")]
    MissingTitle,
}

/// Why a text is not a [`TaskList`]. Every variant but [`Empty`](Self::Empty) and [`TooLarge`](Self::TooLarge)
/// carries the number of the line it is on, counted from 1.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TaskListError {
    /// The text has nothing in it but blank lines, or only a `Coordinator:` line.
    #[error("the task list is empty")]
    Empty,
    /// The text is longer than [`MAX_TASK_LIST_BYTES`].
    #[error("the task list is {len} bytes: the limit is {max}")]
    TooLarge {
        /// The length of the text, in bytes.
        len: usize,
        /// The limit, in bytes.
        max: usize,
    },
    /// The `Coordinator:` line has no valid name.
    #[error("line {line}: invalid coordinator")]
    InvalidCoordinator {
        /// The line.
        line: NonZeroUsize,
        /// What is wrong with the name.
        #[source]
        source: InvalidName,
    },
    /// The number of a feature is not a number from 1 to 65535 without leading zeros.
    #[error("line {line}: invalid feature number {text:?}")]
    FeatureNumber {
        /// The line.
        line: NonZeroUsize,
        /// The text before the `.`.
        text: String,
    },
    /// A feature line has no title.
    #[error("line {line}: the feature has no title")]
    FeatureWithoutTitle {
        /// The line.
        line: NonZeroUsize,
    },
    /// A feature number was used before.
    #[error("line {line}: feature {number} is already defined on line {first_line}")]
    DuplicateFeature {
        /// The line.
        line: NonZeroUsize,
        /// The number.
        number: NonZeroU16,
        /// The line of the first feature with this number.
        first_line: NonZeroUsize,
    },
    /// A line that starts an item is not a valid item line.
    #[error("line {line}: malformed item line")]
    MalformedItem {
        /// The line.
        line: NonZeroUsize,
        /// What is wrong with it.
        #[source]
        problem: ItemProblem,
    },
    /// An item comes before the first feature.
    #[error("line {line}: the item is not in a feature")]
    ItemBeforeFeature {
        /// The line.
        line: NonZeroUsize,
    },
    /// An item's number names another feature than the one it is under.
    #[error("line {line}: item {id} is under feature {feature}")]
    ItemInOtherFeature {
        /// The line.
        line: NonZeroUsize,
        /// The item's number.
        id: ItemId,
        /// The feature the item is under.
        feature: NonZeroU16,
    },
    /// An item number was used before.
    #[error("line {line}: item {id} is already defined on line {first_line}")]
    DuplicateItem {
        /// The line.
        line: NonZeroUsize,
        /// The item's number.
        id: ItemId,
        /// The line of the first item with this number.
        first_line: NonZeroUsize,
    },
    /// A `Depends on:` line is not `none` or a list of feature numbers.
    #[error("line {line}: invalid Depends on: {text:?}")]
    MalformedDependsOn {
        /// The line.
        line: NonZeroUsize,
        /// What follows `Depends on:`.
        text: String,
    },
    /// A `Depends on:` line names a feature that is not defined before this one.
    #[error("line {line}: feature {number} is not defined before this feature")]
    UnknownDependency {
        /// The line.
        line: NonZeroUsize,
        /// The number.
        number: NonZeroU16,
    },
    /// A `Depends on:` line that is not right after its feature's line, or a second one.
    #[error("line {line}: Depends on: belongs right after the feature's line, once")]
    MisplacedDependsOn {
        /// The line.
        line: NonZeroUsize,
    },
    /// An indented line in a feature before its first item.
    #[error("line {line}: indented text outside an item")]
    TextBeforeItem {
        /// The line.
        line: NonZeroUsize,
    },
    /// A line at column 0 in a feature that is not a feature, an item or `Depends on:`.
    #[error("line {line}: text at column 0 inside a feature: indent it under its item")]
    UnexpectedLine {
        /// The line.
        line: NonZeroUsize,
    },
    /// An item has two parts with the same label.
    #[error("line {line}: the item has {part} twice")]
    DuplicatePart {
        /// The line of the second label.
        line: NonZeroUsize,
        /// The label.
        part: ItemPart,
    },
    /// A label has nothing after it.
    #[error("line {line}: {part} has no text")]
    EmptyPart {
        /// The line of the label.
        line: NonZeroUsize,
        /// The label.
        part: ItemPart,
    },
}

impl TaskList {
    /// Parses a task list.
    ///
    /// # Errors
    /// A [`TaskListError`]: the text is empty or over [`MAX_TASK_LIST_BYTES`], or a line is not valid where it is;
    /// all but those two carry the line.
    pub fn parse(text: &str) -> Result<Self, TaskListError> {
        let _ = text;
        Ok(Self {
            coordinator: None,
            brief: String::new(),
            features: Vec::new(),
        })
    }

    /// The name from the `Coordinator:` line, if the list starts with one. It may name a normal agent or an
    /// investigator, which have the same form: the configuration says which it is.
    #[must_use]
    pub fn coordinator(&self) -> Option<&AgentName> {
        self.coordinator.as_ref()
    }

    /// The text before the first feature, which every member reads; the whole text when there are no features.
    #[must_use]
    pub fn brief(&self) -> &str {
        &self.brief
    }

    /// The features, in the order written.
    #[must_use]
    pub fn features(&self) -> &[Feature] {
        &self.features
    }

    /// The feature numbered `number`, if the list has it.
    #[must_use]
    pub fn feature(&self, number: NonZeroU16) -> Option<&Feature> {
        self.features.iter().find(|feature| feature.number == number)
    }

    /// Every item, feature by feature, in the order written.
    pub fn items(&self) -> impl Iterator<Item = &Item> {
        self.features.iter().flat_map(|feature| feature.items.iter())
    }

    /// The item numbered `id`, if the list has it.
    #[must_use]
    pub fn item(&self, id: ItemId) -> Option<&Item> {
        self.items().find(|item| item.id == id)
    }
}

impl FromStr for TaskList {
    type Err = TaskListError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        Self::parse(text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parsed(text: &str) -> TaskList {
        TaskList::parse(text).unwrap()
    }

    fn failure(text: &str) -> TaskListError {
        TaskList::parse(text).unwrap_err()
    }

    fn line(n: usize) -> NonZeroUsize {
        NonZeroUsize::new(n).unwrap()
    }

    fn number(n: u16) -> NonZeroU16 {
        NonZeroU16::new(n).unwrap()
    }

    fn id(text: &str) -> ItemId {
        text.parse().unwrap()
    }

    /// The titles of the items of `list`, in order.
    fn titles(list: &TaskList) -> Vec<&str> {
        list.items().map(|item| item.title.as_str()).collect()
    }

    /// A small list with every part.
    const SMALL: &str = "\
Build the thing.

1. First feature
Depends on: none
[ ] 1.1 First item
    Do this.
    Expected result:
    - a file
    Checks:
    - run it
[x] 1.2 Done item
";

    #[test]
    fn a_list_has_a_brief_features_items_and_their_parts() {
        let list = parsed(SMALL);

        assert_eq!(list.coordinator(), None);
        assert_eq!(list.brief(), "Build the thing.");
        assert_eq!(list.features().len(), 1);
        let feature = &list.features()[0];
        assert_eq!((feature.number, feature.title.as_str()), (number(1), "First feature"));
        assert_eq!(feature.depends_on, Vec::<NonZeroU16>::new());
        assert_eq!(feature.items.len(), 2);

        let first = &feature.items[0];
        assert_eq!(
            (first.id, first.title.as_str(), first.mark),
            (id("1.1"), "First item", ItemMark::Open)
        );
        assert_eq!(first.description, "Do this.");
        assert_eq!(first.expected_result.as_deref(), Some("- a file"));
        assert_eq!(first.checks.as_deref(), Some("- run it"));
        assert!(!first.is_done());

        let second = &feature.items[1];
        assert_eq!(
            (second.id, second.title.as_str(), second.mark),
            (id("1.2"), "Done item", ItemMark::Done)
        );
        assert_eq!(second.description, "");
        assert_eq!(
            (second.expected_result.as_deref(), second.checks.as_deref()),
            (None, None)
        );
        assert!(second.is_done());
    }

    #[test]
    fn both_marks_of_done_are_accepted() {
        let list = parsed("1. F\n[x] 1.1 a\n[X] 1.2 b\n[ ] 1.3 c\n");
        let marks: Vec<_> = list.items().map(|item| item.mark).collect();
        assert_eq!(marks, [ItemMark::Done, ItemMark::Done, ItemMark::Open]);
    }

    #[test]
    fn text_without_features_is_all_brief() {
        let list = parsed("Fix the flaky test in foo.\n\n  It fails on CI.\n");
        assert_eq!(list.brief(), "Fix the flaky test in foo.\n\n  It fails on CI.");
        assert_eq!(list.features().len(), 0);
        assert_eq!(list.items().count(), 0);
    }

    #[test]
    fn a_blank_text_or_one_with_only_a_coordinator_is_empty() {
        for text in [
            "",
            "\n",
            "  \n\t\n \n",
            "Coordinator: claude-1\n",
            "\n\nCoordinator: claude-1\n\n",
        ] {
            assert_eq!(failure(text), TaskListError::Empty, "{text:?}");
        }
    }

    #[test]
    fn a_text_over_the_limit_is_refused_and_one_at_the_limit_is_read() {
        let at_the_limit = "x".repeat(MAX_TASK_LIST_BYTES);
        assert_eq!(parsed(&at_the_limit).brief().len(), MAX_TASK_LIST_BYTES);
        let over = format!("{at_the_limit}x");
        assert_eq!(
            failure(&over),
            TaskListError::TooLarge {
                len: MAX_TASK_LIST_BYTES + 1,
                max: MAX_TASK_LIST_BYTES
            }
        );
    }

    #[test]
    fn the_coordinator_line_names_the_coordinator() {
        for text in [
            "Coordinator: inv-opus\n\nBrief\n",
            "Coordinator:inv-opus\nBrief\n",
            "\u{feff}\n\nCoordinator:   inv-opus   \n\nBrief\n",
            "\u{feff}Coordinator: inv-opus\r\n\r\nBrief\r\n",
        ] {
            let list = parsed(text);
            assert_eq!(list.coordinator().map(AgentName::as_str), Some("inv-opus"), "{text:?}");
            assert_eq!(list.brief(), "Brief", "{text:?}");
        }
    }

    #[test]
    fn a_coordinator_line_that_is_not_the_first_line_is_text_of_the_brief() {
        for text in [
            "Brief\nCoordinator: inv-opus\n",
            "coordinator: inv-opus\nBrief\n",
            " Coordinator: inv-opus\n",
        ] {
            let list = parsed(text);
            assert_eq!(list.coordinator(), None, "{text:?}");
            assert!(list.brief().contains("oordinator: inv-opus"), "{text:?}");
        }
    }

    #[test]
    fn a_coordinator_line_with_no_valid_name_is_refused_with_its_line() {
        for (text, at) in [
            ("Coordinator:\nBrief\n", 1),
            ("Coordinator:   \nBrief\n", 1),
            ("Coordinator: Bad Name\nBrief\n", 1),
            ("Coordinator: two names\nBrief\n", 1),
            ("\n\nCoordinator: Żółw\nBrief\n", 3),
        ] {
            let err = failure(text);
            assert!(
                matches!(&err, TaskListError::InvalidCoordinator { line: l, .. } if *l == line(at)),
                "{text:?}: {err:?}"
            );
        }
    }

    #[test]
    fn inline_text_and_file_text_give_the_same_list() {
        let expected = parsed(SMALL);
        // A file may end without a newline, have CRLF line ends, and start with a byte order mark.
        let no_final_newline = SMALL.trim_end_matches('\n');
        let crlf = SMALL.replace('\n', "\r\n");
        let bom = format!("\u{feff}{SMALL}");
        for text in [no_final_newline, crlf.as_str(), bom.as_str(), &format!("{SMALL}\n\n")] {
            assert_eq!(parsed(text), expected, "{text:?}");
        }
        assert_eq!(SMALL.parse::<TaskList>().unwrap(), expected);
    }

    #[test]
    fn features_are_read_in_the_order_written_with_their_dependencies() {
        let text = "\
Brief

1. One
Depends on: none
[ ] 1.1 a
2. Two
Depends on: 1
[ ] 2.1 b
[ ] 2.2 c

3. Three
[ ] 3.1 d
4. Four
Depends on: 2, 3
[ ] 4.1 e
5. Five
Depends on: 1 4
6. Six
Depends on:  None
7. Seven
Depends on: 2,3,1,
";
        let list = parsed(text);
        let seen: Vec<_> = list
            .features()
            .iter()
            .map(|f| {
                (
                    f.number.get(),
                    f.title.as_str(),
                    f.depends_on.iter().map(|d| d.get()).collect::<Vec<_>>(),
                    f.items.len(),
                )
            })
            .collect();
        assert_eq!(
            seen,
            [
                (1, "One", vec![], 1),
                (2, "Two", vec![1], 2),
                (3, "Three", vec![], 1),
                (4, "Four", vec![2, 3], 1),
                (5, "Five", vec![1, 4], 0),
                (6, "Six", vec![], 0),
                (7, "Seven", vec![2, 3, 1], 0),
            ]
        );
    }

    #[test]
    fn items_and_features_are_found_by_their_numbers() {
        let list = parsed("1. One\n[ ] 1.1 a\n[ ] 1.2 b\n2. Two\n[ ] 2.1 c\n");
        assert_eq!(titles(&list), ["a", "b", "c"]);
        assert_eq!(list.item(id("1.2")).map(|i| i.title.as_str()), Some("b"));
        assert_eq!(list.item(id("2.1")).map(|i| i.title.as_str()), Some("c"));
        assert_eq!(list.item(id("2.2")), None);
        assert_eq!(list.feature(number(2)).map(|f| f.title.as_str()), Some("Two"));
        assert_eq!(list.feature(number(3)), None);
    }

    #[test]
    fn the_title_is_the_rest_of_the_line_without_the_spaces_around_it() {
        let list = parsed("1.   Spaced   out  \n[ ]   1.1   Title with  inner  spaces   \n");
        assert_eq!(list.features()[0].title, "Spaced   out");
        assert_eq!(list.features()[0].items[0].title, "Title with  inner  spaces");
    }

    #[test]
    fn titles_and_text_may_be_in_any_language() {
        let list = parsed("Zażółć gęślą jaźń\n\n1. Funkcja żółwia\n[ ] 1.1 Ścieżka 日本語\n    Opis: ✓ 🙂\n");
        assert_eq!(list.brief(), "Zażółć gęślą jaźń");
        assert_eq!(list.features()[0].title, "Funkcja żółwia");
        assert_eq!(list.features()[0].items[0].title, "Ścieżka 日本語");
        assert_eq!(list.features()[0].items[0].description, "Opis: ✓ 🙂");
    }

    #[test]
    fn a_feature_may_have_no_items() {
        let list = parsed("1. Empty\n2. Also\n[ ] 2.1 x\n");
        assert_eq!(list.features()[0].items.len(), 0);
        assert_eq!(list.features()[1].items.len(), 1);
    }

    #[test]
    fn the_brief_keeps_its_layout_and_stops_at_the_first_feature() {
        let text = "\n\nTitle\n\nLayout:\n    indented\n\n    [ ] 1.1 not an item\n  Depends on: 9\n[docs](x) is text\n\n\n1. F\n";
        let list = parsed(text);
        assert_eq!(
            list.brief(),
            "Title\n\nLayout:\n    indented\n\n    [ ] 1.1 not an item\n  Depends on: 9\n[docs](x) is text"
        );
        assert_eq!(list.features().len(), 1);
    }

    #[test]
    fn a_numbered_line_at_column_zero_starts_the_first_feature() {
        let list = parsed("Do these:\n1. first\n2. second\n");
        assert_eq!(list.brief(), "Do these:");
        let titles: Vec<_> = list.features().iter().map(|f| f.title.as_str()).collect();
        assert_eq!(titles, ["first", "second"]);

        // Indented, it is text.
        let list = parsed("Do these:\n  1. first\n  2. second\n");
        assert_eq!(list.features().len(), 0);
    }

    #[test]
    fn a_description_keeps_its_paragraphs_and_its_relative_indentation() {
        let text =
            "1. F\n[ ] 1.1 T\n    First paragraph.\n\n    Second:\n      - nested\n        deeper\n\n\n[ ] 1.2 U\n";
        let list = parsed(text);
        assert_eq!(
            list.features()[0].items[0].description,
            "First paragraph.\n\nSecond:\n  - nested\n    deeper"
        );
    }

    #[test]
    fn the_parts_end_at_the_next_label_or_the_next_item() {
        let text = "\
1. F
[ ] 1.1 T
    Description line.
    Expected result:
    - Files: `a`,
      `b`.

    - Adds: c.
    Checks:
    - one
    - two
[ ] 1.2 U
    Expected result:
    - only this
";
        let list = parsed(text);
        let first = &list.features()[0].items[0];
        assert_eq!(first.description, "Description line.");
        assert_eq!(
            first.expected_result.as_deref(),
            Some("- Files: `a`,\n  `b`.\n\n- Adds: c.")
        );
        assert_eq!(first.checks.as_deref(), Some("- one\n- two"));
        let second = &list.features()[0].items[1];
        assert_eq!(second.description, "");
        assert_eq!(second.expected_result.as_deref(), Some("- only this"));
        assert_eq!(second.checks, None);
    }

    #[test]
    fn the_parts_may_come_in_either_order_and_without_a_description() {
        let list = parsed("1. F\n[ ] 1.1 T\n    Checks:\n    - c\n    Expected result:\n    - e\n");
        let item = &list.features()[0].items[0];
        assert_eq!(item.description, "");
        assert_eq!(item.expected_result.as_deref(), Some("- e"));
        assert_eq!(item.checks.as_deref(), Some("- c"));
    }

    #[test]
    fn a_label_is_a_label_only_alone_on_a_line_at_the_indentation_of_the_item_text() {
        let text = "\
1. F
[ ] 1.1 T
    Text.
        Expected result:
    Expected result: inline
      Checks:
    - Expected result:
    expected result:
";
        let list = parsed(text);
        let item = &list.features()[0].items[0];
        assert_eq!(item.expected_result, None);
        assert_eq!(item.checks, None);
        assert!(
            item.description.contains("    Expected result:"),
            "{}",
            item.description
        );
        assert!(
            item.description.contains("Expected result: inline"),
            "{}",
            item.description
        );
    }

    #[test]
    fn indentation_may_be_tabs() {
        let list = parsed("1. F\n[ ] 1.1 T\n\tDo it.\n\tExpected result:\n\t- done\n\t\tnested\n");
        let item = &list.features()[0].items[0];
        assert_eq!(item.description, "Do it.");
        assert_eq!(item.expected_result.as_deref(), Some("- done\n\tnested"));
    }

    #[test]
    fn a_malformed_item_line_is_refused_with_its_line_number() {
        let bad_id = |text: &str| ItemProblem::BadId(text.parse::<ItemId>().unwrap_err());
        let cases = [
            ("[?] 1.1 T", ItemProblem::BadMark),
            ("[] 1.1 T", ItemProblem::BadMark),
            ("[ 1.1 T", ItemProblem::BadMark),
            ("[xx] 1.1 T", ItemProblem::BadMark),
            ("[ ]1.1 T", ItemProblem::NoSpaceAfterMark),
            ("[x]1.1 T", ItemProblem::NoSpaceAfterMark),
            ("[ ] 1.x T", bad_id("1.x")),
            ("[ ] 1 T", bad_id("1")),
            ("[ ] 1.1.1 T", bad_id("1.1.1")),
            ("[ ] 0.1 T", bad_id("0.1")),
            ("[ ] 1.01 T", bad_id("1.01")),
            ("[ ] 1.1T", bad_id("1.1T")),
            ("[ ] T", bad_id("T")),
            ("[ ] 1.1", ItemProblem::MissingTitle),
            ("[ ]   1.1   ", ItemProblem::MissingTitle),
            ("[ ] ", bad_id("")),
        ];
        for (item, problem) in cases {
            // The bad line comes after a feature line, a good item with its description, and blank lines.
            let text = format!("Brief\n\n1. F\n[ ] 1.1 Good\n    description\n\n{item}\n");
            assert_eq!(
                failure(&text),
                TaskListError::MalformedItem { line: line(7), problem },
                "{item:?}"
            );
        }
    }

    #[test]
    fn line_numbers_count_every_physical_line() {
        // A byte order mark, blank lines, CRLF line ends and a coordinator line do not move the count.
        let text = "\u{feff}Coordinator: a\r\n\r\nBrief\r\n\r\n1. F\r\n[?] 1.1 T\r\n";
        assert_eq!(
            failure(text),
            TaskListError::MalformedItem {
                line: line(6),
                problem: ItemProblem::BadMark
            }
        );
    }

    #[test]
    fn an_item_before_any_feature_is_refused_with_its_line() {
        assert_eq!(
            failure("Brief\n\n[ ] 1.1 T\n"),
            TaskListError::ItemBeforeFeature { line: line(3) }
        );
        assert_eq!(
            failure("[x] 1.1 T\n"),
            TaskListError::ItemBeforeFeature { line: line(1) }
        );
    }

    #[test]
    fn an_item_under_another_features_number_is_refused() {
        assert_eq!(
            failure("1. F\n[ ] 1.1 a\n[ ] 2.1 b\n"),
            TaskListError::ItemInOtherFeature {
                line: line(3),
                id: id("2.1"),
                feature: number(1)
            }
        );
    }

    #[test]
    fn a_repeated_item_number_is_refused_with_both_lines() {
        assert_eq!(
            failure("1. F\n[ ] 1.1 a\n[ ] 1.2 b\n[x] 1.1 c\n"),
            TaskListError::DuplicateItem {
                line: line(4),
                id: id("1.1"),
                first_line: line(2)
            }
        );
    }

    #[test]
    fn a_repeated_feature_number_is_refused_with_both_lines() {
        assert_eq!(
            failure("1. A\n2. B\n[ ] 2.1 x\n1. C\n"),
            TaskListError::DuplicateFeature {
                line: line(4),
                number: number(1),
                first_line: line(1)
            }
        );
    }

    #[test]
    fn a_bad_feature_number_or_title_is_refused_with_its_line() {
        for (head, text) in [
            ("0. Zero", "0"),
            ("01. Leading zero", "01"),
            ("65536. Too big", "65536"),
            ("99999999999999999999. Huge", "99999999999999999999"),
        ] {
            let all = format!("Brief\n{head}\n");
            assert_eq!(
                failure(&all),
                TaskListError::FeatureNumber {
                    line: line(2),
                    text: text.to_owned()
                },
                "{head:?}"
            );
        }
        for head in ["1.", "1.   ", "1.\t"] {
            let all = format!("Brief\n{head}\n");
            assert_eq!(
                failure(&all),
                TaskListError::FeatureWithoutTitle { line: line(2) },
                "{head:?}"
            );
        }
        assert_eq!(parsed("1. F\n65535. G\n").features()[1].number, number(65535));
    }

    #[test]
    fn a_depends_on_line_that_is_not_none_or_numbers_is_refused() {
        for text in [
            "x", "", "  ", "0", "01", "1.5", "none, 1", "1 none", "1, x", "-1", "65536", ",",
        ] {
            let all = format!("1. A\n2. B\nDepends on: {text}\n");
            assert_eq!(
                failure(&all),
                TaskListError::MalformedDependsOn {
                    line: line(3),
                    text: text.trim().to_owned()
                },
                "{text:?}"
            );
        }
    }

    #[test]
    fn a_feature_may_depend_only_on_features_before_it() {
        for (text, at, missing) in [
            ("1. A\nDepends on: 2\n2. B\n", 2, 2),
            ("1. A\nDepends on: 1\n", 2, 1),
            ("1. A\n2. B\nDepends on: 1, 3\n3. C\n", 3, 3),
            ("5. A\nDepends on: 7\n", 2, 7),
        ] {
            assert_eq!(
                failure(text),
                TaskListError::UnknownDependency {
                    line: line(at),
                    number: number(missing)
                },
                "{text:?}"
            );
        }
    }

    #[test]
    fn a_depends_on_line_is_right_after_its_feature_and_once() {
        for (text, at) in [
            ("1. A\n[ ] 1.1 x\nDepends on: none\n", 3),
            ("1. A\nDepends on: none\nDepends on: none\n", 3),
            ("1. A\n[ ] 1.1 x\n    d\nDepends on: none\n", 4),
        ] {
            assert_eq!(
                failure(text),
                TaskListError::MisplacedDependsOn { line: line(at) },
                "{text:?}"
            );
        }
        // Blank lines in between are fine.
        assert_eq!(
            parsed("1. A\n\n\nDepends on: none\n\n[ ] 1.1 x\n").features()[0]
                .items
                .len(),
            1
        );
    }

    #[test]
    fn indented_text_before_the_first_item_of_a_feature_is_refused() {
        assert_eq!(
            failure("1. A\n    text\n[ ] 1.1 x\n"),
            TaskListError::TextBeforeItem { line: line(2) }
        );
        assert_eq!(
            failure("1. A\nDepends on: none\n\n  text\n"),
            TaskListError::TextBeforeItem { line: line(4) }
        );
    }

    #[test]
    fn text_at_column_zero_inside_a_feature_is_refused() {
        assert_eq!(
            failure("1. A\nstray\n[ ] 1.1 x\n"),
            TaskListError::UnexpectedLine { line: line(2) }
        );
        assert_eq!(
            failure("1. A\n[ ] 1.1 x\n    d\nstray\n"),
            TaskListError::UnexpectedLine { line: line(4) }
        );
        assert_eq!(
            failure("1. A\n[ ] 1.1 x\n\nCoordinator: a\n"),
            TaskListError::UnexpectedLine { line: line(4) }
        );
    }

    #[test]
    fn a_label_used_twice_is_refused_at_the_second() {
        assert_eq!(
            failure("1. A\n[ ] 1.1 x\n    Expected result:\n    - a\n    Expected result:\n    - b\n"),
            TaskListError::DuplicatePart {
                line: line(5),
                part: ItemPart::ExpectedResult
            }
        );
        assert_eq!(
            failure("1. A\n[ ] 1.1 x\n    Checks:\n    - a\n    Expected result:\n    - b\n    Checks:\n    - c\n"),
            TaskListError::DuplicatePart {
                line: line(7),
                part: ItemPart::Checks
            }
        );
    }

    #[test]
    fn a_label_with_nothing_after_it_is_refused_at_the_label() {
        for (text, at, part) in [
            ("1. A\n[ ] 1.1 x\n    Expected result:\n", 3, ItemPart::ExpectedResult),
            (
                "1. A\n[ ] 1.1 x\n    Expected result:\n\n[ ] 1.2 y\n",
                3,
                ItemPart::ExpectedResult,
            ),
            (
                "1. A\n[ ] 1.1 x\n    Expected result:\n    Checks:\n    - c\n",
                3,
                ItemPart::ExpectedResult,
            ),
            (
                "1. A\n[ ] 1.1 x\n    Expected result:\n    - e\n    Checks:\n",
                5,
                ItemPart::Checks,
            ),
        ] {
            assert_eq!(
                failure(text),
                TaskListError::EmptyPart { line: line(at), part },
                "{text:?}"
            );
        }
    }

    #[test]
    fn the_first_fault_is_the_one_reported() {
        assert_eq!(
            failure("1. A\n[?] 1.1 x\n[ ] 9.9 y\nstray\n"),
            TaskListError::MalformedItem {
                line: line(2),
                problem: ItemProblem::BadMark
            }
        );
    }
}
