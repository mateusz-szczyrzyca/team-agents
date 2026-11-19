//! Team allowances survive the same serialized state that the daemon reloads.
use super::*;
use crate::state::fixtures;

#[test]
fn team_allowances_survive_serialization_and_are_available_for_every_round() {
    let original = fixtures::team(1);
    let mut wire = serde_json::to_value(&original).unwrap();
    wire["task_list"] = serde_json::Value::String(format!(
        "Allow: Cargo.toml\nAllow: tests/**/*.golden\n{}",
        fixtures::LIST.replace("Coordinator: inv-opus\n", "")
    ));
    let team: Team = serde_json::from_value(wire).unwrap();
    let expected = ["Cargo.toml", "tests/**/*.golden"];
    assert_eq!(
        team.allowances()
            .iter()
            .map(crate::task_allowance::TaskAllowance::as_str)
            .collect::<Vec<_>>(),
        expected
    );
    let restored: Team = serde_json::from_str(&serde_json::to_string(&team).unwrap()).unwrap();
    assert_eq!(restored.allowances(), team.allowances());
    assert_eq!(restored.items().len(), team.items().len());
    assert_eq!(original.allowances(), []);
}
