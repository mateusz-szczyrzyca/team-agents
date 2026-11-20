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
        bind_delivery(&team, &[round_id(99)]),
        Err(DeliveryBindingError::Unknown { round: round_id(99) })
    );
    drive(&mut team, "1.2", PhaseKind::InReview);
    assert_eq!(bind_delivery(&team, &[round_id(1)]), Err(DeliveryBindingError::Missing));
    assert_eq!(bind_delivery(&team, &[round_id(1), round_id(2)]), Ok(round_id(2)));
    team.deliver_round(&round_id(2), crate::state::fixtures::pr(7).number)
        .unwrap();
    assert_eq!(bind_delivery(&team, &[round_id(2)]), Err(DeliveryBindingError::Missing));
}
