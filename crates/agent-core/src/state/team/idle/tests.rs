//! Durable activity and pre-idle compatibility contracts.
use super::*;
use crate::state::{State, fixtures};

#[test]
fn observations_keep_last_activity_and_reset_refusal_only_for_new_work() {
    let heads = BTreeMap::from([(
        "branch".into(),
        "1111111111111111111111111111111111111111".parse().unwrap(),
    )]);
    let first = IdleWatch::observe(None, fixtures::at(0), fixtures::at(8000), heads.clone());
    assert_eq!(first.last_activity, fixtures::at(0));
    let previous = IdleWatch {
        refusal: Some("cannot bundle".into()),
        ..first
    };
    let unchanged = IdleWatch::observe(Some(&previous), fixtures::at(0), fixtures::at(9000), heads);
    assert_eq!(unchanged, previous);
    let changed = BTreeMap::from([(
        "branch".into(),
        "2222222222222222222222222222222222222222".parse().unwrap(),
    )]);
    let new_work = IdleWatch::observe(Some(&previous), fixtures::at(0), fixtures::at(9000), changed);
    assert_eq!(new_work.last_activity, fixtures::at(9000));
    assert_eq!(new_work.refusal, None);
    let deleted = IdleWatch::observe(Some(&new_work), fixtures::at(0), fixtures::at(9500), BTreeMap::new());
    assert_eq!(deleted.last_activity, fixtures::at(9000));
    let run = IdleWatch::observe(
        Some(&previous),
        fixtures::at(9100),
        fixtures::at(9200),
        previous.heads.clone(),
    );
    assert_eq!(run.last_activity, fixtures::at(9100));
    assert_eq!(run.refusal, None);
}

#[test]
fn pre_idle_state_loads_and_observations_survive_serialization() {
    let old = include_str!("legacy.json");
    let mut state = State::from_json(old).unwrap();
    let team_id = state.teams().first().unwrap().id().clone();
    let created = state.team(&team_id).unwrap().created_at();
    assert_eq!(state.team(&team_id).unwrap().idle_watch(), None);
    let watch = IdleWatch {
        last_activity: created,
        heads: BTreeMap::new(),
        refusal: Some("kept".into()),
    };
    state.team_mut(&team_id).unwrap().record_idle_watch(watch.clone());
    let restored = State::from_json(&state.to_json().unwrap()).unwrap();
    assert_eq!(restored.team(&team_id).unwrap().idle_watch(), Some(&watch));
}
