//! Claims are exact protocol markers, never evidence that a PR delivered an item.

use super::*;

#[test]
fn reads_standalone_delivery_claim_with_final_done_status() {
    let report = delivery_report("The work is in the team's PR.\nDELIVERED: #144\nSTATUS: DONE\n").unwrap();
    assert_eq!(report.map(|number| number.get().get()), Some(144));
}

#[test]
fn refuses_malformed_duplicate_and_unsuccessful_claims() {
    for marker in [
        "DELIVERED: 1",
        "DELIVERED: #1 trailing",
        "DELIVERED: #+1",
        "DELIVERED: #",
        "DELIVERED: #١",
    ] {
        assert!(matches!(
            delivery_report(&format!("{marker}\nSTATUS: DONE")),
            Err(DeliveryReportError::Syntax { line: 1 })
        ));
    }
    for number in ["0", "4294967296"] {
        assert!(matches!(
            delivery_report(&format!("DELIVERED: #{number}\nSTATUS: DONE")),
            Err(DeliveryReportError::Number { line: 1, .. })
        ));
    }
    assert!(matches!(
        delivery_report("DELIVERED: #1\nDELIVERED: #2\nSTATUS: DONE"),
        Err(DeliveryReportError::Duplicate)
    ));
    for status in [
        "",
        "STATUS: FAILED",
        "STATUS: DONE\nmore",
        "STATUS: FAILED\nSTATUS: DONE",
    ] {
        assert!(matches!(
            delivery_report(&format!("DELIVERED: #1\n{status}")),
            Err(DeliveryReportError::NotDone)
        ));
    }
}

#[test]
fn accepts_the_numeric_boundary_and_ignores_prose_without_a_marker() {
    assert_eq!(
        delivery_report("DELIVERED: #4294967295\nSTATUS: DONE")
            .unwrap()
            .map(|number| number.get().get()),
        Some(u32::MAX)
    );
    for message in ["Delivered2fixes", "PR #12 delivered the work.\nSTATUS: DONE", ""] {
        assert_eq!(delivery_report(message).unwrap(), None);
    }
}

#[test]
fn delivery_binding_uses_only_the_reported_runs_rounds_and_refuses_ambiguity() {
    use crate::state::PhaseKind;
    use crate::state::fixtures::{at, drive, round_id, team};
    let mut team = team(1);
    for (id, item) in [(1, "1.2"), (2, "2.1")] {
        team.open_round(round_id(id), Some(item.parse().unwrap()), at(1))
            .unwrap();
        drive(&mut team, item, PhaseKind::InProgress);
    }
    assert_eq!(bind_delivery(&team, &[round_id(2)]), Ok(round_id(2)));
    assert_eq!(bind_delivery(&team, &[]), Err(DeliveryBindingError::Missing));
    assert_eq!(
        bind_delivery(&team, &[round_id(1), round_id(2)]),
        Err(DeliveryBindingError::Ambiguous)
    );
    assert_eq!(
        bind_delivery(&team, &[round_id(9)]),
        Err(DeliveryBindingError::Unknown { round: round_id(9) })
    );
    team.item_mut("1.2".parse().unwrap())
        .unwrap()
        .advance(crate::state::ItemPhase::InReview {
            pr: crate::state::fixtures::pr(7),
            reviews: Vec::new(),
        })
        .unwrap();
    assert_eq!(bind_delivery(&team, &[round_id(1)]), Err(DeliveryBindingError::Missing));
    assert_eq!(bind_delivery(&team, &[round_id(1), round_id(2)]), Ok(round_id(2)));
    team.deliver_round(&round_id(2), crate::state::fixtures::pr(7).number)
        .unwrap();
    assert_eq!(bind_delivery(&team, &[round_id(2)]), Err(DeliveryBindingError::Missing));
}

#[test]
fn observations_act_only_on_the_current_waiting_claim() {
    use crate::state::PhaseKind;
    use crate::state::fixtures::{at, drive, pr, round_id, team};
    let mut waiting = team(1);
    for (id, item, phase) in [(1, "1.2", PhaseKind::InReview), (2, "2.1", PhaseKind::InProgress)] {
        waiting
            .open_round(round_id(id), Some(item.parse().unwrap()), at(1))
            .unwrap();
        drive(&mut waiting, item, phase);
    }
    waiting.deliver_round(&round_id(2), pr(7).number).unwrap();
    let mut ended = waiting.clone();
    ended.end_delivery(&round_id(2), at(40)).unwrap();
    let mut done = waiting.clone();
    done.complete_delivery(&round_id(2), at(40)).unwrap();
    for (name, team, observation, expected) in [
        (
            "merged",
            &waiting,
            DeliveryObservation::Merged,
            DeliveryAction::Complete,
        ),
        ("open", &waiting, DeliveryObservation::Open, DeliveryAction::Wait),
        ("closed", &waiting, DeliveryObservation::Closed, DeliveryAction::Return),
        (
            "closed and seen",
            &ended,
            DeliveryObservation::Closed,
            DeliveryAction::Ignore,
        ),
        (
            "reopened after seen",
            &ended,
            DeliveryObservation::Open,
            DeliveryAction::Ignore,
        ),
        (
            "reopened before seen",
            &waiting,
            DeliveryObservation::Open,
            DeliveryAction::Wait,
        ),
        ("item done", &done, DeliveryObservation::Merged, DeliveryAction::Ignore),
    ] {
        assert_eq!(delivery_action(team, &round_id(2), observation), expected, "{name}");
    }
}

#[test]
fn evaluate_delivery_report_table_covers_all_spec_cases() {
    struct Case {
        name: &'static str,
        same_repo: bool,
        pr_branch: &'static str,
        task_branch: &'static str,
        part_branch: Option<&'static str>,
        state: DeliveryObservation,
        expected: DeliveryVerdict,
    }

    let cases = [
        Case {
            name: "merged pr matching task branch is accepted",
            same_repo: true,
            pr_branch: "agents/codex-1/task-1-1.1",
            task_branch: "agents/codex-1/task-1-1.1",
            part_branch: None,
            state: DeliveryObservation::Merged,
            expected: DeliveryVerdict::Accepted,
        },
        Case {
            name: "merged pr matching part branch is accepted",
            same_repo: true,
            pr_branch: "agents/codex-1/task-1-1.1b",
            task_branch: "agents/codex-1/task-1-1.1",
            part_branch: Some("agents/codex-1/task-1-1.1b"),
            state: DeliveryObservation::Merged,
            expected: DeliveryVerdict::Accepted,
        },
        Case {
            name: "open pr is refused as unmerged",
            same_repo: true,
            pr_branch: "agents/codex-1/task-1-1.1",
            task_branch: "agents/codex-1/task-1-1.1",
            part_branch: None,
            state: DeliveryObservation::Open,
            expected: DeliveryVerdict::RefusedOpen,
        },
        Case {
            name: "closed without merge is refused",
            same_repo: true,
            pr_branch: "agents/codex-1/task-1-1.1",
            task_branch: "agents/codex-1/task-1-1.1",
            part_branch: None,
            state: DeliveryObservation::Closed,
            expected: DeliveryVerdict::RefusedClosed,
        },
        Case {
            name: "pr of another repository is refused",
            same_repo: false,
            pr_branch: "agents/codex-1/task-1-1.1",
            task_branch: "agents/codex-1/task-1-1.1",
            part_branch: None,
            state: DeliveryObservation::Merged,
            expected: DeliveryVerdict::RefusedForeignRepository,
        },
        Case {
            name: "pr branch matching neither task nor part branch is refused",
            same_repo: true,
            pr_branch: "random-foreign-branch",
            task_branch: "agents/codex-1/task-1-1.1",
            part_branch: Some("agents/codex-1/task-1-1.1b"),
            state: DeliveryObservation::Merged,
            expected: DeliveryVerdict::RefusedForeignBranch,
        },
        Case {
            name: "pr branch of parent task branch is accepted when delivering a split part",
            same_repo: true,
            pr_branch: "agents/codex-1/task-1-1.1",
            task_branch: "agents/codex-1/task-1-1.1",
            part_branch: Some("agents/codex-1/task-1-1.1b"),
            state: DeliveryObservation::Merged,
            expected: DeliveryVerdict::Accepted,
        },
        Case {
            name: "pr branch of another split part branch is refused",
            same_repo: true,
            pr_branch: "agents/codex-1/task-1-1.1a",
            task_branch: "agents/codex-1/task-1-1.1",
            part_branch: Some("agents/codex-1/task-1-1.1b"),
            state: DeliveryObservation::Merged,
            expected: DeliveryVerdict::RefusedForeignBranch,
        },
    ];

    for case in cases {
        let verdict = evaluate_delivery_report(DeliveryReportFacts {
            same_repo: case.same_repo,
            pr_branch: case.pr_branch,
            task_branch: case.task_branch,
            part_branch: case.part_branch,
            state: case.state,
        });
        assert_eq!(verdict, case.expected, "case: {}", case.name);
    }
}
