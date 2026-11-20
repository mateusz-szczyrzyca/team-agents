//! Starting with unpublished base commits is explicit and durable; schema 11 grants no permission.
use agent_core::state::{State, UnpushedBase};
use std::num::NonZeroU64;

#[test]
fn unpushed_base_roundtrips_and_schema_eleven_migrates_without_granting_permission() {
    let old = include_str!("fixtures/state-before-unpushed-base.json");
    let state = State::from_json(old).unwrap();
    assert!(state.teams().iter().all(|team| team.unpushed_base().is_none()));
    let mut written: serde_json::Value = serde_json::from_str(&state.to_json().unwrap()).unwrap();
    assert_eq!(written["schema_version"], 13);
    written["schema_version"] = serde_json::json!(11);
    assert_eq!(written, serde_json::from_str::<serde_json::Value>(old).unwrap());
    let mut team = state.teams()[0].clone();
    let evidence = UnpushedBase {
        branch: "main".parse().unwrap(),
        count: NonZeroU64::new(2).unwrap(),
        first: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".parse().unwrap(),
        subject: "Żółć unpublished".to_owned(),
    };
    team.record_unpushed_base(evidence.clone());
    assert_eq!(team.unpushed_base(), Some(&evidence));
    let loaded: agent_core::state::Team = serde_json::from_value(serde_json::to_value(&team).unwrap()).unwrap();
    assert_eq!(loaded, team);
    assert_eq!(loaded.unpushed_base(), Some(&evidence));
    let mut bad = serde_json::to_value(evidence).unwrap();
    bad["count"] = serde_json::json!(0);
    assert!(matches!(serde_json::from_value::<UnpushedBase>(bad), Err(error) if error.is_data()));
}
