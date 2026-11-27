//! The vocabulary of a checkpoint: its keywords, the step of a round that leaves one, and its JSON.

use std::num::NonZeroU32;
use std::str::FromStr;

use strum::VariantArray;

use super::{CheckpointCause, CheckpointContent, CheckpointJob};
use crate::history::{CheckOutcome, HistoryEvent, LostCause, LostKind, RecordKind, SplitDecision};
use crate::ids::{InvestigatorName, RoundId, TaskId};
use crate::kinds::{Tier, UnknownKeyword};
use crate::state::{CommitId, PhaseKind, PrNumber, RoundEnd};
use crate::status::ReviewVerdict;

fn pr() -> PrNumber {
    PrNumber::new(NonZeroU32::new(7).unwrap())
}

fn commit() -> CommitId {
    "a".repeat(40).parse().unwrap()
}

fn task() -> TaskId {
    "task-20261126-180909-yiy0".parse().unwrap()
}

fn round() -> RoundId {
    "round-20261126-180909-vet6".parse().unwrap()
}

fn reviewer() -> InvestigatorName {
    "inv-codex".parse().unwrap()
}

/// The decision of a finding with the fields every one has.
fn finding(make: fn(PrNumber, NonZeroU32, u32, crate::ids::AgentName, String) -> HistoryEvent) -> HistoryEvent {
    make(
        pr(),
        NonZeroU32::MIN,
        1,
        "inv-codex".parse().unwrap(),
        "evidence".to_owned(),
    )
}

fn explained() -> HistoryEvent {
    finding(|pr, number, pass, by, why| HistoryEvent::FindingExplained {
        pr,
        number,
        pass,
        by,
        why,
    })
}

fn accepted() -> HistoryEvent {
    finding(|pr, number, pass, by, why| HistoryEvent::FindingAccepted {
        pr,
        number,
        pass,
        by,
        why,
    })
}

fn rejected() -> HistoryEvent {
    finding(|pr, number, pass, by, why| HistoryEvent::FindingRejected {
        pr,
        number,
        pass,
        by,
        why,
    })
}

fn raised() -> HistoryEvent {
    finding(|pr, number, pass, by, why| HistoryEvent::FindingRaised {
        pr,
        number,
        pass,
        by,
        why,
    })
}

fn confirmed() -> HistoryEvent {
    finding(|pr, number, pass, by, why| HistoryEvent::FindingConfirmed {
        pr,
        number,
        pass,
        by,
        why,
    })
}

fn not_confirmed() -> HistoryEvent {
    finding(|pr, number, pass, by, why| HistoryEvent::FindingNotConfirmed {
        pr,
        number,
        pass,
        by,
        why,
    })
}

fn second_opinion_asked() -> HistoryEvent {
    finding(|pr, number, pass, by, why| HistoryEvent::SecondOpinionAsked {
        pr,
        number,
        pass,
        by,
        why,
    })
}

fn checkpoint(content: Option<CheckpointContent>) -> HistoryEvent {
    HistoryEvent::Checkpoint {
        round: round(),
        item: Some("1.1".parse().unwrap()),
        task: task(),
        job: CheckpointJob::Item,
        tier: Tier::Investigator,
        cause: CheckpointCause::Commit,
        pull_request: Some(pr()),
        head: Some(commit()),
        draft: true,
        phase: PhaseKind::InProgress,
        review_pass: Some(1),
        fix_pass: None,
        snapshot: Some("refs/agents/snapshots/round-20261126-180909-vet6/task-20261126-180909-yiy0/1".to_owned()),
        content,
    }
}

#[test]
fn every_cause_reads_back_from_its_snake_case_keyword_and_from_its_json() {
    for cause in CheckpointCause::VARIANTS {
        let keyword = cause.to_string();
        assert_eq!(CheckpointCause::from_str(&keyword).unwrap(), *cause);
        let json = serde_json::to_string(cause).unwrap();
        assert_eq!(json, format!("\"{keyword}\""));
        assert_eq!(serde_json::from_str::<CheckpointCause>(&json).unwrap(), *cause);
    }
    assert_eq!(CheckpointCause::VARIANTS.len(), 23);
}

#[test]
fn an_unknown_cause_or_job_is_refused() {
    for text in ["pull-opened", "whenever"] {
        assert_eq!(
            CheckpointCause::from_str(text).unwrap_err(),
            UnknownKeyword::of::<CheckpointCause>(text)
        );
    }
    assert_eq!(
        CheckpointJob::from_str("review_pass").unwrap_err(),
        UnknownKeyword::of::<CheckpointJob>("review_pass")
    );
    let cause = serde_json::from_str::<CheckpointCause>("\"whenever\"").unwrap_err();
    assert!(cause.is_data(), "{cause}");
    let job = serde_json::from_str::<CheckpointJob>("\"chat\"").unwrap_err();
    assert!(job.is_data(), "{job}");
}

#[test]
fn every_job_reads_back_from_its_snake_case_keyword() {
    let keywords: Vec<String> = CheckpointJob::VARIANTS.iter().map(ToString::to_string).collect();
    assert_eq!(keywords, ["item", "worker_request", "review", "investigation"]);
    for job in CheckpointJob::VARIANTS {
        assert_eq!(
            serde_json::from_str::<CheckpointJob>(&serde_json::to_string(job).unwrap()).unwrap(),
            *job
        );
    }
}

#[test]
fn a_checkpoint_reads_back_equal_from_its_json_without_content() {
    let event = checkpoint(None);
    let json = serde_json::to_string(&event).unwrap();
    assert!(!json.contains("content"), "{json}");
    assert_eq!(serde_json::from_str::<HistoryEvent>(&json).unwrap(), event);
}

#[test]
fn a_checkpoint_reads_back_equal_from_its_json_with_content() {
    let event = checkpoint(Some(CheckpointContent {
        meant: "Record a checkpoint after each step — 日本語".to_owned(),
        done: "The types.".to_owned(),
        left: "The owner.".to_owned(),
    }));
    let json = serde_json::to_string(&event).unwrap();
    assert!(json.contains("\"content\""), "{json}");
    assert_eq!(serde_json::from_str::<HistoryEvent>(&json).unwrap(), event);
}

#[test]
fn a_checkpoint_written_without_its_optional_fields_reads_with_them_absent() {
    let json = r#"{"event":"checkpoint","round":"round-20261126-180909-vet6","task":"task-20261126-180909-yiy0","job":"item","tier":"normal","cause":"round_opened","draft":false,"phase":"pending"}"#;
    let HistoryEvent::Checkpoint {
        item,
        head,
        snapshot,
        content,
        pull_request,
        review_pass,
        fix_pass,
        ..
    } = serde_json::from_str::<HistoryEvent>(json).unwrap()
    else {
        panic!("not a checkpoint");
    };
    assert_eq!(
        (item, head, snapshot, content, pull_request, review_pass, fix_pass),
        (None, None, None, None, None, None, None)
    );
}

#[test]
fn a_checkpoint_with_an_unknown_cause_is_refused() {
    let json = r#"{"event":"checkpoint","round":"round-20261126-180909-vet6","task":"task-20261126-180909-yiy0","job":"item","tier":"normal","cause":"whenever","draft":false,"phase":"pending"}"#;
    let refusal = serde_json::from_str::<HistoryEvent>(json).unwrap_err();
    assert!(refusal.is_data(), "{refusal}");
}

fn assert_cause(event: &HistoryEvent, cause: Option<CheckpointCause>) {
    assert_eq!(event.checkpoint_cause(), cause, "{event:?}");
}

#[test]
fn a_pull_request_opened_leaves_a_pull_opened_checkpoint() {
    let event = HistoryEvent::PullOpened {
        number: pr(),
        branch: "agents/a1/20261126-180909".parse().unwrap(),
        head: commit(),
        title: "Add the thing".to_owned(),
        body: "Body".to_owned(),
    };
    assert_cause(&event, Some(CheckpointCause::PullOpened));
}

#[test]
fn a_push_leaves_a_pull_pushed_checkpoint() {
    assert_cause(
        &HistoryEvent::PullPushed {
            pr: pr(),
            head: commit(),
        },
        Some(CheckpointCause::PullPushed),
    );
}

#[test]
fn a_pull_request_marked_ready_leaves_a_pull_ready_checkpoint() {
    assert_cause(&HistoryEvent::PullReady { pr: pr() }, Some(CheckpointCause::PullReady));
}

#[test]
fn the_checks_leave_a_checked_checkpoint() {
    let event = HistoryEvent::Checked {
        pr: Some(pr()),
        head: commit(),
        issues: Vec::new(),
        outcome: CheckOutcome::Allowed,
    };
    assert_cause(&event, Some(CheckpointCause::Checked));
}

#[test]
fn a_split_decision_leaves_a_split_decided_checkpoint() {
    let event = HistoryEvent::SplitDecision {
        task: task(),
        item: "1.1".parse().unwrap(),
        decision: SplitDecision::Corrected,
        corrections: 1,
        question: None,
        reason: "why".to_owned(),
    };
    assert_cause(&event, Some(CheckpointCause::SplitDecided));
}

#[test]
fn a_submitted_review_leaves_a_review_submitted_checkpoint() {
    let event = HistoryEvent::ReviewSubmitted {
        pr: pr(),
        pass: 1,
        reviewer: reviewer(),
        task: task(),
        verdict: ReviewVerdict::Approve,
        report: "ok".to_owned(),
    };
    assert_cause(&event, Some(CheckpointCause::ReviewSubmitted));
}

#[test]
fn a_fix_pass_started_leaves_a_fix_pass_started_checkpoint() {
    let event = HistoryEvent::FixPassStarted {
        pr: pr(),
        pass: NonZeroU32::MIN,
        of: 5,
        findings: "Blocker".to_owned(),
    };
    assert_cause(&event, Some(CheckpointCause::FixPassStarted));
}

#[test]
fn an_explained_finding_leaves_a_finding_answered_checkpoint() {
    assert_cause(&explained(), Some(CheckpointCause::FindingAnswered));
}

#[test]
fn an_accepted_finding_leaves_a_finding_answered_checkpoint() {
    assert_cause(&accepted(), Some(CheckpointCause::FindingAnswered));
}

#[test]
fn a_rejected_finding_leaves_a_finding_answered_checkpoint() {
    assert_cause(&rejected(), Some(CheckpointCause::FindingAnswered));
}

#[test]
fn a_raised_finding_leaves_a_finding_answered_checkpoint() {
    assert_cause(&raised(), Some(CheckpointCause::FindingAnswered));
}

#[test]
fn a_confirmed_finding_leaves_a_finding_judged_checkpoint() {
    assert_cause(&confirmed(), Some(CheckpointCause::FindingJudged));
}

#[test]
fn a_finding_not_confirmed_leaves_a_finding_judged_checkpoint() {
    assert_cause(&not_confirmed(), Some(CheckpointCause::FindingJudged));
}

#[test]
fn questions_asked_leave_a_question_asked_checkpoint() {
    let event = HistoryEvent::QuestionAsked {
        task: task(),
        number: NonZeroU32::MIN,
        text: "## 1. Which?".to_owned(),
    };
    assert_cause(&event, Some(CheckpointCause::QuestionAsked));
}

#[test]
fn questions_answered_leave_a_question_answered_checkpoint() {
    let event = HistoryEvent::QuestionAnswered {
        task: task(),
        number: NonZeroU32::MIN,
        text: "## 1. Which?\n**Verdict:** ok".to_owned(),
        via: crate::state::AnsweredVia::Dashboard,
    };
    assert_cause(&event, Some(CheckpointCause::QuestionAnswered));
}

#[test]
fn an_approved_pull_request_leaves_a_pull_approved_checkpoint() {
    assert_cause(
        &HistoryEvent::PullApproved { pr: pr() },
        Some(CheckpointCause::PullApproved),
    );
}

#[test]
fn a_merged_pull_request_leaves_a_pull_merged_checkpoint() {
    let event = HistoryEvent::PullMerged {
        pr: pr(),
        commit: Some(commit()),
    };
    assert_cause(&event, Some(CheckpointCause::PullMerged));
}

#[test]
fn a_pull_request_handed_over_leaves_a_pull_handed_over_checkpoint() {
    assert_cause(
        &HistoryEvent::PullHandedOver { pr: pr() },
        Some(CheckpointCause::PullHandedOver),
    );
}

#[test]
fn a_closed_pull_request_leaves_a_pull_closed_checkpoint() {
    assert_cause(
        &HistoryEvent::PullClosed { pr: pr() },
        Some(CheckpointCause::PullClosed),
    );
}

#[test]
fn an_event_that_is_no_step_of_a_round_leaves_no_checkpoint() {
    let silent = [
        second_opinion_asked(),
        HistoryEvent::ReviewAsked {
            pr: pr(),
            pass: 1,
            head: commit(),
            reviewer: reviewer(),
            task: task(),
            among: Vec::new(),
        },
        HistoryEvent::RoundClosed {
            round: round(),
            end: RoundEnd::Merged,
        },
        HistoryEvent::TeamEnded {},
        HistoryEvent::Notified {
            task: None,
            title: "t".to_owned(),
            body: "b".to_owned(),
        },
        HistoryEvent::Lost {
            task: None,
            what: LostKind::Record(RecordKind::Step),
            cause: LostCause::WriteDiskFull,
            detail: "full".to_owned(),
        },
        checkpoint(None),
    ];
    for event in &silent {
        assert_cause(event, None);
    }
}

#[test]
fn the_gate_a_second_opinion_that_ended_a_control_and_a_reminder_leave_no_checkpoint() {
    let at: crate::state::Timestamp = "2026-11-26T18:09:09Z".parse().unwrap();
    let silent = [
        HistoryEvent::Gate {
            task: task(),
            started_at: at,
            record: crate::state::GateRecord {
                tree: crate::state::GateTree::Clean,
                result: crate::state::GateResult {
                    head: commit(),
                    finished_at: at,
                    outcome: crate::state::GateOutcome::Passed,
                    log: "/tmp/gate.log".parse().unwrap(),
                },
            },
        },
        HistoryEvent::SecondOpinionEnded {
            pr: pr(),
            number: NonZeroU32::MIN,
            pass: 1,
            by: "inv-codex".parse().unwrap(),
            why: "the run failed".to_owned(),
            attempt: NonZeroU32::MIN,
        },
        HistoryEvent::Controlled {
            action: crate::protocol::ControlAction::Stop,
            target: crate::protocol::Target::Team("team-20261126-180909-t5m8".parse().unwrap()),
            by: crate::protocol::Client::Dashboard {
                login: "human@example.com".parse().unwrap(),
            },
            tasks: vec![task()],
        },
        HistoryEvent::SerenaReminded {
            task: task(),
            run: "run-20261126-180909-r4b7".parse().unwrap(),
            searches: 40,
            step: 40,
            cli: crate::kinds::CliKind::Claude,
        },
    ];
    for event in &silent {
        assert_cause(event, None);
    }
}

#[test]
fn the_free_texts_of_a_checkpoint_are_the_parts_of_its_content() {
    let mut event = checkpoint(Some(CheckpointContent {
        meant: "meant".to_owned(),
        done: "done".to_owned(),
        left: "left".to_owned(),
    }));
    let found: Vec<(&str, String)> = event
        .texts_mut()
        .into_iter()
        .map(|(name, text)| (name, text.clone()))
        .collect();
    assert_eq!(
        found,
        [
            ("meant", "meant".to_owned()),
            ("done", "done".to_owned()),
            ("left", "left".to_owned())
        ]
    );
}

#[test]
fn a_checkpoint_without_content_has_no_free_text() {
    assert_eq!(checkpoint(None).texts_mut().len(), 0);
}

#[test]
fn a_checkpoint_of_an_earlier_version_with_no_job_reads_as_the_checkpoint_of_an_item() {
    let json = r#"{"event":"checkpoint","round":"round-20261126-180909-vet6","task":"task-20261126-180909-yiy0","tier":"normal","cause":"commit","draft":false,"phase":"in_progress"}"#;

    let HistoryEvent::Checkpoint { job, .. } = serde_json::from_str::<HistoryEvent>(json).unwrap() else {
        panic!("not a checkpoint");
    };

    assert_eq!(job, CheckpointJob::Item);
    assert_eq!(CheckpointJob::default(), CheckpointJob::Item);
}
