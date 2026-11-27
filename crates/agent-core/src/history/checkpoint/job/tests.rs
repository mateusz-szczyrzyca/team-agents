//! The checkpoints of a worker's request, a review and an investigation: their job and causes, and the three parts.

use std::num::NonZeroU32;
use std::str::FromStr;

use super::{CheckResult, JobWork, ReviewedItem, investigation_meant, job_content, request_meant, review_meant};
use crate::history::{CheckpointCause, CheckpointJob, CommitLine, HistoryEvent};
use crate::state::PrNumber;
use crate::task_list::requirements::Requirement;

fn commit(id: &str) -> CommitLine {
    CommitLine {
        id: id.to_owned(),
        step: None,
    }
}

fn requirement(id: &str, text: &str) -> Requirement {
    Requirement {
        id: id.parse().unwrap(),
        text: text.to_owned(),
    }
}

fn pr() -> PrNumber {
    PrNumber::new(NonZeroU32::new(7).unwrap())
}

#[test]
fn a_record_with_no_job_reads_as_an_item() {
    let json = r#"{"event":"checkpoint","round":"round-20261126-180909-vet6","task":"task-20261126-180909-yiy0","tier":"normal","cause":"round_opened","draft":false,"phase":"pending"}"#;

    let HistoryEvent::Checkpoint { job, .. } = serde_json::from_str::<HistoryEvent>(json).unwrap() else {
        panic!("a checkpoint was written");
    };

    assert_eq!(job, CheckpointJob::Item);
}

#[test]
fn the_default_job_is_an_item() {
    assert_eq!(CheckpointJob::default(), CheckpointJob::Item);
}

#[test]
fn the_causes_of_the_other_jobs_read_and_write_their_keywords() {
    let causes = [
        (CheckpointCause::RequestStarted, "request_started"),
        (CheckpointCause::RequestChecked, "request_checked"),
        (CheckpointCause::ReviewStarted, "review_started"),
        (CheckpointCause::FindingsWritten, "findings_written"),
        (CheckpointCause::InvestigationStarted, "investigation_started"),
        (CheckpointCause::InvestigationAnswered, "investigation_answered"),
    ];
    for (cause, keyword) in causes {
        assert_eq!(cause.to_string(), keyword);
        assert_eq!(CheckpointCause::from_str(keyword).unwrap(), cause);
        assert_eq!(serde_json::to_string(&cause).unwrap(), format!("\"{keyword}\""));
    }
}

#[test]
fn a_request_is_meant_as_its_text_with_its_check() {
    assert_eq!(
        request_meant("Count the headings of docs/guide.md.", Some("test -s docs/count.txt")),
        "R1. Count the headings of docs/guide.md.\nK1. test -s docs/count.txt"
    );
    assert_eq!(request_meant("Look it up.", None), "R1. Look it up.");
}

#[test]
fn a_request_with_a_commit_and_no_check_run_has_the_commit_done_and_its_check_left() {
    let commits = [commit("a1b2c3d")];
    let changed = ["docs/count.txt".to_owned()];
    let work = JobWork::Request {
        last_check: None,
        commits: &commits,
        changed: &changed,
    };

    let content = job_content(request_meant("Count.", Some("test -s docs/count.txt")), &work);

    assert_eq!(content.meant, "R1. Count.\nK1. test -s docs/count.txt");
    assert_eq!(
        content.done,
        "Commits: 1\n- a1b2c3d\nChanged files: docs/count.txt\nCheck: not run yet"
    );
    assert_eq!(
        content.left,
        "K1 has not passed yet.\nnext: make the check pass and commit"
    );
}

#[test]
fn a_request_whose_check_failed_tells_the_line_and_leaves_the_check() {
    let work = JobWork::Request {
        last_check: Some(CheckResult {
            passed: false,
            line: "test failed: 3 != 4",
        }),
        commits: &[],
        changed: &[],
    };

    let content = job_content(request_meant("Count.", Some("make test")), &work);

    assert_eq!(
        content.done,
        "Commits: 0\nChanged files: none\nCheck: failed — test failed: 3 != 4"
    );
    assert_eq!(
        content.left,
        "K1 has not passed yet.\nnext: make the check pass and commit"
    );
}

#[test]
fn a_request_whose_check_passed_has_nothing_left_of_it() {
    let commits = [commit("a1b2c3d"), commit("b2c3d4e")];
    let work = JobWork::Request {
        last_check: Some(CheckResult {
            passed: true,
            line: "ok",
        }),
        commits: &commits,
        changed: &["a.txt".to_owned(), "b.txt".to_owned()],
    };

    let content = job_content(request_meant("Count.", Some("make test")), &work);

    assert_eq!(
        content.done,
        "Commits: 2\n- a1b2c3d\n- b2c3d4e\nChanged files: a.txt, b.txt\nCheck: passed — ok"
    );
    assert_eq!(
        content.left,
        "Nothing is left of the request: its check passed.\nnext: report the result"
    );
}

#[test]
fn a_review_is_meant_against_the_requirements_of_its_item() {
    let requirements = [
        requirement("R1", "Tell the briefing."),
        requirement("K1", "No test changes."),
    ];
    let item = ReviewedItem {
        id: "1.4".parse().unwrap(),
        title: "The briefing",
        requirements: &requirements,
    };

    assert_eq!(
        review_meant(pr(), "abc1234", 2, &item),
        "R1. Review pull request #7 at head abc1234, pass 2, against the requirements of item 1.4 (The briefing):\n\
         - R1. Tell the briefing.\n- K1. No test changes."
    );
}

#[test]
fn a_review_that_wrote_two_findings_has_both_done_and_the_submission_left() {
    let findings = "1. [MAJOR] ERR-3 a.rs:4 — a panic → return an error\n   CHECK: grep unwrap a.rs\n2. [MINOR] DOC-1 b.rs:9 — no doc → add one";
    let work = JobWork::Review { pass: 2, findings };

    let content = job_content("R1. Review.".to_owned(), &work);

    assert_eq!(content.done, format!("Findings written so far: 2\n{findings}"));
    assert_eq!(
        content.left,
        "next: finish the review of pass 2 and submit it with a verdict"
    );
}

#[test]
fn a_review_with_no_findings_file_says_none_were_written() {
    let work = JobWork::Review { pass: 1, findings: "" };

    let content = job_content("R1. Review.".to_owned(), &work);

    assert_eq!(content.done, "Findings written so far: 0");
}

#[test]
fn an_investigation_has_its_notes_done_and_the_answer_left() {
    let commits = [commit("c3d4e5f")];
    let changed = ["notes.md".to_owned()];
    let work = JobWork::Investigation {
        commits: &commits,
        changed: &changed,
    };

    let content = job_content(investigation_meant("Why does the split fail?"), &work);

    assert_eq!(content.meant, "R1. Answer the question: Why does the split fail?");
    assert_eq!(content.done, "Commits: 1\n- c3d4e5f\nChanged files: notes.md");
    assert_eq!(content.left, "next: answer the question");
}

#[test]
fn the_parts_of_a_job_carry_no_totals_line_for_the_listing() {
    let work = JobWork::Investigation {
        commits: &[],
        changed: &[],
    };

    assert_eq!(job_content(investigation_meant("Q?"), &work).counts(), None);
}

#[test]
fn a_list_longer_than_the_limit_is_cut_and_the_rest_counted() {
    let commits: Vec<CommitLine> = (0..45).map(|number| commit(&format!("c{number:06}"))).collect();
    let work = JobWork::Investigation {
        commits: &commits,
        changed: &[],
    };

    let content = job_content(investigation_meant("Q?"), &work);

    assert!(content.done.starts_with("Commits: 45\n- c000000\n"), "{}", content.done);
    assert!(content.done.contains("\n5 more left out\n"), "{}", content.done);
}
