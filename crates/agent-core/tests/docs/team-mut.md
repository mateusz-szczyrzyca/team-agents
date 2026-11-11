```compile_fail
# use agent_core::state::State;
# let mut state = State::from_json(include_str!("../tests/fixtures/state-v1.json")).unwrap();
# let team = state.teams()[0].id().clone();
# let other = state.teams()[1].clone();
*state.team_mut(&team).unwrap() = other;
```

```compile_fail
# use agent_core::state::State;
# let mut state = State::from_json(include_str!("../tests/fixtures/state-v1.json")).unwrap();
# let team = state.teams()[0].id().clone();
# let other = state.teams()[1].clone();
let _ = std::mem::replace(&mut *state.team_mut(&team).unwrap(), other);
```

```compile_fail
# use agent_core::state::State;
# let mut state = State::from_json(include_str!("../tests/fixtures/state-v1.json")).unwrap();
# let team = state.teams()[0].id().clone();
# let other = state.teams()[1].clone();
let item = other.items()[0].clone();
let id = item.id();
*state.team_mut(&team).unwrap().item_mut(id).unwrap() = item;
```

```compile_fail
# use agent_core::state::State;
# let mut state = State::from_json(include_str!("../tests/fixtures/state-v1.json")).unwrap();
# let team = state.teams()[0].id().clone();
# let other = state.teams()[1].clone();
let crew = other.crew().clone();
*state.team_mut(&team).unwrap().crew_mut() = crew;
```

```compile_fail
# use agent_core::state::State;
# let mut state = State::from_json(include_str!("../tests/fixtures/state-v1.json")).unwrap();
# let team = state.teams()[0].id().clone();
# let other = state.teams()[1].clone();
let reviewer = state.teams()[0].reviewers()[0].clone();
state.team_mut(&team).unwrap().reviewers_mut()[0] = reviewer;
```
