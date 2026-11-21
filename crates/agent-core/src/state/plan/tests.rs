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
