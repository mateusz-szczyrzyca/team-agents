//! Claims are exact protocol markers, never evidence that a PR delivered an item.

use super::*;

#[test]
fn reads_standalone_delivery_claim_with_final_done_status() {
    let report = delivery_report("The work is in the team's PR.\nDELIVERED: #144\nSTATUS: DONE\n").unwrap();
    assert_eq!(report.map(|number| number.get().get()), Some(144));
}
