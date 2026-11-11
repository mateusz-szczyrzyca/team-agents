```compile_fail
# use agent_core::state::State;
# let mut state = State::from_json(include_str!("../tests/fixtures/state-v1.json")).unwrap();
# let team = state.teams()[0].id().clone();
# let other = state.teams()[1].clone();
let task = state.teams()[0].crew().roster()[0].1.task().clone();
let member = other.crew().roster()[0].1.clone();
*state.find_task_mut(&task).unwrap().1 = member;
```
