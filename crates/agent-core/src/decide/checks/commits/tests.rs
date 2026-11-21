//! The findings about a commit: which issues are them, and which of them still need the human.
use super::*;
use crate::decide::checks::{Change, CommitFacts, FileChange, Lines, Touch};

fn id(digit: char) -> CommitId {
    digit.to_string().repeat(40).parse().unwrap()
}

fn path(text: &str) -> RepoPath {
    text.parse().unwrap()
}

fn finding(commit: char, file: &str) -> CommitFinding {
    CommitFinding {
        commit: id(commit),
        path: path(file),
        red: id('1'),
    }
}

fn edited(commit: char, file: &str) -> Issue {
    Issue::RedTestEdited {
        path: path(file),
        red: id('1'),
        edit: id(commit),
    }
}

fn commit(digit: char, parents: &str) -> CommitFacts {
    CommitFacts {
        id: id(digit),
        parents: parents.chars().map(id).collect(),
        subject: String::new(),
        body: String::new(),
        files: vec![FileChange {
            path: path("tests/a.rs"),
            change: Change::Modified,
            lines: Lines::Text { added: 1, removed: 1 },
        }],
        merge_files: Vec::new(),
    }
}

/// The round `1 <- 2 <- 3`, and `4`, the merge of the base's commit `9` into `3`.
fn facts() -> RoundFacts {
    RoundFacts {
        commits: vec![
            commit('1', ""),
            commit('2', "1"),
            commit('3', "2"),
            commit('9', ""),
            commit('4', "39"),
        ],
        ..RoundFacts::default()
    }
}

#[test]
fn only_the_red_test_edits_are_findings_about_a_commit() {
    let protected = Issue::ProtectedChanged {
        path: path("AGENTS.md"),
        change: Touch::Modified,
    };
    let issues = vec![
        protected.clone(),
        edited('2', "tests/a.rs"),
        Issue::NoTests,
        edited('3', "tests/b.rs"),
    ];

    let found = split(issues);

    assert_eq!(found.commits, [finding('2', "tests/a.rs"), finding('3', "tests/b.rs")]);
    assert_eq!(found.rest, [protected, Issue::NoTests]);
}

struct Row {
    name: &'static str,
    allowed: Vec<Subject>,
    refused: Vec<Subject>,
    paths: Vec<(&'static str, Option<char>)>,
    task_list: Vec<&'static str>,
    asks: bool,
    inherits: bool,
}

fn row(name: &'static str) -> Row {
    Row {
        name,
        allowed: Vec::new(),
        refused: Vec::new(),
        paths: Vec::new(),
        task_list: Vec::new(),
        asks: true,
        inherits: false,
    }
}

#[test]
fn a_finding_is_asked_unless_the_item_already_has_an_answer_for_its_commit_and_its_path() {
    let this = finding('2', "tests/a.rs").subject();
    let other_commit = finding('3', "tests/a.rs").subject();
    let other_path = finding('2', "tests/b.rs").subject();
    let rows = vec![
        row("nothing was said"),
        Row {
            allowed: vec![this.clone()],
            asks: false,
            ..row("the commit and the path were allowed")
        },
        Row {
            allowed: vec![other_commit.clone()],
            ..row("another commit of the same path was allowed")
        },
        Row {
            allowed: vec![other_path.clone()],
            ..row("another path of the same commit was allowed")
        },
        Row {
            allowed: vec![Subject::Path(path("tests/a.rs"))],
            ..row("the path alone was allowed for another finding")
        },
        Row {
            refused: vec![this.clone()],
            asks: false,
            ..row("the commit and the path were refused")
        },
        Row {
            refused: vec![other_commit],
            ..row("another commit was refused")
        },
        Row {
            task_list: vec!["tests/a.rs"],
            asks: false,
            ..row("the task list allows the path")
        },
        Row {
            task_list: vec!["tests/b.rs"],
            ..row("the task list allows another path")
        },
        Row {
            paths: vec![("tests/a.rs", None)],
            asks: false,
            inherits: true,
            ..row("the file was allowed before a head was recorded")
        },
        Row {
            paths: vec![("tests/a.rs", Some('3'))],
            asks: false,
            inherits: true,
            ..row("the file was allowed at a head that has the commit")
        },
        Row {
            paths: vec![("tests/a.rs", Some('2'))],
            asks: false,
            inherits: true,
            ..row("the file was allowed at the commit itself")
        },
        Row {
            paths: vec![("tests/a.rs", Some('1'))],
            ..row("the file was allowed at a head before the commit")
        },
        Row {
            paths: vec![("tests/b.rs", Some('3'))],
            ..row("another file was allowed at a later head")
        },
    ];
    for row in rows {
        let paths: BTreeMap<RepoPath, Option<CommitId>> = row
            .paths
            .iter()
            .map(|(file, head)| (path(file), head.map(id)))
            .collect();
        let task_list: Vec<TaskAllowance> = row
            .task_list
            .iter()
            .map(|pattern| TaskAllowance::parse(pattern).unwrap())
            .collect();
        let said = Said {
            allowed: &row.allowed,
            refused: &row.refused,
            paths: &paths,
            task_list: &task_list,
        };

        let settled = settle(vec![finding('2', "tests/a.rs")], &facts(), &said);

        let asked: Vec<CommitFinding> = if row.asks {
            vec![finding('2', "tests/a.rs")]
        } else {
            Vec::new()
        };
        let inherited: Vec<Subject> = if row.inherits { vec![this.clone()] } else { Vec::new() };
        assert_eq!(settled, Settled { ask: asked, inherited }, "{}", row.name);
    }
}

#[test]
fn what_was_allowed_still_holds_after_the_base_was_merged_into_the_round() {
    let allowed = [finding('2', "tests/a.rs").subject()];
    let said = Said {
        allowed: &allowed,
        refused: &[],
        paths: &BTreeMap::new(),
        task_list: &[],
    };
    let after_merge = facts();
    assert!(after_merge.commits.iter().any(|known| known.parents.len() == 2));

    let settled = settle(vec![finding('2', "tests/a.rs")], &after_merge, &said);

    assert_eq!(settled, Settled::default());
}

#[test]
fn a_file_allowed_at_a_head_before_a_merge_covers_the_commits_reached_through_it() {
    let paths = BTreeMap::from([(path("tests/a.rs"), Some(id('4')))]);
    let said = Said {
        allowed: &[],
        refused: &[],
        paths: &paths,
        task_list: &[],
    };

    let settled = settle(vec![finding('3', "tests/a.rs")], &facts(), &said);

    assert_eq!(settled.ask, []);
    assert_eq!(settled.inherited, [finding('3', "tests/a.rs").subject()]);
}

#[test]
fn only_the_findings_nobody_has_answered_are_asked_and_each_is_asked_once() {
    let allowed = [finding('2', "tests/a.rs").subject()];
    let refused = [finding('3', "tests/a.rs").subject()];
    let said = Said {
        allowed: &allowed,
        refused: &refused,
        paths: &BTreeMap::new(),
        task_list: &[],
    };

    let settled = settle(
        vec![
            finding('2', "tests/a.rs"),
            finding('3', "tests/a.rs"),
            finding('4', "tests/a.rs"),
        ],
        &facts(),
        &said,
    );

    assert_eq!(settled.ask, [finding('4', "tests/a.rs")]);
}
