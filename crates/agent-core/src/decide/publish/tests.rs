use std::num::NonZeroI32;

use strum::VariantArray as _;

use super::*;
use crate::state::{GateOutcome, GateRecord, GateResult, GateTree};

const TIP: &str = "26b65de7e4076596768ca96b48a7f94799c93874";
const OTHER: &str = "1111111111111111111111111111111111111111";

fn make_record(head: &str, tree: GateTree, outcome: GateOutcome) -> GateRecord {
    GateRecord {
        tree,
        result: GateResult {
            head: head.parse().unwrap(),
            finished_at: "2026-11-13T09:30:00Z".parse().unwrap(),
            outcome,
            log: "/home/u/agents/tasks/task-1/gate-1.log".parse().unwrap(),
        },
    }
}

fn failed(exit_code: i32) -> GateOutcome {
    GateOutcome::Failed {
        exit_code: NonZeroI32::new(exit_code).unwrap(),
    }
}

#[test]
fn publication_wait_reason_decides_every_reason_and_the_precedence_order() {
    let tip_commit: CommitId = TIP.parse().unwrap();

    let clean_passed_at_tip = make_record(TIP, GateTree::Clean, GateOutcome::Passed);
    let dirty_passed_at_tip = make_record(TIP, GateTree::NotClean, GateOutcome::Passed);
    let clean_failed_at_tip = make_record(TIP, GateTree::Clean, failed(1));
    let dirty_failed_at_tip = make_record(TIP, GateTree::NotClean, failed(1));
    let clean_timeout_at_tip = make_record(TIP, GateTree::Clean, GateOutcome::TimedOut);

    let clean_passed_at_other = make_record(OTHER, GateTree::Clean, GateOutcome::Passed);
    let dirty_passed_at_other = make_record(OTHER, GateTree::NotClean, GateOutcome::Passed);
    let clean_failed_at_other = make_record(OTHER, GateTree::Clean, failed(2));
    let dirty_failed_at_other = make_record(OTHER, GateTree::NotClean, failed(2));

    // Table of (tip, record, expected reason, description)
    let cases: &[(
        Option<&CommitId>,
        Option<&GateRecord>,
        Option<PublicationWaitReason>,
        &str,
    )] = &[
        // Missing tip
        (
            None,
            None,
            Some(PublicationWaitReason::NoBranchTip),
            "no tip and no record",
        ),
        (
            None,
            Some(&clean_passed_at_tip),
            Some(PublicationWaitReason::NoBranchTip),
            "no tip with record",
        ),
        // Missing record
        (
            Some(&tip_commit),
            None,
            Some(PublicationWaitReason::NoGateRecord),
            "tip but no record",
        ),
        // Record at another commit takes precedence over whether that record failed or tree was dirty
        (
            Some(&tip_commit),
            Some(&clean_passed_at_other),
            Some(PublicationWaitReason::GateAtAnotherCommit),
            "gate passed at other commit",
        ),
        (
            Some(&tip_commit),
            Some(&dirty_passed_at_other),
            Some(PublicationWaitReason::GateAtAnotherCommit),
            "gate passed at other commit on dirty tree",
        ),
        (
            Some(&tip_commit),
            Some(&clean_failed_at_other),
            Some(PublicationWaitReason::GateAtAnotherCommit),
            "gate failed at other commit",
        ),
        (
            Some(&tip_commit),
            Some(&dirty_failed_at_other),
            Some(PublicationWaitReason::GateAtAnotherCommit),
            "gate failed at other commit on dirty tree",
        ),
        // At tip, failed outcomes
        (
            Some(&tip_commit),
            Some(&clean_failed_at_tip),
            Some(PublicationWaitReason::GateFailed),
            "gate failed at tip",
        ),
        (
            Some(&tip_commit),
            Some(&dirty_failed_at_tip),
            Some(PublicationWaitReason::GateFailed),
            "gate failed at tip on dirty tree",
        ),
        (
            Some(&tip_commit),
            Some(&clean_timeout_at_tip),
            Some(PublicationWaitReason::GateFailed),
            "gate timed out at tip",
        ),
        // At tip, passed outcome but dirty tree
        (
            Some(&tip_commit),
            Some(&dirty_passed_at_tip),
            Some(PublicationWaitReason::GateTreeNotClean),
            "gate passed on dirty tree",
        ),
        // At tip, passed outcome on clean tree -> ready (None)
        (
            Some(&tip_commit),
            Some(&clean_passed_at_tip),
            None,
            "gate passed on clean tree at tip",
        ),
    ];

    for &(tip, record, expected, desc) in cases {
        assert_eq!(publication_wait_reason(tip, record), expected, "case: {desc}");
    }
}

#[test]
fn publication_wait_reason_strum_names_match_the_protocol() {
    assert_eq!(PublicationWaitReason::NoBranchTip.as_str(), "no_branch_tip");
    assert_eq!(PublicationWaitReason::NoGateRecord.as_str(), "no_gate_record");
    assert_eq!(
        PublicationWaitReason::GateAtAnotherCommit.as_str(),
        "gate_at_another_commit"
    );
    assert_eq!(PublicationWaitReason::GateFailed.as_str(), "gate_failed");
    assert_eq!(PublicationWaitReason::GateTreeNotClean.as_str(), "gate_tree_not_clean");

    for reason in PublicationWaitReason::VARIANTS {
        assert_eq!(reason.to_string(), reason.as_str());
        assert_eq!(reason.as_str().parse::<PublicationWaitReason>().unwrap(), *reason);
    }
}
