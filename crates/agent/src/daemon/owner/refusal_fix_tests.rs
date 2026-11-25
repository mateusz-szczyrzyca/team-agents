//! Regression coverage for refusal accounting and cooldown selection at daemon boundaries.

use agent_core::state::RunEnd;
use agent_core::status::TaskStatus;

use super::fixture::Daemon;
use crate::fixtures::{at, run, solo_state, task, team_id};

#[tokio::test]
async fn every_non_refusal_run_end_clears_the_previous_count() {
    for end in [RunEnd::NoStatus { exit_code: None }, RunEnd::Stopped] {
        let mut state = solo_state();
        {
            let (_, mut member) = state.find_task_mut(&task(1)).unwrap();
            member.transition(TaskStatus::Running).unwrap();
            member.begin_run(run(1), at(0), None).unwrap();
        }
        state.record_limit_refusals(task(1), 1);
        let daemon = Daemon::new(&state, None);
        let (mut owner, _sinks) = daemon.bare_owner().await;

        owner
            .end_run(&task(1), &team_id(0), at(1), end, false, None, &mut Vec::new())
            .unwrap();

        assert_eq!(owner.parts.state.limit_refusals(&task(1)), 0);
    }
}
