//! Compatibility with the persisted member shape before worked-time accounting.

use super::*;

#[test]
fn member_without_worked_secs_loads_with_zero_and_preserves_its_shape() {
    let fixture = include_str!("../../../tests/fixtures/member-before-worked-seconds.json");
    let before: serde_json::Value = serde_json::from_str(fixture).unwrap();
    let member: Member = serde_json::from_str(fixture).unwrap();
    assert_eq!(member.lead_worked_secs(), 0);
    assert_eq!(serde_json::to_value(member).unwrap(), before);
}
