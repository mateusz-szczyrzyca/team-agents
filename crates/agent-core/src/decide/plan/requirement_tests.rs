//! A plan whose steps do not name the requirements of their item is sent back; a legacy plan reads as before.

use super::*;
use crate::config::Plan as Policy;
use crate::ids::{ItemId, TaskId};
use crate::kinds::Role;
use crate::plan::parse_plan;
use crate::state::{Member, Participant, PlanKeeping, PlanReason};
use crate::status::RunOutcome;

/// The requirements `R1`, `R2`, `R3`, `K1` and `K2` of an item with three bullets and two checks.
fn known() -> Vec<RequirementId> {
    ["R1", "R2", "R3", "K1", "K2"]
        .iter()
        .map(|text| text.parse().unwrap())
        .collect()
}

/// An item ID.
fn item(text: &str) -> ItemId {
    text.parse().unwrap()
}

/// What a task keeps: created under numbered requirements, or before them.
fn keeping(tied: bool) -> PlanKeeping {
    let task: TaskId = "task-20261111-153001-k3x9".parse().unwrap();
    let member = Member::new(task, Participant::Normal("codex-1".parse().unwrap()));
    let created = member.plan().cloned().unwrap();
    if tied {
        created
    } else {
        serde_json::from_value(without_marker(&created)).unwrap()
    }
}

/// The verdict for a run that reported done with the plan `text`, for an item with the requirements `known`.
fn verdict(text: &str, tied: bool, known: Option<&[RequirementId]>) -> PlanVerdict {
    let read = PlanRead::Parsed(parse_plan(text).unwrap());
    let end = RunEnd::Reported {
        outcome: RunOutcome::Done,
    };
    let kept = keeping(tied);
    decide_plan_for_item(
        PlanFacts {
            role: Role::Member,
            end: &end,
            read: &read,
            earlier: &[],
            keeping: Some(&kept),
            policy: &Policy::default(),
        },
        None,
        known,
    )
}

/// The state of a keeping as JSON without the marker of numbered requirements: how a task created before has it.
fn without_marker(created: &PlanKeeping) -> serde_json::Value {
    let mut value = serde_json::to_value(created).unwrap();
    value.as_object_mut().unwrap().remove("requirements_tied");
    value
}

/// A finished plan whose steps name requirements.
const TIED: &str =
    "1. Work\n[x] 1.1 One\n    For: R1, K1\n[x] 1.2 Two\n    For: R2\n[-] 1.3 Skipped\n    Not needed.\n";

#[test]
fn a_plan_whose_steps_name_requirements_of_the_item_is_accepted() {
    assert_eq!(verdict(TIED, true, Some(&known())), PlanVerdict::Accept);
}

#[test]
fn a_step_with_no_for_line_sends_the_plan_back_with_the_requirements_of_the_item() {
    let text = "1. Work\n[x] 1.1 One\n    For: R1\n[x] 1.2 Two\n";
    assert_eq!(
        verdict(text, true, Some(&known())),
        PlanVerdict::SendBack(PlanReason::NoRequirement {
            without: vec![item("1.2")],
            unknown: Vec::new(),
            known: known(),
        })
    );
}

#[test]
fn a_step_that_names_a_requirement_the_item_does_not_have_sends_the_plan_back() {
    let text = "1. Work\n[x] 1.1 One\n    For: R1, R9\n[x] 1.2 Two\n    For: K3\n";
    assert_eq!(
        verdict(text, true, Some(&known())),
        PlanVerdict::SendBack(PlanReason::NoRequirement {
            without: Vec::new(),
            unknown: vec![item("1.1"), item("1.2")],
            known: known(),
        })
    );
}

#[test]
fn a_step_given_up_needs_no_requirement() {
    let text = "1. Work\n[x] 1.1 One\n    For: R1\n[-] 1.2 Two\n    Not needed.\n";
    assert_eq!(verdict(text, true, Some(&known())), PlanVerdict::Accept);
}

#[test]
fn a_task_with_no_item_is_not_asked_for_requirements() {
    assert_eq!(verdict("1. Work\n[x] 1.1 One\n", true, None), PlanVerdict::Accept);
}

#[test]
fn a_plan_of_a_task_created_before_numbered_requirements_with_no_for_line_reads_as_before() {
    assert_eq!(
        verdict("1. Work\n[x] 1.1 One\n", false, Some(&known())),
        PlanVerdict::Accept
    );
}

#[test]
fn a_plan_of_an_older_task_is_checked_once_a_step_has_a_for_line() {
    let text = "1. Work\n[x] 1.1 One\n    For: R1\n[x] 1.2 Two\n";
    assert_eq!(
        verdict(text, false, Some(&known())),
        PlanVerdict::SendBack(PlanReason::NoRequirement {
            without: vec![item("1.2")],
            unknown: Vec::new(),
            known: known(),
        })
    );
}

#[test]
fn a_task_created_now_keeps_the_marker_and_one_written_before_does_not() {
    assert!(keeping(true).requirements_tied());
    assert!(!keeping(false).requirements_tied());
}

#[test]
fn the_marker_is_written_when_set_and_read_back_and_a_keeping_without_it_is_not_tied() {
    let created = keeping(true);
    let written = serde_json::to_value(&created).unwrap();
    assert_eq!(written["requirements_tied"], serde_json::Value::Bool(true));
    let again: PlanKeeping = serde_json::from_value(written).unwrap();
    assert_eq!(again, created);
    let old: PlanKeeping = serde_json::from_value(without_marker(&created)).unwrap();
    assert!(!old.requirements_tied());
}

#[test]
fn the_reason_is_kept_in_the_state_and_read_back() {
    let reason = PlanReason::NoRequirement {
        without: vec![item("1.2")],
        unknown: vec![item("1.1")],
        known: known(),
    };
    let text = serde_json::to_string(&reason).unwrap();
    assert_eq!(serde_json::from_str::<PlanReason>(&text).unwrap(), reason);
    assert!(text.contains(r#""kind":"no_requirement""#), "{text}");
    assert!(text.contains(r#""R1""#), "{text}");
}
