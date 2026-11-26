//! The decision of a request of an agent with a plan: one row per outcome of [`decide`].
use super::*;
use std::num::NonZeroUsize;

use crate::plan::parse_plan;

/// A plan with a step still to do, one in hand, one done and one given up.
const PLAN: &str = "\
Publish the guide.

1. Draft
[ ] 1.1 List the questions a reader asks
    Check: reviewer
[>] 1.2 Collect the answers
[ ] 1.3 Write the walkthrough
    Files: docs/guide.md
    Check: exists docs/guide.md
[x] 1.4 Check the spelling
[-] 1.5 Translate it into German
    The human asked for English only.
2. Publish
[ ] 2.1 Put the guide in the release
";

/// The plan of the rows.
fn plan() -> Plan {
    parse_plan(PLAN).expect("the plan parses")
}

/// The steps of the rows.
fn id(text: &str) -> ItemId {
    text.parse().expect("a step ID")
}

/// A complete request for step 1.3 of the plan.
fn complete() -> Request {
    Request {
        named: Named {
            step: "1.3".to_owned(),
            files: vec!["docs/guide.md".to_owned(), "README.md".to_owned()],
            check: "exists docs/guide.md".to_owned(),
            doing: "  Write the walkthrough from the answers.  ".to_owned(),
        },
        refused: vec!["agent gate".to_owned(), "make gate".to_owned()],
    }
}

/// The rejection of `request` against the plan.
fn refused(request: &Request) -> Rejection {
    decide(&plan(), request).expect_err("the request is refused")
}

#[test]
fn a_complete_request_is_accepted_with_its_step_files_check_and_what_to_do() {
    let accepted = decide(&plan(), &complete()).expect("a complete request");

    assert_eq!(accepted.step.id(), id("1.3"));
    assert_eq!(accepted.step.title(), "Write the walkthrough");
    assert_eq!(
        accepted.files.iter().map(RepoPath::as_str).collect::<Vec<_>>(),
        ["docs/guide.md", "README.md"]
    );
    assert_eq!(accepted.check, Check::Exists("docs/guide.md".parse().expect("a path")));
    assert_eq!(accepted.doing, "Write the walkthrough from the answers.");
}

#[test]
fn a_request_names_a_step_that_can_be_sent() {
    // A step still to do and a step in hand can be sent; a done or given up step cannot.
    for (step, accepted) in [("1.1", true), ("1.2", true), ("1.4", false), ("1.5", false)] {
        let mut request = complete();
        request.named.step = step.to_owned();
        assert_eq!(decide(&plan(), &request).is_ok(), accepted, "for {step}");
    }
}

/// A row of the table of refusals: its name, how it spoils a complete request, and the rejection it must get.
type Row = (&'static str, fn(&mut Request), Rejection);

/// Every missing or wrong piece of a request, one row each.
fn refusal_rows() -> Vec<Row> {
    vec![
        ("", |r: &mut Request| r.named.step.clear(), Rejection::MissingStep),
        (
            "not a step",
            |r: &mut Request| r.named.step = "the first one".to_owned(),
            Rejection::BadStep {
                step: "the first one".to_owned(),
                source: "the first one".parse::<ItemId>().expect_err("no ID"),
            },
        ),
        (
            "no such step",
            |r: &mut Request| r.named.step = "2.9".to_owned(),
            Rejection::NoSuchStep {
                step: id("2.9"),
                steps: "1.1, 1.2, 1.3, 1.4, 1.5, 2.1".to_owned(),
            },
        ),
        (
            "done",
            |r: &mut Request| r.named.step = "1.4".to_owned(),
            Rejection::StepClosed {
                step: id("1.4"),
                state: "done",
            },
        ),
        (
            "given up",
            |r: &mut Request| r.named.step = "1.5".to_owned(),
            Rejection::StepClosed {
                step: id("1.5"),
                state: "given up",
            },
        ),
        (
            "no files",
            |r: &mut Request| r.named.files.clear(),
            Rejection::MissingFiles,
        ),
        (
            "an absolute path",
            |r: &mut Request| r.named.files = vec!["/etc/passwd".to_owned()],
            Rejection::BadFiles {
                path: "/etc/passwd".to_owned(),
                source: "/etc/passwd".parse::<RepoPath>().expect_err("no path"),
            },
        ),
        (
            "a path going up",
            |r: &mut Request| r.named.files = vec!["docs/../../secret".to_owned()],
            Rejection::BadFiles {
                path: "docs/../../secret".to_owned(),
                source: "docs/../../secret".parse::<RepoPath>().expect_err("no path"),
            },
        ),
        (
            "an empty path",
            |r: &mut Request| r.named.files = vec!["docs/guide.md".to_owned(), String::new()],
            Rejection::BadFiles {
                path: String::new(),
                source: String::new().parse::<RepoPath>().expect_err("no path"),
            },
        ),
        (
            "no check",
            |r: &mut Request| r.named.check.clear(),
            Rejection::MissingCheck,
        ),
        (
            "reviewer",
            |r: &mut Request| r.named.check = "reviewer".to_owned(),
            Rejection::CheckRefused {
                check: "reviewer".to_owned(),
            },
        ),
        (
            "the gate",
            |r: &mut Request| r.named.check = "agent gate --wait".to_owned(),
            Rejection::CheckRefused {
                check: "agent gate --wait".to_owned(),
            },
        ),
        (
            "exists without a path",
            |r: &mut Request| r.named.check = "exists".to_owned(),
            Rejection::BadCheck {
                check: "exists".to_owned(),
                source: PlanError::ExistsWithoutPath {
                    line: NonZeroUsize::MIN,
                },
            },
        ),
        (
            "nothing to do",
            |r: &mut Request| r.named.doing = "  \n ".to_owned(),
            Rejection::MissingDo,
        ),
    ]
}

#[test]
fn every_missing_or_wrong_piece_is_refused_with_its_own_outcome() {
    for (name, change, expected) in refusal_rows() {
        let mut request = complete();
        change(&mut request);
        assert_eq!(refused(&request), expected, "{name}");
    }
}

#[test]
fn a_command_check_is_read_as_a_step_reads_it() {
    let mut request = complete();
    request.named.check = "  cargo test -p guide  ".to_owned();

    let accepted = decide(&plan(), &request).expect("a command check");

    assert_eq!(accepted.check, Check::Command("cargo test -p guide".to_owned()));
}

/// The rejection of a complete request whose check is `check`, or `None` when it is accepted.
fn with_check(check: &str) -> Option<Rejection> {
    let mut request = complete();
    request.named.check = check.to_owned();
    decide(&plan(), &request).err()
}

#[test]
fn a_check_that_goes_on_as_a_sentence_is_refused_at_its_first_word_of_prose() {
    // The checks of issue #290, as the requester wrote them: a command, then words that say what it shows.
    let rows = [
        (
            "grep -n 'worker' docs/guide.md shows the line in the agent debug section",
            "shows",
        ),
        (
            "wc -l docs/guide.md gives 10, and jq -c . docs/guide.json exits 0",
            "gives",
        ),
        ("test -s docs/guide.md, then the guide is written", "docs/guide.md,"),
        ("jq -c . … docs/guide.json", "…"),
        ("grep -q walkthrough docs/guide.md and README.md", "and"),
    ];
    for (check, word) in rows {
        assert_eq!(
            with_check(check),
            Some(Rejection::SentenceCheck {
                check: check.to_owned(),
                word: word.to_owned(),
            }),
            "{check}"
        );
    }
}

#[test]
fn a_command_whose_words_are_quoted_options_or_paths_is_not_a_sentence() {
    for check in [
        "grep -q \"shows the line\" docs/guide.md",
        "grep -q 'and' docs/guide.md && test -s README.md",
        "grep -rq walkthrough crates docs prompts",
        "cut -d, -f1 docs/guide.md",
        "find docs -name '*.md' -and -newer README.md",
        "cargo test -p agent-core delegation",
        "grep -q the\\ guide docs/guide.md",
    ] {
        assert_eq!(with_check(check), None, "{check}");
    }
}

#[test]
fn the_refusal_of_a_sentence_says_a_check_is_one_command_and_how_to_keep_a_word() {
    let rejection = with_check("grep -n worker docs/guide.md shows the line").expect("refused");

    let text = rejection.to_string();
    assert!(text.contains("\"shows\""), "{text}");
    assert!(text.contains("one command"), "{text}");
    assert!(text.contains("quote"), "{text}");
}
