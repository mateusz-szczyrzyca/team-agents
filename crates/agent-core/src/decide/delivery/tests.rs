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
