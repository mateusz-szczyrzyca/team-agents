//! The plan "Rewrite `bin/agent` in Rust" (`tests/fixtures/rust-agent-plan.md`) is a task list in the format of
//! `docs/task-list.md`, and the first fixture of the parser: it must parse into what it says.

use std::num::NonZeroUsize;

use agent_core::ids::ItemId;
use agent_core::task_list::{ItemMark, ItemProblem, TaskList, TaskListError};

/// The plan, as it was when this test was written.
const PLAN: &str = include_str!("fixtures/rust-agent-plan.md");

/// The numbers of the items of the plan, in order.
const ITEM_NUMBERS: [&str; 16] = [
    "1.1", "2.1", "2.2", "2.3", "3.1", "4.1", "4.2", "5.1", "6.1", "7.1", "7.2", "7.3", "8.1", "9.1", "10.1", "10.2",
];

#[test]
fn the_plan_parses_into_ten_features_and_sixteen_items() {
    let plan = TaskList::parse(PLAN).unwrap();

    assert_eq!(plan.features().len(), 10);
    assert_eq!(plan.items().count(), 16);
    let numbers: Vec<u16> = plan.features().iter().map(|feature| feature.number.get()).collect();
    assert_eq!(numbers, [1, 2, 3, 4, 5, 6, 7, 8, 9, 10]);
}

#[test]
fn the_items_of_the_plan_are_in_the_order_written_and_all_open() {
    let plan = TaskList::parse(PLAN).unwrap();

    let ids: Vec<String> = plan.items().map(|item| item.id.to_string()).collect();
    assert_eq!(ids, ITEM_NUMBERS);
    assert!(plan.items().all(|item| item.mark == ItemMark::Open));
    for item in plan.items() {
        let feature = plan.feature(item.id.feature()).unwrap();
        assert!(feature.items.iter().any(|candidate| candidate.id == item.id));
    }
}

#[test]
fn every_item_of_the_plan_has_a_title_a_description_an_expected_result_and_checks() {
    let plan = TaskList::parse(PLAN).unwrap();

    assert_eq!(plan.items().count(), 16);
    for item in plan.items() {
        assert!(!item.title.is_empty(), "{}", item.id);
        assert!(!item.description.is_empty(), "{}", item.id);
        let expected = item.expected_result.as_deref().unwrap_or_default();
        assert!(expected.starts_with("- "), "{}: {expected:?}", item.id);
        let checks = item.checks.as_deref().unwrap_or_default();
        assert!(checks.starts_with("- "), "{}: {checks:?}", item.id);
        // No label is left in a part it does not start: a mention in a sentence is text.
        for text in [item.description.as_str(), expected, checks] {
            assert!(
                !text
                    .lines()
                    .any(|line| matches!(line.trim(), "Expected result:" | "Checks:")),
                "{}",
                item.id
            );
        }
    }
}

#[test]
fn the_brief_of_the_plan_is_the_text_before_its_first_feature() {
    let plan = TaskList::parse(PLAN).unwrap();

    assert_eq!(plan.coordinator(), None);
    assert!(plan.brief().starts_with("Rewrite `bin/agent` in Rust — task list\n"));
    assert!(plan.brief().contains("\nGround rules\n"));
    assert!(plan.brief().contains("\nTarget layout\n\n    Cargo.toml"));
    assert!(
        plan.brief()
            .ends_with("an `agentbus` socket API (it is called as a process).")
    );
    assert!(!plan.brief().contains("Workspace and core vocabulary"));
}

#[test]
fn the_dependencies_of_the_plan_are_read() {
    let plan = TaskList::parse(PLAN).unwrap();

    let depends: Vec<(u16, Vec<u16>)> = plan
        .features()
        .iter()
        .map(|feature| {
            (
                feature.number.get(),
                feature.depends_on.iter().map(|d| d.get()).collect(),
            )
        })
        .collect();
    assert_eq!(
        depends,
        [
            (1, vec![]),
            (2, vec![1]),
            (3, vec![2]),
            (4, vec![2]),
            (5, vec![2]),
            (6, vec![3, 4, 5]),
            (7, vec![6]),
            (8, vec![7]),
            (9, vec![6]),
            (10, vec![8, 9]),
        ]
    );
    assert_eq!(plan.features()[1].title, "Core models and contracts");
}

#[test]
fn item_2_1_of_the_plan_is_read_into_its_three_parts() {
    let plan = TaskList::parse(PLAN).unwrap();
    let id: ItemId = "2.1".parse().unwrap();

    let item = plan.item(id).unwrap();

    assert_eq!(item.title, "Configuration and the task list file");
    assert!(
        item.description
            .starts_with("`agent-core`: the typed model of `agents.toml` and `secrets.toml`")
    );
    assert!(
        item.description
            .ends_with("file itself is a test fixture that must parse. `docs/task-list.md`.")
    );
    assert!(
        item.description
            .contains("\n`config/agents.example.toml`, `docs/configuration.md`.")
    );
    assert_eq!(
        item.expected_result.as_deref(),
        Some(
            "\
- Files: `crates/agent-core/src/{config,task_list}.rs` (or modules of those names),
  `config/agents.example.toml`, `docs/configuration.md`, `docs/task-list.md`.
- Adds: `Config`, `Secrets`, `AgentConfig`, `InvestigatorConfig`, `RepoConfig` and their typed load errors;
  `TaskList`, `Feature`, `Item` (title, description, expected result) and the parser with its typed errors.
- Tests: the example configuration loads; each validation error; task-list fixtures, this plan among them;
  line numbers in errors.
- Why: every start reads both, and an item's expected result is what its reviewers check."
        )
    );
    assert_eq!(
        item.checks.as_deref(),
        Some(
            "\
- `config/agents.example.toml` loads in a test; a copy with one name in both tier lists fails with the typed error
  naming that name.
- This plan file parses in a test into 10 features and 16 items, each item with its expected result.
- A task list with a malformed item line fails with the number of that line."
        )
    );
}

#[test]
fn a_malformed_item_line_in_the_plan_fails_with_the_number_of_that_line() {
    let good = "[ ] 2.2 State, control protocol and questions files";
    let bad = "[ ]2.2 State, control protocol and questions files";
    let broken = PLAN.replace(good, bad);
    assert_ne!(broken, PLAN, "the plan has no item line {good:?} to break");
    let at = PLAN.lines().position(|line| line == good).unwrap() + 1;

    let err = TaskList::parse(&broken).unwrap_err();

    assert_eq!(
        err,
        TaskListError::MalformedItem {
            line: NonZeroUsize::new(at).unwrap(),
            problem: ItemProblem::NoSpaceAfterMark
        }
    );
}

#[test]
fn the_plan_read_from_a_file_with_crlf_line_ends_is_the_same_list() {
    let crlf = PLAN.replace('\n', "\r\n");

    assert_eq!(TaskList::parse(&crlf).unwrap(), TaskList::parse(PLAN).unwrap());
}
