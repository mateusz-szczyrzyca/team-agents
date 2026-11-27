//! The rule of a commit about to be made: the frozen tests of a round, and the commits that may and may not change them.

use super::*;
use crate::decide::checks::{Change, Lines};
use crate::repo_path::{PathPattern, PathPatterns};

fn patterns(texts: &[&str]) -> PathPatterns {
    PathPatterns::try_from(
        texts
            .iter()
            .map(|text| text.parse::<PathPattern>().unwrap())
            .collect::<Vec<_>>(),
    )
    .unwrap()
}

/// Tests under `tests/` and in `*_test.go`; code under `src/`.
fn rules() -> Checks {
    Checks {
        test_paths: patterns(&["tests/", "*_test.go"]),
        code_paths: patterns(&["src/"]),
        protected_paths: PathPatterns::default(),
    }
}

fn path(text: &str) -> RepoPath {
    text.parse().unwrap()
}

/// A commit ID of forty copies of `digit`.
fn id(digit: char) -> CommitId {
    digit.to_string().repeat(40).parse().unwrap()
}

fn file(path_text: &str, change: Change, lines: Lines) -> FileChange {
    FileChange {
        path: path(path_text),
        change,
        lines,
    }
}

fn modified(path_text: &str, added: u32, removed: u32) -> FileChange {
    file(path_text, Change::Modified, Lines::Text { added, removed })
}

fn added(path_text: &str, lines: u32) -> FileChange {
    file(
        path_text,
        Change::Added,
        Lines::Text {
            added: lines,
            removed: 0,
        },
    )
}

fn commit(digit: char, subject: &str, files: Vec<FileChange>) -> CommitFacts {
    CommitFacts {
        id: id(digit),
        parents: Vec::new(),
        subject: subject.to_owned(),
        body: String::new(),
        files,
        merge_files: Vec::new(),
    }
}

fn frozen(path_text: &str, red: char) -> FrozenTest {
    FrozenTest {
        path: path(path_text),
        red: id(red),
    }
}

fn judge(frozen_tests: &[FrozenTest], subject: &str, files: &[FileChange]) -> Verdict {
    judge_commit(&rules(), frozen_tests, &PendingCommit { subject, files })
}

// Rule: the frozen tests are the test files that the round's `RED:` commits touched, each once, with the first of them.
#[test]
fn the_frozen_tests_are_the_test_files_of_the_red_commits() {
    let commits = vec![
        commit(
            '1',
            "Add the parser",
            vec![added("src/a.rs", 9), added("tests/early.rs", 3)],
        ),
        commit(
            '2',
            "RED: parse a line",
            vec![added("tests/a.rs", 5), added("src/a.rs", 1)],
        ),
        commit(
            '3',
            "RED: parse two lines",
            vec![modified("tests/a.rs", 4, 0), added("tests/b.rs", 2)],
        ),
        commit('4', "RED: stub only", vec![added("src/c.rs", 1)]),
        commit(
            '5',
            "GREEN: parse",
            vec![modified("src/a.rs", 6, 1), added("tests/after.rs", 1)],
        ),
    ];

    let found = frozen_tests(&rules(), &commits);

    assert_eq!(found, vec![frozen("tests/a.rs", '2'), frozen("tests/b.rs", '3')]);
}

#[test]
fn a_round_without_red_commits_freezes_nothing() {
    let commits = vec![commit('1', "Add a test", vec![added("tests/a.rs", 5)])];

    assert_eq!(frozen_tests(&rules(), &commits), Vec::new());
    assert_eq!(frozen_tests(&rules(), &[]), Vec::new());
}

fn edited(name: &str, red: char, taken: Taken) -> Refusal {
    Refusal::FrozenTestEdited {
        path: path(name),
        red: id(red),
        taken,
    }
}

fn tests() -> [FrozenTest; 2] {
    [frozen("tests/a.rs", '2'), frozen("tests/b.rs", '3')]
}

// Rule: a commit that takes nothing out of a frozen test is made.
#[test]
fn a_commit_that_takes_nothing_out_of_a_frozen_test_is_allowed() {
    let cases: Vec<(&str, Verdict)> = vec![
        (
            "nothing is frozen, so nothing is refused",
            judge(&[], "Tidy the test", &[modified("tests/a.rs", 1, 1)]),
        ),
        (
            "lines only added to a frozen test",
            judge(&tests(), "Cover one more case", &[modified("tests/a.rs", 4, 0)]),
        ),
        (
            "a new test file",
            judge(&tests(), "Add a test", &[added("tests/c.rs", 10)]),
        ),
        (
            "a test that no RED commit froze may be edited",
            judge(&tests(), "Tidy another test", &[modified("tests/other.rs", 2, 2)]),
        ),
        (
            "production code only",
            judge(&tests(), "Write the code", &[modified("src/a.rs", 8, 3)]),
        ),
    ];

    for (name, got) in cases {
        assert_eq!(got, Verdict::Allowed, "{name}");
    }
}

// Rule: a commit that takes lines out of a frozen test is refused, whatever it is for, and each test is named.
#[test]
fn a_commit_that_takes_lines_out_of_a_frozen_test_is_refused_and_names_each_test() {
    let cases: Vec<(&str, Verdict, Vec<Refusal>)> = vec![
        (
            "lines replaced in a frozen test",
            judge(&tests(), "Format the test", &[modified("tests/a.rs", 2, 2)]),
            vec![edited("tests/a.rs", '2', Taken::Lines(2))],
        ),
        (
            "a line removed from a frozen test next to production code",
            judge(
                &tests(),
                "Drop a debug line",
                &[modified("src/a.rs", 1, 0), modified("tests/b.rs", 0, 1)],
            ),
            vec![edited("tests/b.rs", '3', Taken::Lines(1))],
        ),
        (
            "two frozen tests edited",
            judge(
                &tests(),
                "Refactor the tests",
                &[modified("tests/a.rs", 1, 1), modified("tests/b.rs", 3, 2)],
            ),
            vec![
                edited("tests/a.rs", '2', Taken::Lines(1)),
                edited("tests/b.rs", '3', Taken::Lines(2)),
            ],
        ),
        (
            "a frozen test deleted",
            judge(
                &tests(),
                "Remove the test",
                &[file(
                    "tests/a.rs",
                    Change::Deleted,
                    Lines::Text { added: 0, removed: 5 },
                )],
            ),
            vec![edited("tests/a.rs", '2', Taken::WholeFile)],
        ),
        (
            "a frozen test moved: its old path is deleted",
            judge(
                &tests(),
                "Move the test",
                &[
                    file("tests/a.rs", Change::Deleted, Lines::Text { added: 0, removed: 5 }),
                    added("tests/moved.rs", 5),
                ],
            ),
            vec![edited("tests/a.rs", '2', Taken::WholeFile)],
        ),
        (
            "a binary frozen test replaced",
            judge(
                &tests(),
                "New fixture",
                &[file("tests/a.rs", Change::Modified, Lines::Binary)],
            ),
            vec![edited("tests/a.rs", '2', Taken::WholeFile)],
        ),
    ];

    for (name, got, want) in cases {
        assert_eq!(got, Verdict::Refused(want), "{name}");
    }
}

// Rule: a `TEST-FIX:` commit may take lines out of a frozen test when it changes test files only; the prefix is the
// start of the subject, in capitals.
#[test]
fn a_test_fix_commit_of_test_files_only_may_edit_a_frozen_test() {
    let cases: Vec<(&str, Verdict, Verdict)> = vec![
        (
            "TEST-FIX of test files only",
            judge(
                &tests(),
                "TEST-FIX: the case expected the wrong value",
                &[modified("tests/a.rs", 1, 1), added("tests/c.rs", 2)],
            ),
            Verdict::Allowed,
        ),
        (
            "TEST-FIX that also changes production code",
            judge(
                &tests(),
                "TEST-FIX: the case expected the wrong value",
                &[modified("tests/a.rs", 1, 1), modified("src/a.rs", 1, 0)],
            ),
            Verdict::Refused(vec![Refusal::TestFixTouchesOtherFiles {
                others: vec![path("src/a.rs")],
            }]),
        ),
        (
            "TEST-FIX that edits no frozen test may touch production code",
            judge(
                &tests(),
                "TEST-FIX: add a case",
                &[modified("src/a.rs", 1, 0), modified("tests/a.rs", 1, 0)],
            ),
            Verdict::Allowed,
        ),
        (
            "a prefix in another case is no TEST-FIX",
            judge(&tests(), "test-fix: the case", &[modified("tests/a.rs", 1, 1)]),
            Verdict::Refused(vec![edited("tests/a.rs", '2', Taken::Lines(1))]),
        ),
        (
            "the prefix is at the start of the subject",
            judge(&tests(), "Do a TEST-FIX: of the case", &[modified("tests/a.rs", 1, 1)]),
            Verdict::Refused(vec![edited("tests/a.rs", '2', Taken::Lines(1))]),
        ),
    ];

    for (name, got, want) in cases {
        assert_eq!(got, want, "{name}");
    }
}

/// A plan whose steps are 1.1 and 1.2; `with_for` gives them `For:` lines.
fn plan(with_for: bool) -> Plan {
    let for_line = if with_for { "    For: R1\n" } else { "" };
    crate::plan::parse_plan(&format!(
        "Do it.\n\n1. Work\n[x] 1.1 First\n{for_line}[ ] 1.2 Second\n{for_line}"
    ))
    .unwrap()
}

fn both_steps() -> Vec<ItemId> {
    vec!["1.1".parse().unwrap(), "1.2".parse().unwrap()]
}

// Rule: when the plan has `For:` lines, a commit message with no `Step:` trailer is refused.
#[test]
fn a_message_with_no_step_trailer_is_refused_when_the_plan_has_for_lines() {
    let verdict = judge_step(&plan(true), "Fix the parser\n\nIt read the line twice.\n");

    assert_eq!(
        verdict,
        StepVerdict::Refused(StepRefusal::Missing { steps: both_steps() })
    );
}

// Rule: a trailer names a step of the plan, or the commit is refused with what it named.
#[test]
fn a_trailer_that_names_no_step_of_the_plan_is_refused_with_what_it_names() {
    let cases: [(&str, &[&str]); 3] = [
        ("Fix it\n\nStep: 3.1\n", &["3.1"]),
        ("Fix it\n\nStep: banana\n", &["banana"]),
        ("Fix it\n\nStep: 1.1\nStep: 2.2\n", &["2.2"]),
    ];

    for (message, named) in cases {
        let want = StepVerdict::Refused(StepRefusal::NotInPlan {
            named: named.iter().map(|text| (*text).to_owned()).collect(),
            steps: both_steps(),
        });
        assert_eq!(judge_step(&plan(true), message), want, "{message:?}");
    }
}

// Rule: a commit that names a step of the plan in a `Step:` trailer is allowed, also when it is a `RED:`, a `TEST-FIX:`
// or a merge commit.
#[test]
fn a_trailer_that_names_a_step_of_the_plan_allows_the_commit() {
    let messages = [
        "Fix the parser\n\nIt read the line twice.\n\nStep: 1.2\n",
        "RED: parse a line\n\nStep: 1.1\n",
        "TEST-FIX: the case\n\nWrong value.\n\nStep: 1.1\n",
        "Merge branch 'other'\n\nStep: 1.2\n",
        "Fix it\n\nStep:   1.1  \n",
    ];

    for message in messages {
        assert_eq!(judge_step(&plan(true), message), StepVerdict::Allowed, "{message:?}");
    }
}

// Rule: only a line that starts with `Step:` is a trailer; a comment of the editor or the word inside a sentence is not.
#[test]
fn only_a_line_that_starts_with_step_is_a_trailer() {
    let messages = [
        "Fix it\n\nSee Step: 1.1 for why.\n",
        "Fix it\n# Step: 1.1\n",
        "Fix it\n\nstep: 1.1\n",
    ];

    for message in messages {
        assert_eq!(
            judge_step(&plan(true), message),
            StepVerdict::Refused(StepRefusal::Missing { steps: both_steps() }),
            "{message:?}"
        );
    }
}

// Rule: a plan with no `For:` line (a legacy plan) is not checked.
#[test]
fn a_plan_with_no_for_line_does_not_check_the_commit() {
    assert_eq!(judge_step(&plan(false), "Fix it\n"), StepVerdict::Allowed);
    assert_eq!(judge_step(&plan(false), "Fix it\n\nStep: 9.9\n"), StepVerdict::Allowed);
}

// Rule: the `Step:` trailer ends the message: a `Step:` line followed by more of the body is no trailer.
#[test]
fn a_step_line_that_the_body_goes_on_after_is_no_trailer() {
    let messages = [
        "Fix it\n\nStep: 1.1\n\nExplanation after the trailer.\n",
        "Fix it\n\nStep: 1.1\n\nMore of the body.\n",
        "Step: 1.1\n\nFix it\n",
    ];

    for message in messages {
        assert_eq!(
            judge_step(&plan(true), message),
            StepVerdict::Refused(StepRefusal::Missing { steps: both_steps() }),
            "{message:?}"
        );
    }
}

// Rule: the comment lines an editor leaves under a message, and blank lines, do not move the trailer off the end.
#[test]
fn comment_lines_and_blank_lines_after_the_trailer_leave_it_the_end_of_the_message() {
    let messages = [
        "Fix it\n\nStep: 1.1\n\n# Please enter the commit message.\n# Lines starting with '#' are ignored.\n",
        "Fix it\n\nStep: 1.1\n\n\n",
    ];

    for message in messages {
        assert_eq!(judge_step(&plan(true), message), StepVerdict::Allowed, "{message:?}");
    }
}

// Rule: the trailer block holds trailers only: a `Step:` line that the paragraph goes on after, with no blank line
// between, is part of the body.
#[test]
fn a_step_line_followed_by_body_text_in_the_same_paragraph_is_no_trailer() {
    let messages = [
        "Fix it\n\nStep: 1.1\nMore of the body.\n",
        "Fix it\n\nStep: 1.1\nMore of the body.\n\n# Please enter the commit message.\n",
    ];

    for message in messages {
        assert_eq!(
            judge_step(&plan(true), message),
            StepVerdict::Refused(StepRefusal::Missing { steps: both_steps() }),
            "{message:?}"
        );
    }
}

// Rule: other trailers beside `Step:`, and a folded value, stay in the trailer block.
#[test]
fn a_step_trailer_beside_other_trailers_allows_the_commit() {
    let messages = [
        "Fix it\n\nTest-First: not possible — a doc\nStep: 1.1\n",
        "Fix it\n\nStep: 1.1\nCo-authored-by: A B <a@example.com>\n",
        "Fix it\n\nSigned-off-by: A B <a@example.com>\nStep: 1.2\nNote: a long value\n  that goes on\n",
    ];

    for message in messages {
        assert_eq!(judge_step(&plan(true), message), StepVerdict::Allowed, "{message:?}");
    }
}

// Rule: what `git commit -v` shows under the scissors line, and the comment character of that line, are not the message.
#[test]
fn the_scissors_line_and_what_follows_it_are_not_part_of_the_message() {
    let messages = [
        "Fix it\n\nStep: 1.1\n\n# ------------------------ >8 ------------------------\n# Do not modify or remove the line above.\ndiff --git a/f b/f\n+x\n",
        "Fix it\n\nStep: 1.1\n\n; Please enter the commit message.\n; ------------------------ >8 ------------------------\n; Do not modify or remove the line above.\ndiff --git a/f b/f\n+x\n",
    ];

    for message in messages {
        assert_eq!(judge_step(&plan(true), message), StepVerdict::Allowed, "{message:?}");
    }
}
