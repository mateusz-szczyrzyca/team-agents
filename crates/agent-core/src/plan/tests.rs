//! The states of a step, the `Files:` and `Check:` lines of its text, and the line of every refused form.
use rand::rngs::Xoshiro256PlusPlus;
use rand::{RngExt, SeedableRng};

use super::*;
use crate::repo_path::PathRule;
use crate::task_list::NumberProblem;

/// `n`, a literal, as the number of a line.
macro_rules! line {
    ($n:literal) => {
        NonZeroUsize::new($n).unwrap()
    };
}

/// `n` as the number of a line.
fn line(n: usize) -> NonZeroUsize {
    NonZeroUsize::new(n).unwrap()
}

/// Parses `text`, which the tests expect to be a plan.
fn parsed(text: &str) -> Plan {
    parse_plan(text).unwrap()
}

/// The error of parsing `text`, which the tests expect to be invalid.
fn failure(text: &str) -> PlanError {
    parse_plan(text).unwrap_err()
}

/// The step numbered `id` of `plan`, which the tests expect to be there.
fn step<'a>(plan: &'a Plan, id: &str) -> &'a Step {
    let id: ItemId = id.parse().unwrap();
    plan.stages
        .iter()
        .flat_map(Stage::steps)
        .find(|step| step.id == id)
        .unwrap()
}

/// The states of the steps of `plan`, with their IDs, in the order written.
fn states(plan: &Plan) -> Vec<(String, StepState)> {
    plan.stages
        .iter()
        .flat_map(Stage::steps)
        .map(|step| (step.id.to_string(), step.state))
        .collect()
}

/// A small plan: an intent, one stage and three steps.
const SMALL: &str = "\
Make the guide readable.

1. Draft
[ ] 1.1 Write the outline
    Start with the audience.
[>] 1.2 Write the first draft
[x] 1.3 Read it aloud
";

#[test]
fn a_plan_has_an_intent_stages_and_steps() {
    let plan = parsed(SMALL);

    assert_eq!(plan.intent(), "Make the guide readable.");
    assert_eq!(plan.stages.len(), 1);
    assert_eq!(plan.stages[0].title(), "Draft");
    assert_eq!(
        states(&plan),
        [
            ("1.1".to_owned(), StepState::Todo),
            ("1.2".to_owned(), StepState::InHand),
            ("1.3".to_owned(), StepState::Done),
        ]
    );
    assert_eq!(step(&plan, "1.1").note, "Start with the audience.");
    assert_eq!(step(&plan, "1.2").note, "");
}

#[test]
fn every_mark_of_a_step_is_a_state_of_its_own() {
    let plan = parsed("1. S\n[ ] 1.1 a\n[>] 1.2 b\n[x] 1.3 c\n[X] 1.4 d\n[-] 1.5 e\n    It is not needed.\n");

    assert_eq!(
        states(&plan),
        [
            ("1.1".to_owned(), StepState::Todo),
            ("1.2".to_owned(), StepState::InHand),
            ("1.3".to_owned(), StepState::Done),
            ("1.4".to_owned(), StepState::Done),
            ("1.5".to_owned(), StepState::GivenUp),
        ]
    );
    assert_eq!(step(&plan, "1.5").note, "It is not needed.");
}

#[test]
fn two_steps_in_hand_are_refused_with_both_lines() {
    let text = "1. S\n[>] 1.1 a\n[>] 1.2 b\n";

    assert_eq!(
        failure(text),
        PlanError::TwoInHand {
            line: line!(3),
            first_line: line!(2),
        }
    );
}

#[test]
fn one_step_in_hand_is_accepted_in_any_stage() {
    let plan = parsed("1. A\n[ ] 1.1 a\n2. B\n[>] 2.1 b\n");

    assert_eq!(step(&plan, "2.1").state, StepState::InHand);
}

#[test]
fn an_unknown_mark_is_refused_with_its_line() {
    for (mark, text) in [
        ("[?]", "[?] 1.1 a"),
        ("[-]", "[- ] 1.1 a"),
        ("[//]", "[//] 1.1 a"),
        ("[]", "[] 1.1 a"),
    ] {
        let all = format!("1. S\n[ ] 1.1 a\n{text}\n");
        assert_eq!(
            failure(&all),
            PlanError::MalformedStep {
                line: line!(3),
                problem: StepProblem::BadMark,
            },
            "{mark}"
        );
    }
}

#[test]
fn a_step_given_up_keeps_its_reason_as_its_note() {
    let plan = parsed("1. S\n[-] 1.1 Translate it\n    Nobody asked\n      for a translation.\n[ ] 1.2 b\n");

    let given_up = step(&plan, "1.1");

    assert_eq!(given_up.state(), StepState::GivenUp);
    assert_eq!(given_up.note(), "Nobody asked\n  for a translation.");
    assert_eq!(step(&plan, "1.2").note(), "");
}

#[test]
fn a_step_given_up_without_a_reason_is_refused_with_its_line() {
    let text = "1. S\n[-] 1.1 a\n  \n\t\n";

    assert_eq!(failure(text), PlanError::GivenUpWithoutReason { line: line!(2) });
}

/// The paths of the `Files:` line of the step numbered `id`, as text.
fn files(plan: &Plan, id: &str) -> Vec<String> {
    step(plan, id).files.iter().map(ToString::to_string).collect()
}

/// A step's `Files:` line lists the paths it is about, relative to the root of the repository.
#[test]
fn a_files_line_lists_the_paths_of_its_step() {
    let plan = parsed("1. S\n[ ] 1.1 a\n    Files: crates/agent-core/src/plan.rs, docs/plan.md\n");
    assert_eq!(files(&plan, "1.1"), ["crates/agent-core/src/plan.rs", "docs/plan.md"]);

    let plan = parsed("1. S\n[ ] 1.1 a\n\tFiles: Cargo.toml,Cargo.lock\n");
    assert_eq!(files(&plan, "1.1"), ["Cargo.toml", "Cargo.lock"]);

    let plan = parsed("1. S\n[ ] 1.1 a\n    Files: docs/zażółć.md\n");
    assert_eq!(files(&plan, "1.1"), ["docs/zażółć.md"]);
}

/// A path that is absolute or climbs out of the repository is refused, with the line of the `Files:` line.
#[test]
fn a_files_line_refuses_a_path_outside_the_repository_with_its_line() {
    for (files, rule) in [
        ("src/a.rs, /etc/passwd", PathRule::Absolute),
        ("../outside.rs", PathRule::DotSegment),
        ("src/../a.rs", PathRule::DotSegment),
        ("src//a.rs", PathRule::EmptySegment),
        ("./a.rs", PathRule::DotSegment),
    ] {
        let text = format!("1. S\n[ ] 1.1 a\n[ ] 1.2 b\n    Files: {files}\n");
        assert!(
            matches!(
                failure(&text),
                PlanError::BadFile { line, ref problem } if line == line!(4) && problem.rule == rule
            ),
            "{files}: {:?}",
            failure(&text)
        );
    }
}

/// A step has at most one `Files:` line and at most one `Check:` line.
#[test]
fn a_second_files_or_check_line_is_refused_with_its_line() {
    for (second, part) in [
        ("Files: a.rs", StepPart::Files),
        ("Files:", StepPart::Files),
        ("Check: cargo test", StepPart::Check),
        ("Check: reviewer", StepPart::Check),
    ] {
        let text = format!("1. S\n[ ] 1.1 a\n    Files: a.rs\n    Check: reviewer\n    {second}\n");
        assert_eq!(
            failure(&text),
            PlanError::DuplicatePart { line: line!(5), part },
            "{second}"
        );
    }
}

/// A `Check:` line says `reviewer`, `exists` with a path, or a command; nothing else is a check.
#[test]
fn a_check_line_is_a_reviewer_an_existence_or_a_command() {
    for (check, expected) in [
        ("reviewer", Check::Reviewer),
        ("exists docs/plan.md", Check::Exists("docs/plan.md".parse().unwrap())),
        (
            "cargo test -p agent-core",
            Check::Command("cargo test -p agent-core".to_owned()),
        ),
        (
            " Review   the  draft ",
            Check::Command("Review   the  draft".to_owned()),
        ),
        (
            "reviewer says it is fine",
            Check::Command("reviewer says it is fine".to_owned()),
        ),
        ("Żółć 日本語", Check::Command("Żółć 日本語".to_owned())),
        ("existsX", Check::Command("existsX".to_owned())),
        ("exists\tdocs/a b.md", Check::Exists("docs/a b.md".parse().unwrap())),
    ] {
        let text = format!("1. S\n[ ] 1.1 a\n    Check:{check}\n");
        assert_eq!(step(&parsed(&text), "1.1").check, Some(expected.clone()), "{check}");
    }
}

/// `exists` with a path outside the repository is refused, with the line of the `Check:` line.
#[test]
fn an_exists_check_refuses_a_path_outside_the_repository_with_its_line() {
    for (check, rule) in [
        ("exists /etc/passwd", PathRule::Absolute),
        ("exists ../a", PathRule::DotSegment),
        ("exists src//a", PathRule::EmptySegment),
    ] {
        let text = format!("1. S\n[ ] 1.1 a\n\n    Check: {check}\n");
        assert!(
            matches!(
                failure(&text),
                PlanError::BadCheck { line, ref problem } if line == line!(4) && problem.rule == rule
            ),
            "{check}: {:?}",
            failure(&text)
        );
    }
}

/// `exists` with no path after it is refused with the line of the `Check:` line.
#[test]
fn an_exists_check_without_a_path_is_refused_with_its_line() {
    for check in ["exists", "exists   ", "exists\t"] {
        let text = format!("1. S\n[ ] 1.1 a\n    Note.\n    Check: {check}\n");
        assert_eq!(
            failure(&text),
            PlanError::ExistsWithoutPath { line: line!(4) },
            "{check:?}"
        );
    }
}

/// A `Check:` line with nothing after it is refused with its line.
#[test]
fn an_empty_check_line_is_refused_with_its_line() {
    for check in ["Check:", "Check:   ", "Check:\t"] {
        let text = format!("1. S\n[ ] 1.1 a\n    {check}\n");
        let expected = PlanError::EmptyCheck { line: line!(3) };
        assert_eq!(failure(&text), expected, "{check}");
        assert_eq!(failure(&text).line(), Some(line!(3)), "{check}");
    }
}

/// The `Files:` and `Check:` lines are named only when they start their line of the step's text.
#[test]
fn a_files_or_check_line_is_named_only_at_the_start_of_its_line() {
    let plan = parsed("1. S\n[ ] 1.1 a\n    - Files: a.rs\n    See Check: later\n");
    assert_eq!(files(&plan, "1.1"), Vec::<String>::new());
    assert_eq!(step(&plan, "1.1").check, None);
    assert_eq!(step(&plan, "1.1").note, "- Files: a.rs\nSee Check: later");
}

/// Text with nothing in it but blank lines is not a plan.
#[test]
fn a_blank_text_is_refused() {
    for text in ["", "\n", "  \n\t\n \n", "\u{feff}"] {
        assert_eq!(failure(text), PlanError::Empty, "{text:?}");
    }
}

/// A plan's stages need a title and a number of their own; a repeated number names both its lines.
#[test]
fn a_stage_needs_a_title_and_a_number_of_its_own() {
    for (stage, problem, text) in [
        (
            "0. Zero",
            NumberProblem::Range("0".parse::<NonZeroU16>().unwrap_err()),
            "0",
        ),
        ("01. Leading zero", NumberProblem::Shape, "01"),
        (
            "70000. Too big",
            NumberProblem::Range("70000".parse::<NonZeroU16>().unwrap_err()),
            "70000",
        ),
    ] {
        let expected = PlanError::StageNumber {
            line: line!(2),
            text: text.to_owned(),
            problem,
        };
        assert_eq!(failure(&format!("Intent\n{stage}\n")), expected, "{stage}");
    }
    assert_eq!(
        failure("1.\n[ ] 1.1 a\n"),
        PlanError::StageWithoutTitle { line: line!(1) }
    );
    assert_eq!(
        failure("1. A\n[ ] 1.1 a\n2. B\n1. C\n"),
        PlanError::DuplicateStage {
            line: line!(4),
            number: NonZeroU16::new(1).unwrap(),
            first_line: line!(1),
        }
    );
}

/// A step's number is its own, it is in the stage its number names, and a repeated number names both lines.
#[test]
fn a_step_needs_a_number_of_its_own_in_its_stage() {
    let bad_id = |text: &str| StepProblem::BadId(text.parse::<ItemId>().unwrap_err());
    for (step, problem) in [
        ("[ ]1.1 a", StepProblem::NoSpaceAfterMark),
        ("[>]1.1 a", StepProblem::NoSpaceAfterMark),
        ("[ ] 1.x a", bad_id("1.x")),
        ("[ ] 1.1   ", StepProblem::MissingTitle),
        ("[ ] 1.1", StepProblem::MissingTitle),
    ] {
        let all = format!("1. S\n[ ] 1.9 b\n{step}\n");
        assert_eq!(
            failure(&all),
            PlanError::MalformedStep {
                line: line!(3),
                problem
            },
            "{step}"
        );
    }
    assert_eq!(
        failure("1. S\n[ ] 1.1 a\n[ ] 2.1 b\n2. T\n[ ] 2.2 c\n"),
        PlanError::StepInOtherStage {
            line: line!(3),
            id: "2.1".parse().unwrap(),
            stage: NonZeroU16::new(1).unwrap(),
        }
    );
    assert_eq!(
        failure("1. S\n[x] 1.1 a\n[x] 1.1 b\n"),
        PlanError::DuplicateStep {
            line: line!(3),
            id: "1.1".parse().unwrap(),
            first_line: line!(2),
        }
    );
}

/// A step comes after the first stage; text at column 0 inside a stage is the author's mistake of indentation.
#[test]
fn steps_belong_to_a_stage_and_only_steps_stand_at_column_zero_in_one() {
    assert_eq!(
        failure("The intent.\n[ ] 1.1 a\n1. S\n"),
        PlanError::StepBeforeStage { line: line!(2) }
    );
    assert_eq!(
        failure("The intent.\n[>] 1.1 a\n"),
        PlanError::StepBeforeStage { line: line!(2) }
    );
    for text in [
        "1. S\nDepends on: none\n[ ] 1.1 a\n",
        "1. S\n[ ] 1.1 a\nstray\n",
        "1. S\nAllow: Cargo.toml\n",
        "1. S\n[ ] 1.1 a\nCoordinator: inv-opus\n",
    ] {
        let at = text
            .lines()
            .position(|line| line.starts_with(['s', 'D', 'A', 'C']))
            .unwrap()
            + 1;
        assert_eq!(failure(text), PlanError::UnexpectedLine { line: line(at) }, "{text:?}");
    }
    assert_eq!(
        failure("1. S\n    text\n[ ] 1.1 a\n"),
        PlanError::TextBeforeStep { line: line!(2) }
    );
    // Text before the first stage is the intent, whatever it looks like, unless it is a step.
    let plan = parsed("[?] a checkbox is intent here, not a step\nDepends on: nothing\n");
    assert_eq!(
        plan.intent(),
        "[?] a checkbox is intent here, not a step\nDepends on: nothing"
    );
}

/// Line numbers count every physical line of the file, byte order mark, CRLF ends and blank lines included.
#[test]
fn line_numbers_count_every_physical_line() {
    let text = "\u{feff}Intent\r\n\r\n1. S\r\n[ ] 1.1 a\r\n    Check: exists a.rs\r\n    Check: reviewer\r\n";
    assert_eq!(
        failure(text),
        PlanError::DuplicatePart {
            line: line!(6),
            part: StepPart::Check
        }
    );
    // Inline text and file text with either line end give the same plan.
    let plain = "Intent\n\n1. S\n[ ] 1.1 a\n    Note.\n";
    let expected = parsed(plain);
    assert_eq!(parsed(plain.trim_end_matches('\n')), expected);
    assert_eq!(parsed(&plain.replace('\n', "\r\n")), expected);
    assert_eq!(parsed(&format!("\u{feff}{plain}")), expected);
}

/// A plan over the limit of the task list, which it shares, is refused; one at the limit is read.
#[test]
fn a_plan_over_the_limit_is_refused() {
    let at_the_limit = format!("x{}", "a".repeat(MAX_TASK_LIST_BYTES - 1));
    assert_eq!(parsed(&at_the_limit).intent(), at_the_limit);
    let over = format!("{at_the_limit}a");
    assert_eq!(
        failure(&over),
        PlanError::TooLarge {
            len: MAX_TASK_LIST_BYTES + 1,
            max: MAX_TASK_LIST_BYTES,
        }
    );
}

/// The first fault of the text is the one reported.
#[test]
fn the_first_fault_is_the_one_reported() {
    let text = "1. S\n[?] 1.1 a\n[>] 1.2 b\n[>] 1.3 c\nstray\n";
    assert_eq!(
        failure(text),
        PlanError::MalformedStep {
            line: line!(2),
            problem: StepProblem::BadMark
        }
    );
}

/// A `[-]` step whose text is only its `Files:` and `Check:` lines has no reason.
#[test]
fn a_step_given_up_with_only_its_files_and_check_has_no_reason() {
    let text = "1. S\n[ ] 1.1 a\n[-] 1.2 b\n    Files: a.rs\n    Check: reviewer\n";

    assert_eq!(failure(text), PlanError::GivenUpWithoutReason { line: line!(3) });
}

/// A plan written back is the text a task keeps, and reading it again gives the same plan.
#[test]
fn a_plan_written_back_reads_as_the_same_plan() {
    let text = "\
Make the guide readable.

1. Draft
[ ] 1.1 Write the outline
    Start with the audience.
    Files: docs/plan.md, crates/agent-core/src/plan.rs
    Check: exists docs/plan.md
[>] 1.2 Write the first draft
2. Publish
[x] 2.1 Add the guide to the release
    Check: cargo test -p guide
[-] 2.2 Translate it
    Nobody asked for it.
    Check: reviewer
";
    let plan = parsed(text);

    let written = plan.to_text();

    assert_eq!(parsed(&written), plan);
    assert_eq!(
        written,
        "\
Make the guide readable.

1. Draft
[ ] 1.1 Write the outline
    Start with the audience.
    Files: docs/plan.md, crates/agent-core/src/plan.rs
    Check: exists docs/plan.md
[>] 1.2 Write the first draft
2. Publish
[x] 2.1 Add the guide to the release
    Check: cargo test -p guide
[-] 2.2 Translate it
    Nobody asked for it.
    Check: reviewer
"
    );
}

/// A plan without an intent, with an empty stage and a step of a title and no text, is written and read back.
#[test]
fn a_plan_of_edges_written_back_reads_as_the_same_plan() {
    for text in [
        "1. S\n",
        "\n\n1. S\n[ ] 1.1 a\n2. Empty\n",
        "Only the intent.\n\nWith two paragraphs.",
        "[?] not a step: intent\n",
    ] {
        let plan = parsed(text);
        assert_eq!(parsed(&plan.to_text()), plan, "{text:?}");
    }
}

/// Lines a generated plan's intent and notes are made of: none starts a stage, a step or a label.
const PROSE: [&str; 6] = [
    "Make the guide readable.",
    "Zażółć gęślą jaźń — 日本語 too.",
    "- a bullet, not a step",
    "  indented two more",
    "",
    "3 steps, not a stage",
];

/// Titles a generated plan's stages and steps take.
const TITLES: [&str; 4] = ["Draft", "Write the outline", "Przetłumacz to", "Check [x] marks"];

/// Paths a generated step's `Files:` line or `exists` check names.
const PATHS: [&str; 4] = [
    "Cargo.toml",
    "docs/plan.md",
    "crates/agent-core/src/plan.rs",
    "docs/zażółć jaźń.md",
];

/// Commands a generated step's `Check:` line runs.
const COMMANDS: [&str; 4] = [
    "cargo test -p agent-core plan",
    "make gate",
    "reviewer says ok",
    "test -s docs/plan.md",
];

/// One of `items`, drawn from `rng`.
fn pick<'a>(rng: &mut Xoshiro256PlusPlus, items: &[&'a str]) -> &'a str {
    items[rng.random_range(0..items.len())]
}

/// Up to `max` lines of [`PROSE`], joined as the parser keeps them: no blank line at the start or the end, and no
/// indentation common to all of them.
fn prose(rng: &mut Xoshiro256PlusPlus, max: usize) -> String {
    let count = rng.random_range(0..=max);
    let lines: Vec<&str> = (0..count).map(|_| pick(rng, &PROSE)).collect();
    let text = task_list::join_trimmed(&lines);
    if text.lines().all(|line| line.trim().is_empty() || line.starts_with(' ')) {
        return String::new();
    }
    text
}

/// A plan drawn from `rng`: up to four stages of up to four steps, every state, every kind of check, `Files:` lines
/// and notes, with at most one step in hand and a note on every step given up.
fn generated_plan(rng: &mut Xoshiro256PlusPlus) -> Plan {
    let mut in_hand = false;
    let mut stages = Vec::new();
    let mut number = 0u16;
    for _ in 0..rng.random_range(1..=4) {
        number += rng.random_range(1..=3);
        let stage_number = NonZeroU16::new(number).unwrap();
        let mut steps = Vec::new();
        let mut step_number = 0u16;
        for _ in 0..rng.random_range(0..=4) {
            step_number += rng.random_range(1..=2);
            let mut state =
                [StepState::Todo, StepState::InHand, StepState::Done, StepState::GivenUp][rng.random_range(0..4)];
            if state == StepState::InHand && std::mem::replace(&mut in_hand, true) {
                state = StepState::Done;
            }
            let mut note = prose(rng, 3);
            if state == StepState::GivenUp && note.is_empty() {
                note = "Nobody asked for it.".to_owned();
            }
            let files = (0..rng.random_range(0..=3))
                .map(|_| pick(rng, &PATHS).parse().unwrap())
                .collect();
            let check = match rng.random_range(0..4) {
                0 => None,
                1 => Some(Check::Reviewer),
                2 => Some(Check::Exists(pick(rng, &PATHS).parse().unwrap())),
                _ => Some(Check::Command(pick(rng, &COMMANDS).to_owned())),
            };
            steps.push(Step {
                id: format!("{number}.{step_number}").parse().unwrap(),
                title: pick(rng, &TITLES).to_owned(),
                state,
                files,
                check,
                note,
            });
        }
        stages.push(Stage {
            number: stage_number,
            title: pick(rng, &TITLES).to_owned(),
            steps,
        });
    }
    Plan {
        intent: prose(rng, 4),
        stages,
    }
}

/// Any plan written back reads as the same plan: a property over plans drawn from a seeded source.
#[test]
fn every_generated_plan_written_back_reads_as_the_same_plan() {
    for seed in 0..500 {
        let mut rng = Xoshiro256PlusPlus::seed_from_u64(seed);
        let plan = generated_plan(&mut rng);

        let text = plan.to_text();

        assert_eq!(parse_plan(&text), Ok(plan), "seed {seed}:\n{text}");
    }
}

/// The IDs of `steps` as text.
fn ids(steps: &[ItemId]) -> Vec<String> {
    steps.iter().map(ToString::to_string).collect()
}

/// A plan with steps in a known state, read from its text.
fn version(text: &str) -> Plan {
    parsed(text)
}

/// The older version's steps in the newer one are added; the one in both keeps its ID.
#[test]
fn a_comparison_reports_the_added_steps() {
    let old = version("1. S\n[x] 1.1 a\n");
    let new = version("1. S\n[x] 1.1 a\n[ ] 1.2 b\n2. T\n[ ] 2.1 c\n");

    let changes = old.compare(&new);

    assert_eq!(ids(&changes.added), ["1.2", "2.1"]);
    assert_eq!(changes.state_changed, []);
    assert_eq!(changes.disappeared, []);
    assert_eq!(changes.retitled, []);
    assert!(!changes.is_empty());
}

/// A step whose mark changed is reported with its state before and after.
#[test]
fn a_comparison_reports_the_steps_that_changed_their_state() {
    let old = version("1. S\n[ ] 1.1 a\n[>] 1.2 b\n[ ] 1.3 c\n");
    let new = version("1. S\n[x] 1.1 a\n[x] 1.2 b\n[ ] 1.3 c\n");

    let changes = old.compare(&new);

    assert_eq!(
        changes.state_changed,
        [
            StateChange {
                id: "1.1".parse().unwrap(),
                from: StepState::Todo,
                to: StepState::Done,
            },
            StateChange {
                id: "1.2".parse().unwrap(),
                from: StepState::InHand,
                to: StepState::Done,
            },
        ]
    );
    assert_eq!(changes.added, []);
}

/// A step of the older version that the newer one no longer has disappeared.
#[test]
fn a_comparison_reports_the_steps_that_disappeared() {
    let old = version("1. S\n[ ] 1.1 a\n[x] 1.2 b\n[-] 1.3 c\n    Dropped.\n2. T\n[ ] 2.1 d\n");
    let new = version("1. S\n[ ] 1.1 a\n2. T\n[ ] 2.1 d\n");

    let changes = old.compare(&new);

    assert_eq!(ids(&changes.disappeared), ["1.2", "1.3"]);
    assert_eq!(changes.added, []);
    assert_eq!(changes.state_changed, []);
}

/// A step whose ID the newer version uses for another title is reported with both titles.
#[test]
fn a_comparison_reports_an_id_used_for_another_step() {
    let old = version("1. S\n[ ] 1.1 Write the outline\n[ ] 1.2 Read it aloud\n");
    let new = version("1. S\n[ ] 1.1 Write the outline again\n[ ] 1.2 Read it aloud\n");

    let changes = old.compare(&new);

    assert_eq!(
        changes.retitled,
        [Retitle {
            id: "1.1".parse().unwrap(),
            was: "Write the outline".to_owned(),
            now: "Write the outline again".to_owned(),
        }]
    );
    assert!(!changes.is_empty());
}

/// A plan compared with itself has nothing to report.
#[test]
fn a_plan_compared_with_itself_has_no_changes() {
    let plan = version("1. S\n[x] 1.1 a\n[>] 1.2 b\n[-] 1.3 c\n    Dropped.\n");

    let changes = plan.compare(&plan.clone());

    assert_eq!(changes, PlanChanges::default());
    assert!(changes.is_empty());
    assert!(!changes.loses_a_step());
}

/// One comparison of two versions finds every kind of change at once.
#[test]
fn a_comparison_of_two_versions_finds_every_kind_of_change() {
    let old = version(
        "1. S\n[ ] 1.1 kept\n[ ] 1.2 state\n[ ] 1.3 gone\n[ ] 1.4 same\n[ ] 1.5 retitled\n2. T\n[x] 2.1 kept\n",
    );
    let new = version(
        "1. S\n[ ] 1.1 kept\n[>] 1.2 state\n[ ] 1.4 same\n[ ] 1.5 retitled now\n[ ] 1.6 added\n2. T\n[x] 2.1 kept\n[ ] 2.2 added later\n",
    );

    let changes = old.compare(&new);

    assert_eq!(
        changes,
        PlanChanges {
            added: vec!["1.6".parse().unwrap(), "2.2".parse().unwrap()],
            state_changed: vec![StateChange {
                id: "1.2".parse().unwrap(),
                from: StepState::Todo,
                to: StepState::InHand,
            }],
            disappeared: vec!["1.3".parse().unwrap()],
            retitled: vec![Retitle {
                id: "1.5".parse().unwrap(),
                was: "retitled".to_owned(),
                now: "retitled now".to_owned(),
            }],
        }
    );
    assert!(changes.loses_a_step());
    // The newer version compared with the older one reports the other direction.
    let back = new.compare(&old);
    assert_eq!(ids(&back.added), ["1.3"]);
    assert_eq!(ids(&back.disappeared), ["1.6", "2.2"]);
}

/// A fault in a step's text is reported before the text at column 0 that ends the step, as the first fault is.
#[test]
fn a_fault_in_a_step_text_comes_before_the_stray_line_after_it() {
    let bad_file = failure("1. S\n[ ] 1.1 a\n    Files: ../a\nstray\n");
    assert!(
        matches!(&bad_file, PlanError::BadFile { line, problem } if *line == line!(3) && problem.rule == PathRule::DotSegment),
        "{bad_file:?}"
    );
    let bad_check = failure("1. S\n[ ] 1.1 a\n    Check: exists /etc/passwd\nstray\n");
    assert!(
        matches!(&bad_check, PlanError::BadCheck { line, problem } if *line == line!(3) && problem.rule == PathRule::Absolute),
        "{bad_check:?}"
    );
    assert_eq!(
        failure("1. S\n[-] 1.1 a\nstray\n"),
        PlanError::GivenUpWithoutReason { line: line!(2) }
    );
}

/// A `Files:` line with no path, or with an empty entry between its commas, is refused with its line: a path is
/// never dropped, and the line written back can never be lost.
#[test]
fn a_files_line_with_an_empty_entry_is_refused_with_its_line() {
    for files in [
        "Files:",
        "Files:   ",
        "Files: a.rs,,b.rs",
        "Files: a.rs,",
        "Files: , a.rs",
    ] {
        let text = format!("1. S\n[ ] 1.1 a\n    {files}\n        Check: echo example\n");
        let error = failure(&text);
        assert!(
            matches!(&error, PlanError::BadFile { line, problem } if *line == line!(3) && problem.rule == PathRule::Empty),
            "{files:?}: {error:?}"
        );
    }
}

/// A plan at the limit is written back into a text at the limit, which reads as the same plan.
#[test]
fn a_plan_at_the_limit_written_back_reads_as_the_same_plan() {
    let plan = parsed(&"x".repeat(MAX_TASK_LIST_BYTES));

    let written = plan.to_text();

    assert_eq!(written.len(), MAX_TASK_LIST_BYTES);
    assert_eq!(parse_plan(&written), Ok(plan));
}

/// A text within the limit whose plan written back is over it is refused, so that every plan read can be written
/// back and read again.
#[test]
fn a_plan_whose_text_written_back_is_over_the_limit_is_refused() {
    let head = "1. S\n[ ] 1.1 a\n";
    let lines = (MAX_TASK_LIST_BYTES - head.len()) / " n\n".len();
    let text = format!("{head}{}", " n\n".repeat(lines));
    assert!(text.len() <= MAX_TASK_LIST_BYTES);

    assert_eq!(
        failure(&text),
        PlanError::TooLargeWrittenBack {
            len: head.len() + lines * "    n\n".len(),
            max: MAX_TASK_LIST_BYTES,
        }
    );
}

/// Each kind of change alone makes a comparison not empty; a step lost or retitled alone loses a step, a step added
/// or a state changed alone does not.
#[test]
fn each_kind_of_change_alone_counts() {
    let old = version("1. S\n[ ] 1.1 a\n[ ] 1.2 b\n");
    for (newer, loses) in [
        ("1. S\n[ ] 1.1 a\n[ ] 1.2 b\n[ ] 1.3 c\n", false),
        ("1. S\n[x] 1.1 a\n[ ] 1.2 b\n", false),
        ("1. S\n[ ] 1.1 a\n", true),
        ("1. S\n[ ] 1.1 a\n[ ] 1.2 another\n", true),
    ] {
        let changes = old.compare(&version(newer));

        assert!(!changes.is_empty(), "{newer:?}");
        assert_eq!(changes.loses_a_step(), loses, "{newer:?}");
    }
}
