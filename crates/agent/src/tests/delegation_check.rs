//! `agent _delegation-check`: the check the bus asks of a request to a worker, against the requester's plan.
//!
//! The bus reads no worktree and has no plan reader: it runs this hidden command, which reads the plan file at the
//! moment of the request and the `[plan] check_refuse` of the home's configuration, and prints what the bus takes:
//! `ACCEPTED`, then the step in full; or `REFUSED <keyword>`, then a line that says what to add; or `NO-PLAN` for an
//! agent without a plan file, which may send a request in the old form.

use std::ffi::OsString;
use std::process::ExitCode;

use super::*;

/// The plan a member's worktree holds in the tests.
const PLAN: &str = "\
Publish the guide.

1. Draft
[ ] 1.1 List the questions a reader asks
    Check: reviewer
[>] 1.2 Write the walkthrough
    Files: docs/guide.md
    These words are part of the step.
    Check: exists docs/guide.md
[x] 1.3 Check the spelling
";

/// A home in a temporary directory whose `agents.toml` is the example configuration, and the plan file in it.
fn home_with_plan() -> (tempfile::TempDir, std::path::PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    std::fs::write(
        root.join("agents.toml"),
        include_str!("../../../../config/agents.example.toml"),
    )
    .unwrap();
    let plan = root.join("plan.md");
    std::fs::write(&plan, PLAN).unwrap();
    (dir, plan)
}

/// Runs `agent _delegation-check --plan PLAN --for STEP --files FILES --check CHECK --do DO`, and returns its status
/// and its output.
fn check(plan: &std::path::Path, step: &str, files: &str, check: &str, doing: &str) -> (ExitCode, String) {
    let dir = tempfile::tempdir().unwrap();
    let environment = Environment::new(Some(OsString::from(plan.parent().unwrap_or(dir.path()))));
    let words = [
        "agent",
        "_delegation-check",
        "--plan",
        plan.to_str().unwrap(),
        "--for",
        step,
        "--files",
        files,
        "--check",
        check,
        "--do",
        doing,
    ];
    let mut out = Vec::new();
    let mut err = Vec::new();
    let code = run_in(&environment, words, &mut out, &mut err).unwrap();
    assert!(err.is_empty(), "{}", String::from_utf8_lossy(&err));
    (code, String::from_utf8(out).unwrap())
}

#[test]
fn a_complete_request_prints_the_step_in_full_with_its_files_check_and_what_to_do() {
    let (_dir, plan) = home_with_plan();

    let (code, out) = check(
        &plan,
        "1.2",
        "docs/guide.md,README.md",
        "exists docs/guide.md",
        "Write the walkthrough from the answers.",
    );

    assert_eq!(code, ExitCode::SUCCESS);
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(
        lines,
        [
            "ACCEPTED",
            "The step of the requester's plan:",
            "Step 1.2 Write the walkthrough",
            "These words are part of the step.",
            "Files: docs/guide.md",
            "Check: exists docs/guide.md",
            "Request files: docs/guide.md, README.md",
            "Request check: exists docs/guide.md",
            "Do: Write the walkthrough from the answers.",
        ]
    );
}

#[test]
fn a_missing_piece_is_refused_with_its_keyword_and_what_to_add() {
    let (_dir, plan) = home_with_plan();
    let rows = [
        ("MISSING-STEP", "", "docs/guide.md", "exists docs/guide.md", "x"),
        ("MISSING-FILES", "1.2", "", "exists docs/guide.md", "x"),
        ("MISSING-CHECK", "1.2", "docs/guide.md", "", "x"),
        ("MISSING-DO", "1.2", "docs/guide.md", "exists docs/guide.md", " "),
    ];
    for (keyword, step, files, check_line, doing) in rows {
        let (code, out) = check(&plan, step, files, check_line, doing);
        assert_eq!(code, ExitCode::SUCCESS, "{keyword}");
        let mut lines = out.lines();
        assert_eq!(lines.next(), Some(format!("REFUSED {keyword}").as_str()), "{out}");
        assert!(lines.next().is_some_and(|line| !line.trim().is_empty()), "{out}");
    }
}

#[test]
fn every_refusal_has_its_own_keyword() {
    let (_dir, plan) = home_with_plan();
    let rows = [
        ("NO-SUCH-STEP", "9.9", "docs/guide.md", "exists docs/guide.md"),
        ("STEP-CLOSED", "1.3", "docs/guide.md", "exists docs/guide.md"),
        ("PATH-OUTSIDE", "1.2", "/etc/passwd", "exists docs/guide.md"),
        ("CHECK-REFUSED", "1.2", "docs/guide.md", "reviewer"),
        ("CHECK-REFUSED", "1.2", "docs/guide.md", "make gate"),
        ("CHECK-REFUSED", "1.2", "docs/guide.md", "agent gate --wait"),
    ];
    for (keyword, step, files, check_line) in rows {
        let (_, out) = check(&plan, step, files, check_line, "x");
        assert_eq!(
            out.lines().next(),
            Some(format!("REFUSED {keyword}").as_str()),
            "{step} {files} {check_line}: {out}"
        );
    }
}

#[test]
fn a_plan_that_is_not_there_lets_the_old_form_through() {
    let (_dir, plan) = home_with_plan();
    let missing = plan.with_file_name("absent.md");

    let (code, out) = check(&missing, "1.2", "docs/guide.md", "exists docs/guide.md", "x");

    assert_eq!(code, ExitCode::SUCCESS);
    assert_eq!(out.lines().next(), Some("NO-PLAN"));
}

#[test]
fn a_plan_that_does_not_read_is_refused() {
    let (_dir, plan) = home_with_plan();
    std::fs::write(&plan, "1. Draft\n[?] 1.1 Not a mark\n").unwrap();

    let (_, out) = check(&plan, "1.2", "docs/guide.md", "exists docs/guide.md", "x");

    assert_eq!(out.lines().next(), Some("REFUSED PLAN-UNREADABLE"));
}

#[test]
fn a_value_that_starts_with_a_hyphen_is_taken_as_the_value() {
    let (_dir, plan) = home_with_plan();

    let (code, out) = check(&plan, "1.2", "-guide.md", "exists docs/guide.md", "-Write it.");

    assert_eq!(code, ExitCode::SUCCESS);
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines.first(), Some(&"ACCEPTED"), "{out}");
    assert!(lines.contains(&"Request files: -guide.md"), "{out}");
    assert!(lines.contains(&"Do: -Write it."), "{out}");
}

#[test]
fn an_empty_entry_in_the_files_is_refused_as_no_path() {
    let (_dir, plan) = home_with_plan();

    for files in ["README.md,,docs/guide.md", "README.md,", ", docs/guide.md"] {
        let (_, out) = check(&plan, "1.2", files, "exists docs/guide.md", "x");
        assert_eq!(out.lines().next(), Some("REFUSED PATH-OUTSIDE"), "{files}: {out}");
    }
}

#[test]
fn a_check_of_more_than_one_line_is_refused() {
    let (_dir, plan) = home_with_plan();

    for check_line in ["true\nreviewer", "true\r\nfalse", "exists docs/guide.md\nmake gate"] {
        let (_, out) = check(&plan, "1.2", "docs/guide.md", check_line, "x");
        assert_eq!(
            out.lines().next(),
            Some("REFUSED CHECK-REFUSED"),
            "{check_line:?}: {out}"
        );
    }
}

#[test]
fn a_refusal_says_the_cause_of_what_does_not_read() {
    let (_dir, plan) = home_with_plan();
    let path_cause = "/etc/passwd"
        .parse::<agent_core::repo_path::RepoPath>()
        .unwrap_err()
        .to_string();

    let (_, outside) = check(&plan, "1.2", "/etc/passwd", "exists docs/guide.md", "x");
    std::fs::write(&plan, "1. Draft\n[?] 1.1 Not a mark\n").unwrap();
    let plan_cause = agent_core::plan::parse_plan("1. Draft\n[?] 1.1 Not a mark\n")
        .unwrap_err()
        .to_string();
    let (_, unreadable) = check(&plan, "1.1", "docs/guide.md", "exists docs/guide.md", "x");

    assert!(outside.contains(&path_cause), "{path_cause:?} is not in {outside:?}");
    assert!(
        unreadable.contains(&plan_cause),
        "{plan_cause:?} is not in {unreadable:?}"
    );
}
