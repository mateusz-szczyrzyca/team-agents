use std::str::FromStr;

use super::*;
use crate::ids::ItemId;

#[test]
fn legacy_plan_keeping_without_refused_gate_loads() {
    let json = r#"{"sent_back":2,"granted":1,"stage":"open"}"#;
    let keeping: PlanKeeping = serde_json::from_str(json).expect("deserialize legacy PlanKeeping");
    assert_eq!(keeping.sent_back(), 2);
    assert_eq!(keeping.granted(), 1);
    assert_eq!(keeping.stage(), PlanStage::Open);
    assert_eq!(keeping.refused_gate(), []);
}

#[test]
fn plan_keeping_with_multiple_refused_gate_entries_roundtrips() {
    let mut keeping = PlanKeeping::default();
    let step1 = ItemId::from_str("1.1").unwrap();
    let step2 = ItemId::from_str("1.2").unwrap();
    keeping.add_refused_gate(step1, "agent gate".to_owned());
    keeping.add_refused_gate(step2, "make gate".to_owned());
    // Repeated addition of the same step and command is deduplicated
    keeping.add_refused_gate(step1, "agent gate".to_owned());

    assert_eq!(
        keeping.refused_gate(),
        &[(step1, "agent gate".to_owned()), (step2, "make gate".to_owned())]
    );

    let json = serde_json::to_string(&keeping).expect("serialize PlanKeeping");
    assert!(
        json.contains(r#""refused_gate":[["1.1","agent gate"],["1.2","make gate"]]"#),
        "{json}"
    );

    let loaded: PlanKeeping = serde_json::from_str(&json).expect("deserialize PlanKeeping");
    assert_eq!(loaded.refused_gate(), keeping.refused_gate());

    let taken = keeping.take_refused_gate();
    assert_eq!(taken.len(), 2);
    assert_eq!(keeping.refused_gate(), []);
}

#[test]
fn legacy_mismatch_reason_without_title_or_reason_loads() {
    let json = r#"{"kind":"mismatch","step":"1.1","check":"cargo test","tail":"some error"}"#;
    let reason: PlanReason = serde_json::from_str(json).expect("deserialize legacy PlanReason::Mismatch");
    match reason {
        PlanReason::Mismatch {
            step,
            title,
            check,
            reason,
            tail,
        } => {
            assert_eq!(step, ItemId::from_str("1.1").unwrap());
            assert_eq!(title, "");
            assert_eq!(check.as_deref(), Some("cargo test"));
            assert_eq!(reason, None);
            assert_eq!(tail, "some error");
        }
        other => panic!("expected Mismatch, got {other:?}"),
    }
}

#[test]
fn running_plan_check_roundtrips_and_is_omitted_when_cleared() {
    use crate::status::RunOutcome;

    let mut keeping = PlanKeeping::default();
    let check = RunningPlanCheck {
        run: "run-20261122-060000-abcd".parse().unwrap(),
        end: RunEnd::Reported {
            outcome: RunOutcome::Done,
        },
        pause: None,
        made_progress: true,
        final_message: Some("Finished.\nSTATUS: DONE".to_owned()),
    };
    keeping.set_running_check(Some(check.clone()));
    assert_eq!(keeping.running_check(), Some(&check));

    let json = serde_json::to_string(&keeping).expect("serialize PlanKeeping with running_check");
    assert!(json.contains(r#""running_check":"#), "{json}");
    let loaded: PlanKeeping = serde_json::from_str(&json).expect("deserialize PlanKeeping with running_check");
    assert_eq!(loaded.running_check(), Some(&check));

    keeping.set_running_check(None);
    assert_eq!(keeping.running_check(), None);
    let cleared_json = serde_json::to_string(&keeping).expect("serialize cleared PlanKeeping");
    assert!(!cleared_json.contains("running_check"), "{cleared_json}");
}

#[test]
fn state_from_main_before_running_check_loads() {
    let fixture = include_str!("../../../tests/fixtures/state-v0-36-before-proposals.json");
    let state = crate::state::State::from_json(fixture).expect("legacy state from main loads");
    assert_eq!(state.teams().len(), 3);
}

#[test]
fn legacy_member_without_plan_persists_running_plan_check_without_enabling_plan_keeping() {
    use crate::decide::outcome::{Pause, PauseReason};
    use crate::state::Member;
    use crate::state::fixtures::{at, normal, task};

    let mut member = Member::without_plan(task(1), normal("claude-1"));
    assert_eq!(member.plan(), None);
    assert_eq!(member.plan_running_check(), None);

    let check = RunningPlanCheck {
        run: "run-20261122-060000-abcd".parse().unwrap(),
        end: RunEnd::RateLimited,
        pause: Some(Pause {
            until: at(600),
            reason: PauseReason::RateLimit,
            evidence: "429 Too Many Requests".to_owned(),
        }),
        made_progress: false,
        final_message: None,
    };
    member.set_plan_running_check(Some(check.clone()));
    assert_eq!(member.plan(), None, "legacy member still has no PlanKeeping");
    assert_eq!(member.plan_running_check(), Some(&check));

    let json = serde_json::to_string(&member).expect("serialize legacy member with running_check");
    assert!(json.contains(r#""running_check":"#), "{json}");
    let loaded: Member = serde_json::from_str(&json).expect("deserialize legacy member with running_check");
    assert_eq!(loaded.plan(), None);
    assert_eq!(loaded.plan_running_check(), Some(&check));

    member.set_plan_running_check(None);
    assert_eq!(member.plan_running_check(), None);
    let cleared = serde_json::to_string(&member).expect("serialize cleared legacy member");
    assert!(!cleared.contains("running_check"), "{cleared}");
}
