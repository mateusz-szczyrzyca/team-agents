//! The values of the briefing's review variables: the pull request, where the review stands, and the findings.

use std::num::NonZeroU32;

use agent_core::review_findings::{Finding, Severity};
use agent_core::state::FindingStatus;

use super::super::{BriefingNotes, briefing_variables_with};
use super::*;

/// The facts of a briefing that decide nothing but the review.
fn facts() -> super::super::Briefing<'static> {
    super::super::Briefing {
        reason: "no_status",
        task: "task-20261120-144101-r7q2",
        prompt: "Fix the parser.",
        base: "main",
        branch: "agents/claude-1/task-20261120-144101-r7q2-1.1",
        commits: &[],
        uncommitted: &[],
        plan: None,
        messages: &[],
        left_out: None,
        snapshot: "refs/agents/snapshots/round-1/task-20261120-144101-r7q2/1",
    }
}

/// A number that is not zero.
fn nonzero(number: u32) -> NonZeroU32 {
    NonZeroU32::new(number).unwrap()
}

/// A finding of the ledger, reported in pass `pass` by `inv-codex`.
fn recorded(number: u32, pass: u32, severity: Severity, status: FindingStatus) -> RecordedFinding {
    RecordedFinding {
        number: nonzero(number),
        finding: Finding {
            severity,
            rule: "TDD-2".to_owned(),
            place: format!("src/lib.rs:{number}"),
            text: format!("the case {number} is untested → add coverage"),
            check: format!("read src/lib.rs:{number}"),
        },
        reporter: "inv-codex".parse().unwrap(),
        pass: nonzero(pass),
        status,
    }
}

/// The review of a pull request that stands as `standing`, with no findings.
fn standing_review(standing: ReviewStanding) -> BriefingReview<'static> {
    BriefingReview {
        standing,
        ..BriefingReview::default()
    }
}

/// The state line of the review that stands as `standing`.
fn state_line(standing: ReviewStanding) -> String {
    review_values(&standing_review(standing)).review_state
}

#[test]
fn a_pull_request_in_review_is_told_with_its_pass() {
    assert_eq!(
        state_line(ReviewStanding::InReview { pass: nonzero(2) }),
        "in review, pass 2: the reviewers are reading it; wait for their findings."
    );
}

#[test]
fn a_fix_pass_is_told_with_its_number_and_its_limit() {
    assert_eq!(
        state_line(ReviewStanding::Fixing {
            pass: nonzero(1),
            limit: 5
        }),
        "fix pass 1 of 5: fix the open findings below, in this pull request."
    );
}

#[test]
fn a_fix_pass_that_waits_for_the_human_is_told_to_read_the_humans_messages_first() {
    assert_eq!(
        state_line(ReviewStanding::WaitingForHuman {
            pass: nonzero(3),
            limit: 3
        }),
        "fix pass 3 of 3 was sent back to its limit; it waits for the human. Read the human's messages first."
    );
}

#[test]
fn an_approved_pull_request_is_told_to_wait_for_the_merge() {
    assert_eq!(
        state_line(ReviewStanding::Approved),
        "approved: nothing is left to fix; wait for the merge."
    );
}

#[test]
fn an_item_not_in_review_is_told_so() {
    assert_eq!(state_line(ReviewStanding::NotInReview), "not in review yet.");
}

#[test]
fn a_task_with_no_pull_request_is_told_there_is_none_and_has_no_findings() {
    let values = review_values(&BriefingReview::default());

    assert_eq!(values.pull_request, "No pull request yet.");
    assert_eq!(values.review_state, "not in review yet.");
    assert_eq!(values.findings, "");
}

#[test]
fn a_pull_request_is_told_with_its_number_its_branch_and_when_it_was_opened() {
    let review = BriefingReview {
        pull_request: Some(BriefingPullRequest {
            number: nonzero(313),
            branch: "agents/inv-claude-sonnet/task-20261126-170229-myec-1.1",
            opened: "2026-11-26 17:02",
        }),
        ..BriefingReview::default()
    };

    assert_eq!(
        review_values(&review).pull_request,
        "Pull request #313 on branch agents/inv-claude-sonnet/task-20261126-170229-myec-1.1, opened 2026-11-26 17:02."
    );
}

#[test]
fn each_finding_is_a_line_in_the_order_of_its_number_with_its_status_pass_and_reporter() {
    let ledger = [
        recorded(1, 1, Severity::Blocker, FindingStatus::Open),
        recorded(2, 1, Severity::Major, FindingStatus::Resolved),
        recorded(3, 2, Severity::Minor, FindingStatus::NotConfirmed),
    ];
    let review = BriefingReview {
        findings: BriefingFindings::Ledger(&ledger),
        ..BriefingReview::default()
    };

    assert_eq!(
        review_values(&review).findings,
        "- #1 [BLOCKER] src/lib.rs:1 — the case 1 is untested → add coverage (open; reported in pass 1 by inv-codex)\n\
         - #2 [MAJOR] src/lib.rs:2 — the case 2 is untested → add coverage (resolved; reported in pass 1 by inv-codex)\n\
         - #3 [MINOR] src/lib.rs:3 — the case 3 is untested → add coverage (not confirmed; reported in pass 2 by inv-codex)"
    );
}

#[test]
fn a_finding_with_a_text_of_several_lines_and_non_ascii_letters_stays_one_line() {
    let mut finding = recorded(1, 1, Severity::Major, FindingStatus::Open);
    finding.finding.place = "src/żółć.rs:7".to_owned();
    finding.finding.text = "the parser drops a line\nof \u{201c}zażółć\u{201d}".to_owned();
    let ledger = [finding];
    let review = BriefingReview {
        findings: BriefingFindings::Ledger(&ledger),
        ..BriefingReview::default()
    };

    assert_eq!(
        review_values(&review).findings,
        "- #1 [MAJOR] src/żółć.rs:7 — the parser drops a line of \u{201c}zażółć\u{201d} (open; reported in pass 1 by inv-codex)"
    );
}

#[test]
fn forty_findings_are_all_listed() {
    let ledger: Vec<_> = (1..=40)
        .map(|number| recorded(number, 1, Severity::Minor, FindingStatus::Resolved))
        .collect();
    let review = BriefingReview {
        findings: BriefingFindings::Ledger(&ledger),
        ..BriefingReview::default()
    };

    let findings = review_values(&review).findings;

    assert_eq!(findings.lines().count(), 40);
    assert!(!findings.contains("left out"), "{findings}");
}

#[test]
fn above_forty_findings_the_open_ones_are_listed_and_the_others_are_counted() {
    let ledger: Vec<_> = (1..=45)
        .map(|number| {
            let status = if number == 7 || number == 44 {
                FindingStatus::Open
            } else if number % 2 == 0 {
                FindingStatus::Resolved
            } else {
                FindingStatus::NotConfirmed
            };
            recorded(number, 1, Severity::Major, status)
        })
        .collect();
    let review = BriefingReview {
        findings: BriefingFindings::Ledger(&ledger),
        ..BriefingReview::default()
    };

    assert_eq!(
        review_values(&review).findings,
        "- #7 [MAJOR] src/lib.rs:7 — the case 7 is untested → add coverage (open; reported in pass 1 by inv-codex)\n\
         - #44 [MAJOR] src/lib.rs:44 — the case 44 is untested → add coverage (open; reported in pass 1 by inv-codex)\n\
         43 resolved or not confirmed findings left out: see the pull request."
    );
}

#[test]
fn above_forty_open_findings_every_one_is_listed_and_nothing_is_said_to_be_left_out() {
    let ledger: Vec<_> = (1..=41)
        .map(|number| recorded(number, 1, Severity::Minor, FindingStatus::Open))
        .collect();
    let review = BriefingReview {
        findings: BriefingFindings::Ledger(&ledger),
        ..BriefingReview::default()
    };

    let findings = review_values(&review).findings;

    assert_eq!(findings.lines().count(), 41);
    assert!(!findings.contains("left out"), "{findings}");
}

#[test]
fn a_pull_request_reviewed_before_the_findings_were_numbered_points_to_its_comments() {
    let review = BriefingReview {
        findings: BriefingFindings::Legacy {
            pull_request: nonzero(82),
        },
        ..BriefingReview::default()
    };

    assert_eq!(
        review_values(&review).findings,
        "The review of this pull request kept no numbered findings: read its comments with gh pr view 82 --comments."
    );
}

#[test]
fn the_briefing_gives_the_review_variables_its_template_names() {
    let ledger = [recorded(1, 1, Severity::Major, FindingStatus::Open)];
    let notes = BriefingNotes {
        review: BriefingReview {
            pull_request: Some(BriefingPullRequest {
                number: nonzero(5),
                branch: "agents/claude-1/task-20261120-144101-r7q2-1.1",
                opened: "2026-11-20 14:41",
            }),
            standing: ReviewStanding::Fixing {
                pass: nonzero(1),
                limit: 5,
            },
            findings: BriefingFindings::Ledger(&ledger),
        },
        ..BriefingNotes::default()
    };

    let values = briefing_variables_with(&facts(), &notes).unwrap();
    let value = |name: &str| {
        let name = name.parse().expect("a variable name");
        values.get(&name).map(str::to_owned)
    };

    assert_eq!(
        value("pull_request").as_deref(),
        Some("Pull request #5 on branch agents/claude-1/task-20261120-144101-r7q2-1.1, opened 2026-11-20 14:41.")
    );
    assert_eq!(
        value("review_state").as_deref(),
        Some("fix pass 1 of 5: fix the open findings below, in this pull request.")
    );
    assert_eq!(
        value("findings").as_deref(),
        Some(
            "- #1 [MAJOR] src/lib.rs:1 — the case 1 is untested → add coverage (open; reported in pass 1 by inv-codex)"
        )
    );
}
