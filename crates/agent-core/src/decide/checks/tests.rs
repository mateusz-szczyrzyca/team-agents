//! The rules of the checks before a review, each as a decision: the case that raises it and the nearest ones that do not.

use super::markers::Marker;
use super::*;
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

/// Tests under `tests/` and in `*_test.go`; code under `src/` and `bin/`; `.github/` and `AGENTS.md` judge a round.
fn rules() -> Checks {
    Checks {
        test_paths: patterns(&["tests/", "*_test.go"]),
        code_paths: patterns(&["src/", "bin/"]),
        protected_paths: patterns(&[".github/", "AGENTS.md"]),
    }
}

fn path(text: &str) -> RepoPath {
    text.parse().unwrap()
}

/// A commit ID of forty copies of `digit`.
fn id(digit: char) -> CommitId {
    digit.to_string().repeat(40).parse().unwrap()
}

fn text(added: u32, removed: u32) -> Lines {
    Lines::Text { added, removed }
}

fn file(path_text: &str, change: Change, lines: Lines) -> FileChange {
    FileChange {
        path: path(path_text),
        change,
        lines,
    }
}

fn modified(path_text: &str, added: u32, removed: u32) -> FileChange {
    file(path_text, Change::Modified, text(added, removed))
}

fn added(path_text: &str, lines: u32) -> FileChange {
    file(path_text, Change::Added, text(lines, 0))
}

/// A commit named by a digit, with parents named by digits.
fn commit(digit: char, parents: &str, subject: &str, files: Vec<FileChange>) -> CommitFacts {
    CommitFacts {
        id: id(digit),
        parents: parents.chars().map(id).collect(),
        subject: subject.to_owned(),
        body: String::new(),
        files,
        merge_parents: Vec::new(),
    }
}

fn with_body(mut commit: CommitFacts, body: &str) -> CommitFacts {
    body.clone_into(&mut commit.body);
    commit
}

fn round(files: Vec<FileChange>) -> RoundFacts {
    RoundFacts {
        files,
        ..RoundFacts::default()
    }
}

fn history(commits: Vec<CommitFacts>) -> RoundFacts {
    RoundFacts {
        commits,
        ..RoundFacts::default()
    }
}

fn line(path_text: &str, number: u32, text: &str) -> AddedLine {
    AddedLine {
        path: path(path_text),
        number,
        text: text.to_owned(),
    }
}

fn found(facts: &RoundFacts) -> Vec<Issue> {
    assess(&rules(), facts, &[])
}

// ---- protected files ----------------------------------------------------------------------------------------------

// Rule: a protected file that changes in any way is an issue only the human may allow.
#[test]
fn a_protected_file_that_changes_is_an_issue_only_the_human_may_allow() {
    let issues = found(&round(vec![modified("AGENTS.md", 1, 0)]));

    assert_eq!(
        issues,
        [Issue::ProtectedChanged {
            path: path("AGENTS.md"),
            change: Touch::Modified
        }]
    );
    assert_eq!(issues[0].judge(), Judge::Human);
    assert_eq!(issues[0].subject(), Subject::Path(path("AGENTS.md")));
}

#[test]
fn a_protected_file_is_an_issue_whether_added_modified_deleted_or_renamed() {
    let cases = [
        (
            added(".github/workflows/ci.yml", 5),
            ".github/workflows/ci.yml",
            Touch::Added,
        ),
        (
            modified(".github/workflows/ci.yml", 1, 1),
            ".github/workflows/ci.yml",
            Touch::Modified,
        ),
        (
            file(".github/workflows/ci.yml", Change::Deleted, text(0, 5)),
            ".github/workflows/ci.yml",
            Touch::Deleted,
        ),
        // moved out of a protected place: the issue is about where it was
        (
            file(
                "docs/rules.md",
                Change::Renamed {
                    from: path("AGENTS.md"),
                },
                text(0, 0),
            ),
            "AGENTS.md",
            Touch::Renamed,
        ),
        // moved into a protected place: the issue is about where it is
        (
            file(
                "AGENTS.md",
                Change::Renamed {
                    from: path("docs/rules.md"),
                },
                text(0, 0),
            ),
            "AGENTS.md",
            Touch::Renamed,
        ),
    ];
    for (change, protected, touch) in cases {
        assert_eq!(
            found(&round(vec![change.clone()])),
            [Issue::ProtectedChanged {
                path: path(protected),
                change: touch
            }],
            "{change:?}"
        );
    }
}

#[test]
fn a_protected_name_is_protected_at_any_depth_and_its_neighbours_are_not() {
    assert_eq!(found(&round(vec![modified("docs/team/AGENTS.md", 1, 0)])).len(), 1);
    assert_eq!(found(&round(vec![modified("docs/AGENTS.md.bak", 1, 0)])), []);
    assert_eq!(found(&round(vec![modified("docs/readme.md", 1, 1)])), []);
}

#[test]
fn an_allowed_path_is_no_issue_and_another_one_still_is() {
    let facts = round(vec![
        modified("AGENTS.md", 1, 0),
        modified(".github/workflows/ci.yml", 1, 0),
    ]);

    let issues = assess(&rules(), &facts, &[Subject::Path(path("AGENTS.md"))]);

    assert_eq!(
        issues,
        [Issue::ProtectedChanged {
            path: path(".github/workflows/ci.yml"),
            change: Touch::Modified
        }]
    );
}

// ---- existing tests -------------------------------------------------------------------------------------------------

// Rule: an existing test file changed other than by added lines is an issue only the human may allow; adding lines,
// or a new file, is not.
#[test]
fn an_existing_test_file_with_lines_taken_out_is_an_issue_only_the_human_may_allow() {
    let issues = found(&round(vec![modified("tests/unit/a.bats", 3, 2)]));

    assert_eq!(
        issues,
        [Issue::TestChanged {
            path: path("tests/unit/a.bats"),
            how: TestChange::Edited { removed: 2 }
        }]
    );
    assert_eq!(issues[0].judge(), Judge::Human);
    assert_eq!(issues[0].subject(), Subject::Path(path("tests/unit/a.bats")));
}

#[test]
fn added_lines_a_new_test_file_and_an_unchanged_content_are_no_issue() {
    for change in [
        modified("tests/unit/a.bats", 4, 0),
        added("tests/unit/new.bats", 10),
        modified("tests/unit/a.bats", 0, 0),
        added("agentbus/new_test.go", 10),
    ] {
        assert_eq!(found(&round(vec![change.clone()])), [], "{change:?}");
    }
}

#[test]
fn a_deleted_renamed_or_replaced_test_file_is_an_issue() {
    let cases = [
        (
            file("agentbus/a_test.go", Change::Deleted, text(0, 30)),
            Issue::TestChanged {
                path: path("agentbus/a_test.go"),
                how: TestChange::Deleted,
            },
        ),
        (
            file(
                "agentbus/b_test.go",
                Change::Renamed {
                    from: path("agentbus/a_test.go"),
                },
                text(0, 0),
            ),
            Issue::TestChanged {
                path: path("agentbus/a_test.go"),
                how: TestChange::Renamed {
                    to: path("agentbus/b_test.go"),
                },
            },
        ),
        (
            file("tests/fixture.bin", Change::Modified, Lines::Binary),
            Issue::TestChanged {
                path: path("tests/fixture.bin"),
                how: TestChange::EditedBinary,
            },
        ),
    ];
    for (change, issue) in cases {
        assert_eq!(found(&round(vec![change.clone()])), [issue], "{change:?}");
    }
}

#[test]
fn a_file_that_is_not_a_test_may_lose_lines() {
    assert_eq!(found(&round(vec![modified("docs/notes.md", 0, 8)])), []);
    assert_eq!(
        found(&round(vec![modified("src/lib.rs", 3, 3), added("tests/new.rs", 4)])),
        []
    );
}

#[test]
fn a_protected_test_file_is_reported_as_protected_only() {
    let facts = round(vec![modified("tests/AGENTS.md", 0, 1)]);

    assert_eq!(
        found(&facts),
        [Issue::ProtectedChanged {
            path: path("tests/AGENTS.md"),
            change: Touch::Modified
        }]
    );
}

#[test]
fn an_allowed_test_file_is_no_issue() {
    let facts = round(vec![modified("tests/a.rs", 0, 2)]);

    assert_eq!(assess(&rules(), &facts, &[Subject::Path(path("tests/a.rs"))]), []);
}

// ---- RED tests that are edited later --------------------------------------------------------------------------------

/// A `RED:` commit `1` that adds `tests/a.rs` and a later commit `2` with `subject` that changes `files`.
fn red_then(subject: &str, files: Vec<FileChange>) -> RoundFacts {
    history(vec![
        commit('1', "", "RED: test a", vec![added("tests/a.rs", 10)]),
        commit('2', "1", subject, files),
    ])
}

// Rule: a later commit that removes or replaces lines of a test file a `RED:` commit touched is an issue the team
// fixes (TDD-14); one that only adds lines is not.
#[test]
fn a_later_commit_that_takes_lines_out_of_a_red_test_is_an_issue_for_the_team() {
    let issues = found(&red_then("GREEN: code a", vec![modified("tests/a.rs", 1, 2)]));

    assert_eq!(
        issues,
        [Issue::RedTestEdited {
            path: path("tests/a.rs"),
            red: id('1'),
            edit: id('2')
        }]
    );
    assert_eq!(issues[0].judge(), Judge::Team);
    assert_eq!(issues[0].subject(), Subject::Path(path("tests/a.rs")));
}

#[test]
fn a_later_commit_that_only_adds_lines_to_a_red_test_is_no_issue() {
    assert_eq!(
        found(&red_then("GREEN: code a", vec![modified("tests/a.rs", 4, 0)])),
        []
    );
}

#[test]
fn a_later_commit_that_deletes_a_red_test_is_an_issue() {
    let issues = found(&red_then(
        "GREEN: drop a",
        vec![file("tests/a.rs", Change::Deleted, text(0, 10))],
    ));

    assert_eq!(issues.len(), 1);
}

#[test]
fn a_test_file_the_red_commit_did_not_touch_may_be_edited() {
    let facts = history(vec![
        commit('1', "", "RED: test a", vec![added("tests/a.rs", 10)]),
        commit('2', "1", "GREEN: code a", vec![modified("tests/other.rs", 1, 1)]),
    ]);

    assert_eq!(found(&facts), []);
}

#[test]
fn a_red_commit_that_touches_no_test_file_protects_nothing() {
    let facts = history(vec![
        commit('1', "", "RED: stub", vec![added("src/lib.rs", 3)]),
        commit(
            '2',
            "1",
            "GREEN: code",
            vec![modified("src/lib.rs", 3, 3), modified("tests/a.rs", 0, 1)],
        ),
    ]);

    assert_eq!(found(&facts), []);
}

#[test]
fn a_commit_that_is_not_after_the_red_commit_is_no_issue() {
    // two branches of members that meet in a merge: neither is later than the other
    let facts = history(vec![
        commit('1', "", "RED: test a", vec![added("tests/a.rs", 10)]),
        commit('2', "", "Member work", vec![modified("tests/a.rs", 0, 3)]),
        commit('3', "12", "Merge", vec![]),
    ]);

    assert_eq!(found(&facts), []);
}

#[test]
fn a_commit_after_a_merge_of_the_red_commit_is_later_than_it() {
    let facts = history(vec![
        commit('1', "", "RED: test a", vec![added("tests/a.rs", 10)]),
        commit('2', "", "Member work", vec![added("src/b.rs", 5)]),
        commit('3', "12", "Merge", vec![]),
        commit('4', "3", "GREEN: code", vec![modified("tests/a.rs", 0, 1)]),
    ]);

    assert_eq!(
        found(&facts),
        [Issue::RedTestEdited {
            path: path("tests/a.rs"),
            red: id('1'),
            edit: id('4')
        }]
    );
}

#[test]
fn a_test_file_two_red_commits_touched_is_one_issue_per_edit() {
    let facts = history(vec![
        commit('1', "", "RED: test a", vec![added("tests/a.rs", 10)]),
        commit('2', "1", "RED: more of a", vec![modified("tests/a.rs", 5, 0)]),
        commit('3', "2", "GREEN: code", vec![modified("tests/a.rs", 0, 1)]),
    ]);

    assert_eq!(found(&facts).len(), 1);
}

/// The description of a pull request with the section of the tests changed after RED, naming `named`.
fn describing(named: &str) -> String {
    format!(
        "Work of the team.\n\n## Tests changed after RED\n\n- {named}: the assertion was wrong\n\n## Gate\n\nPassed.\n"
    )
}

/// The facts of a `TEST-FIX:` commit `2` of `tests/a.rs` after the `RED:` commit, with `description`.
fn test_fix(subject: &str, files: Vec<FileChange>, description: &str) -> RoundFacts {
    RoundFacts {
        description: description.to_owned(),
        ..red_then(subject, files)
    }
}

// Rule: a `TEST-FIX:` commit that touches test files only and is named under `## Tests changed after RED` may correct
// a test; every other way to edit it is an issue.
#[test]
fn a_test_fix_that_the_description_names_may_edit_a_red_test() {
    // the commit's ID is forty `2`: an abbreviation is the same digit, as many times
    let full = id('2').to_string();
    for named in [
        full.clone(),
        "2".repeat(7),
        "2".repeat(12),
        format!("`{}`", "2".repeat(7)),
        format!("https://github.com/o/r/commit/{full}"),
    ] {
        let facts = test_fix(
            "TEST-FIX: the assertion was wrong",
            vec![modified("tests/a.rs", 1, 1)],
            &describing(&named),
        );

        assert_eq!(found(&facts), [], "named as {named}");
    }
}

#[test]
fn a_test_fix_that_the_description_does_not_name_is_an_issue() {
    let cases = [
        String::new(),
        "Work of the team.\n".to_owned(),
        describing(&id('3').to_string()),
        // too short to name a commit
        describing(&"2".repeat(6)),
        // named, but not under the heading
        format!("## Notes\n\n- {}\n", id('2')),
        // the heading is of another section
        format!("## Tests changed after RED\n\n## Gate\n\n{}\n", id('2')),
    ];
    for description in cases {
        let facts = test_fix(
            "TEST-FIX: the assertion was wrong",
            vec![modified("tests/a.rs", 1, 1)],
            &description,
        );

        assert_eq!(found(&facts).len(), 1, "{description:?}");
    }
}

#[test]
fn a_commit_that_is_not_a_test_fix_is_not_excused_by_being_named() {
    let description = describing(&id('2').to_string());
    let cases = [
        // the subject does not start with the marker
        ("Fix the test", vec![modified("tests/a.rs", 1, 1)]),
        ("TEST-FIX the assertion", vec![modified("tests/a.rs", 1, 1)]),
        ("test-fix: the assertion", vec![modified("tests/a.rs", 1, 1)]),
        // it touches more than test files
        (
            "TEST-FIX: and more",
            vec![modified("tests/a.rs", 1, 1), modified("src/lib.rs", 1, 0)],
        ),
    ];
    for (subject, files) in cases {
        let facts = test_fix(subject, files, &description);

        assert_eq!(found(&facts).len(), 1, "{subject}");
    }
}

// ---- production code without tests ------------------------------------------------------------------------------------

// Rule: a round that changes production code adds a line to a test file, or one of its commits says why not.
#[test]
fn a_round_that_changes_code_and_adds_no_test_line_is_an_issue_for_the_team() {
    let issues = found(&round(vec![modified("src/lib.rs", 5, 1)]));

    assert_eq!(issues, [Issue::NoTests]);
    assert_eq!(issues[0].judge(), Judge::Team);
    assert_eq!(issues[0].subject(), Subject::NoTests);
}

#[test]
fn a_round_that_adds_its_tests_is_no_issue() {
    for tests in [
        added("tests/new.rs", 3),
        modified("agentbus/a_test.go", 2, 0),
        modified("tests/a.rs", 2, 0),
    ] {
        assert_eq!(
            found(&round(vec![modified("src/lib.rs", 5, 1), tests.clone()])),
            [],
            "{tests:?}"
        );
    }
}

#[test]
fn a_test_file_that_only_loses_lines_adds_no_test() {
    let facts = round(vec![modified("src/lib.rs", 5, 1), modified("tests/a.rs", 0, 2)]);

    let issues = found(&facts);

    assert!(issues.contains(&Issue::NoTests), "{issues:?}");
}

#[test]
fn a_round_that_changes_no_code_needs_no_test() {
    for files in [
        vec![modified("docs/notes.md", 5, 0)],
        vec![added("tests/new.rs", 3)],
        vec![modified("README.md", 1, 1), modified("config/agents.toml", 2, 2)],
        vec![],
    ] {
        assert_eq!(found(&round(files.clone())), [], "{files:?}");
    }
}

#[test]
fn deleting_or_renaming_code_is_a_change_of_code() {
    let deleted = round(vec![file("src/old.rs", Change::Deleted, text(0, 9))]);
    let renamed = round(vec![file(
        "docs/old.rs",
        Change::Renamed {
            from: path("src/old.rs"),
        },
        text(0, 0),
    )]);

    assert_eq!(found(&deleted), [Issue::NoTests]);
    assert_eq!(found(&renamed), [Issue::NoTests]);
}

#[test]
fn a_file_that_is_both_a_test_and_code_is_a_test() {
    let rules = Checks {
        test_paths: patterns(&["src/**/tests.rs"]),
        code_paths: patterns(&["src/"]),
        protected_paths: patterns(&[]),
    };

    assert_eq!(assess(&rules, &round(vec![modified("src/a/tests.rs", 3, 0)]), &[]), []);
}

// Rule: a commit that says `Test-First: not possible — <reason>` excuses a round without new test lines.
#[test]
fn a_commit_that_says_why_there_is_no_test_excuses_the_round() {
    let code = vec![modified("src/lib.rs", 5, 1)];
    for message in [
        "Test-First: not possible — layout only",
        "Why.\n\nTest-First: not possible — the change is a line of configuration\n\nAgent: x",
        "Test-First: not possible - ascii dash",
        "Test-First: not possible – en dash",
        "   Test-First: not possible — indented",
    ] {
        let mut facts = history(vec![with_body(commit('1', "", "Change", code.clone()), message)]);
        facts.files.clone_from(&code);

        assert_eq!(found(&facts), [], "{message}");
    }
}

#[test]
fn the_line_of_a_subject_counts_as_well_as_the_body() {
    let mut facts = history(vec![commit(
        '1',
        "",
        "Test-First: not possible — a refactoring",
        vec![],
    )]);
    facts.files = vec![modified("src/lib.rs", 5, 1)];

    assert_eq!(found(&facts), []);
}

#[test]
fn a_line_without_a_reason_or_in_other_words_excuses_nothing() {
    for message in [
        "Test-First: not possible",
        "Test-First: not possible —",
        "Test-First: not possible —   ",
        "Test-First: possible — we did",
        "test-first: not possible — lowercase",
        "Not a Test-First: not possible — mid-line",
        "Test-First not possible — no colon",
    ] {
        let mut facts = history(vec![with_body(commit('1', "", "Change", vec![]), message)]);
        facts.files = vec![modified("src/lib.rs", 5, 1)];

        assert_eq!(found(&facts), [Issue::NoTests], "{message}");
    }
}

#[test]
fn an_allowed_missing_test_is_no_issue() {
    let facts = round(vec![modified("src/lib.rs", 5, 1)]);

    assert_eq!(assess(&rules(), &facts, &[Subject::NoTests]), []);
}

// ---- markers ----------------------------------------------------------------------------------------------------------

fn marked(files: &[&str], lines: Vec<AddedLine>, description: &str) -> RoundFacts {
    RoundFacts {
        files: files
            .iter()
            .map(|name| modified(name, 1, 0))
            .chain([added("tests/new.rs", 1)])
            .collect(),
        added: lines,
        description: description.to_owned(),
        ..RoundFacts::default()
    }
}

// Rule: an added line that skips a test, suppresses a lint or asks whether the code runs under test is an issue the
// team fixes, unless the description names its file.
#[test]
fn an_added_line_that_skips_a_test_is_an_issue_for_the_team() {
    let facts = marked(&[], vec![line("tests/new.rs", 7, "#[ignore]")], "");

    let issues = found(&facts);

    assert_eq!(
        issues,
        [Issue::Marker {
            path: path("tests/new.rs"),
            marker: Marker::SkipsTest,
            line: 7,
            count: 1
        }]
    );
    assert_eq!(issues[0].judge(), Judge::Team);
    assert_eq!(issues[0].subject(), Subject::Path(path("tests/new.rs")));
}

#[test]
fn an_added_line_that_suppresses_a_lint_is_an_issue_in_any_file_of_a_scanned_language() {
    let facts = marked(&["src/lib.rs"], vec![line("src/lib.rs", 3, "#[allow(dead_code)]")], "");

    assert_eq!(
        found(&facts),
        [Issue::Marker {
            path: path("src/lib.rs"),
            marker: Marker::SuppressesLint,
            line: 3,
            count: 1
        }]
    );
}

#[test]
fn the_lines_of_one_file_and_kind_are_one_issue_with_the_first_line_and_the_count() {
    let facts = marked(
        &["src/lib.rs"],
        vec![
            line("src/lib.rs", 9, "#[allow(dead_code)]"),
            line("src/lib.rs", 4, "let x = 1;"),
            line("src/lib.rs", 20, "#[allow(unused)]"),
            line("src/lib.rs", 30, "if cfg!(test) {"),
        ],
        "",
    );

    assert_eq!(
        found(&facts),
        [
            Issue::Marker {
                path: path("src/lib.rs"),
                marker: Marker::SuppressesLint,
                line: 9,
                count: 2
            },
            Issue::Marker {
                path: path("src/lib.rs"),
                marker: Marker::ChecksUnderTest,
                line: 30,
                count: 1
            },
        ]
    );
}

#[test]
fn a_file_the_description_names_may_have_markers_and_another_file_still_may_not() {
    let facts = marked(
        &["src/lib.rs", "src/other.rs"],
        vec![
            line("src/lib.rs", 3, "#[allow(dead_code)]"),
            line("src/other.rs", 5, "#[allow(unused)]"),
        ],
        "Suppressions: `src/lib.rs` keeps an allow until the next item wires it.",
    );

    assert_eq!(
        found(&facts),
        [Issue::Marker {
            path: path("src/other.rs"),
            marker: Marker::SuppressesLint,
            line: 5,
            count: 1
        }]
    );
}

#[test]
fn a_file_with_an_allowed_subject_may_have_markers() {
    let facts = marked(&["src/lib.rs"], vec![line("src/lib.rs", 3, "#[allow(dead_code)]")], "");

    assert_eq!(assess(&rules(), &facts, &[Subject::Path(path("src/lib.rs"))]), []);
}

// Rule: only production code is asked whether it runs under test; a test, or a file that is neither, may.
#[test]
fn only_production_code_may_not_ask_whether_it_runs_under_test() {
    let facts = marked(
        &["src/lib.rs"],
        vec![
            line("src/lib.rs", 1, "if cfg!(test) {"),
            line("tests/new.rs", 2, "if cfg!(test) {"),
            line("tools/helper.rs", 3, "if cfg!(test) {"),
        ],
        "",
    );

    assert_eq!(
        found(&facts),
        [Issue::Marker {
            path: path("src/lib.rs"),
            marker: Marker::ChecksUnderTest,
            line: 1,
            count: 1
        }]
    );
}

#[test]
fn lines_of_a_language_without_markers_are_no_issue() {
    let facts = marked(&[], vec![line("docs/notes.md", 1, "#[ignore] //nolint")], "");

    assert_eq!(found(&facts), []);
}

// ---- order ------------------------------------------------------------------------------------------------------------

// Rule: the issues come in the order of the rules, so that the human's question and the team's message are stable.
#[test]
fn the_issues_come_in_the_order_of_the_rules() {
    let facts = RoundFacts {
        files: vec![
            modified("src/lib.rs", 5, 1),
            modified("AGENTS.md", 1, 0),
            modified("tests/b.rs", 0, 3),
        ],
        commits: vec![
            commit('1', "", "RED: a", vec![added("tests/a.rs", 4)]),
            commit('2', "1", "GREEN: a", vec![modified("tests/a.rs", 0, 1)]),
        ],
        added: vec![line("src/lib.rs", 2, "#[allow(dead_code)]")],
        description: String::new(),
    };

    let kinds: Vec<&str> = found(&facts)
        .iter()
        .map(|issue| match issue {
            Issue::ProtectedChanged { .. } => "protected",
            Issue::TestChanged { .. } => "test changed",
            Issue::RedTestEdited { .. } => "red edited",
            Issue::NoTests => "no tests",
            Issue::Marker { .. } => "marker",
        })
        .collect();

    assert_eq!(kinds, ["protected", "test changed", "red edited", "no tests", "marker"]);
}

#[test]
fn a_round_with_nothing_changed_has_no_issue() {
    assert_eq!(found(&RoundFacts::default()), []);
}

// Rule: a subject is serialized as it is recorded for an item: a path as an object, the lack of tests as a word.
#[test]
fn a_subject_is_written_as_json_and_read_back() {
    let subjects = [Subject::Path(path("tests/a.rs")), Subject::NoTests];

    let json = serde_json::to_string(&subjects).unwrap();

    assert_eq!(json, r#"[{"path":"tests/a.rs"},"no_tests"]"#);
    assert_eq!(serde_json::from_str::<Vec<Subject>>(&json).unwrap(), subjects);
}

/// A merge with the edits against both parents kept separately.
fn merging(subject: &str, first: Vec<FileChange>, second: Vec<FileChange>) -> CommitFacts {
    let mut merge = commit('3', "12", subject, vec![]);
    merge.merge_parents = vec![
        ParentChanges {
            parent: id('1'),
            files: first,
        },
        ParentChanges {
            parent: id('2'),
            files: second,
        },
    ];
    merge
}

#[test]
fn a_merge_cannot_remove_a_red_test_from_either_parent() {
    for red_parent in ['1', '2'] {
        let red = commit(red_parent, "0", "RED: freeze", vec![added("tests/a.rs", 2)]);
        let other_parent = if red_parent == '1' { '2' } else { '1' };
        let other = commit(other_parent, "0", "Other branch", vec![]);
        let mut merge = merging("Merge", vec![], vec![]);
        for parent in &mut merge.merge_parents {
            parent.files = if parent.parent == red.id {
                vec![modified("tests/a.rs", 0, 1)]
            } else {
                vec![added("tests/a.rs", 1)]
            };
        }
        let facts = history(vec![red.clone(), other, merge]);
        assert_eq!(
            found(&facts),
            [Issue::RedTestEdited {
                path: path("tests/a.rs"),
                red: red.id,
                edit: id('3'),
            }]
        );
    }
}

#[test]
fn importing_red_work_does_not_compare_it_to_an_unrelated_parent() {
    let facts = history(vec![
        commit(
            '1',
            "0",
            "RED: replace before freezing",
            vec![modified("tests/a.rs", 1, 2)],
        ),
        commit('2', "0", "Other branch", vec![]),
        merging("Merge", vec![], vec![modified("tests/a.rs", 1, 2)]),
    ]);
    assert_eq!(found(&facts), []);
}

#[test]
fn a_named_test_fix_merge_must_touch_only_test_files_against_every_parent() {
    for production in [false, true] {
        let mut second = vec![added("tests/a.rs", 1)];
        if production {
            second.push(added("src/lib.rs", 1));
        }
        let mut facts = history(vec![
            commit('1', "0", "RED: freeze", vec![added("tests/a.rs", 2)]),
            commit('2', "0", "Other branch", vec![]),
            merging(
                "TEST-FIX: correct assertion",
                vec![modified("tests/a.rs", 0, 1)],
                second,
            ),
        ]);
        facts.description = describing(id('3').as_str());
        let expected = if production {
            vec![Issue::RedTestEdited {
                path: path("tests/a.rs"),
                red: id('1'),
                edit: id('3'),
            }]
        } else {
            vec![]
        };
        assert_eq!(found(&facts), expected);
    }
}

#[test]
fn a_merge_checks_the_parent_that_inherits_red_through_other_commits() {
    let mut merge = merging("Merge", vec![modified("tests/a.rs", 0, 1)], vec![]);
    merge.parents = vec![id('4'), id('2')];
    merge.merge_parents[0].parent = id('4');
    let facts = history(vec![
        commit('1', "0", "RED: freeze", vec![added("tests/a.rs", 2)]),
        commit('2', "0", "Other branch", vec![]),
        commit('4', "1", "GREEN: implementation", vec![added("src/lib.rs", 1)]),
        merge,
    ]);
    assert_eq!(
        found(&facts),
        [Issue::RedTestEdited {
            path: path("tests/a.rs"),
            red: id('1'),
            edit: id('3'),
        }]
    );
}

#[test]
fn a_red_merge_freezes_tests_for_later_commits() {
    let facts = history(vec![
        commit('1', "0", "First parent", vec![]),
        commit('2', "0", "Second parent", vec![]),
        merging("RED: freeze at merge", vec![added("tests/a.rs", 2)], vec![]),
        commit('4', "3", "GREEN: remove assertion", vec![modified("tests/a.rs", 0, 1)]),
    ]);
    assert_eq!(
        found(&facts),
        [Issue::RedTestEdited {
            path: path("tests/a.rs"),
            red: id('3'),
            edit: id('4'),
        }]
    );
}
