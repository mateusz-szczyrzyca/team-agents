//! Worked time across successful and refused run endings.

use super::*;
use crate::state::fixtures::{at, member, normal, run};

#[test]
fn completed_runs_add_worked_seconds_once() {
    let mut task = member(1, normal("codex-1"));
    assert_eq!(task.lead_worked_secs(), 0);
    task.begin_run(run(1), at(10), None).unwrap();
    task.end_run(at(70), RunEnd::Stopped).unwrap();
    assert_eq!(task.lead_worked_secs(), 60);
    assert_eq!(task.end_run(at(80), RunEnd::Stopped), Err(RunError::NotRunning));
    task.begin_run(run(2), at(100), None).unwrap();
    assert_eq!(
        task.end_run(at(90), RunEnd::Stopped),
        Err(RunError::EndsBeforeStart {
            run: run(2),
            started_at: at(100),
            at: at(90)
        })
    );
    assert_eq!(task.lead_worked_secs(), 60);
    task.end_run(at(130), RunEnd::Stopped).unwrap();
    assert_eq!(task.lead_worked_secs(), 90);
    let loaded: Member = serde_json::from_str(&serde_json::to_string(&task).unwrap()).unwrap();
    assert_eq!(loaded.lead_worked_secs(), 90);
}

#[test]
fn shift_reason_round_trips_as_shift() {
    let reason = crate::decide::ladder::Trouble::Shift;
    assert_eq!(serde_json::to_string(&reason).unwrap(), "\"shift\"");
    assert_eq!(
        serde_json::from_str::<crate::decide::ladder::Trouble>("\"shift\"").unwrap(),
        reason
    );
}
