//! A task whose session went bad is started in a new session again: the ladder decides, the state records the renewal,
//! and recovery keeps both across a restart of the daemon.

use agent_core::config::{Limits, Watchdog};
use agent_core::decide::ladder::{LadderStep, Renewals, Trouble};
use agent_core::state::SessionId;
use agent_core::status::TaskStatus;

use crate::fixtures::{at, run, task, team_state};

/// The session the runs of these tests begin in.
fn session() -> Option<SessionId> {
    Some("00000000-0000-4000-8000-000000000001".parse().unwrap())
}

#[test]
fn after_the_retries_of_the_same_session_a_task_is_started_anew_once() {
    let mut state = team_state();
    let mut member = state.find_task_mut(&task(2)).unwrap().1;
    member.transition(TaskStatus::Running).unwrap();
    member.begin_run(run(1), at(10), session()).unwrap();

    let step = member.renew_after(Trouble::NoStatus, at(20), renewals(1), &watchdog(), &limits());

    assert_eq!(step, Ok(LadderStep::NewSession));
    assert_eq!(
        member.last_session(),
        None,
        "the session before the renewal is not resumed"
    );
    assert_eq!(member.renewals().map(|used| used.used()), Some(1));
}

#[test]
fn a_renewal_that_a_limit_forbids_is_not_recorded() {
    let mut state = team_state();
    let mut member = state.find_task_mut(&task(2)).unwrap().1;
    member.transition(TaskStatus::Running).unwrap();
    member.begin_run(run(1), at(10), session()).unwrap();

    let step = member.renew_after(Trouble::NoStatus, at(20), renewals(0), &watchdog(), &limits());

    assert_eq!(step, Ok(LadderStep::GiveUp));
    assert_eq!(member.last_session(), session().as_ref());
    assert_eq!(member.renewals(), None);
}

/// `allowed` new sessions per task.
fn renewals(allowed: u32) -> Renewals {
    Renewals { used: 0, allowed }
}

/// The watchdog of these tests: three strikes.
fn watchdog() -> Watchdog {
    Watchdog {
        hang_strikes: std::num::NonZeroU32::new(3).unwrap(),
        ..Watchdog::default()
    }
}

/// The limits of these tests.
fn limits() -> Limits {
    Limits::default()
}
