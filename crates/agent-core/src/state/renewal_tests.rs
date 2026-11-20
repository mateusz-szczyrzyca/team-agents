//! A task started again in a new session: what the state keeps of it, and how it reads a state written before it.

use super::*;
use crate::decide::ladder::Trouble;
use crate::status::{RunOutcome, TaskStatus};

/// The session the member of these tests ran in.
const OLD_SESSION: &str = "5f3c9a3e-86f3-4b0e-9d2c-0a8f1c7d5b21";

/// The task of these tests: its member ended seven runs without a status and is failed.
fn task() -> TaskId {
    "task-20261120-144101-r7q2".parse().unwrap()
}

/// A state written by `main` before renewals existed, with that failed member in it.
fn old_state() -> State {
    State::from_json(include_str!("../../tests/fixtures/state-before-renewals.json")).expect("a state of main loads")
}

/// When these tests' runs start.
fn started() -> Timestamp {
    "2026-11-20T20:00:00Z".parse().unwrap()
}

#[test]
fn a_failed_member_of_main_with_the_retries_used_loads_without_a_renewal_limit() {
    let state = old_state();
    let found = state.find_task(&task()).expect("the failed task is there");

    assert_eq!(found.member.status(), TaskStatus::Failed);
    assert_eq!(found.member.renewals(), None);
    assert_eq!(found.member.last_session().map(SessionId::as_str), Some(OLD_SESSION));
    assert_eq!(found.member.runs().len(), 7);
}

/// Puts the failed task of the old state in its session again, with a run that has ended done: the state the
/// daemon reaches just before it starts a task anew.
fn renewed(member: &mut Member) {
    let renewed = member
        .renew(Trouble::NoStatus, started())
        .expect("a failed task may be started anew");
    assert!(renewed);
    member
        .transition(TaskStatus::Queued)
        .expect("a failed task may be queued");
    member.transition(TaskStatus::Running).expect("a queued task runs");
    member
        .begin_run("run-20261120-200001-n1n1".parse().unwrap(), started(), None)
        .expect("a run starts");
}

#[test]
fn a_task_started_anew_forgets_the_session_of_its_runs_before_and_keeps_the_new_one() {
    let mut state = old_state();
    let (_, mut member) = state.find_task_mut(&task()).expect("the task");
    renewed(member.value);

    // The new run began in no session that the CLI named yet: nothing is resumed.
    assert_eq!(member.last_session(), None);
    let new_session: SessionId = "00000000-0000-4000-8000-000000000002".parse().unwrap();
    member
        .attach_session(new_session.clone())
        .expect("the CLI names its session");
    member
        .end_run(
            started(),
            RunEnd::Reported {
                outcome: RunOutcome::Done,
            },
        )
        .expect("the run ends");

    assert_eq!(member.last_session(), Some(&new_session));
    assert_eq!(member.renewals().map(Renewals::used), Some(1), "the renewal is counted");
}

#[test]
fn a_renewal_is_kept_and_round_trips_through_the_state() {
    let mut state = old_state();
    renewed(state.find_task_mut(&task()).expect("the task").1.value);
    let saved = state.to_json().expect("the state saves");
    let loaded = State::from_json(&saved).expect("it loads again");

    let found = loaded.find_task(&task()).expect("the task");
    assert_eq!(found.member.renewals().map(Renewals::used), Some(1));
    assert!(found.member.last_session().is_none());
}

#[test]
fn a_task_started_anew_before_its_last_run_started_is_refused() {
    let mut state = old_state();
    let (_, mut member) = state.find_task_mut(&task()).expect("the task");
    renewed(member.value);
    member
        .end_run(started(), RunEnd::NoStatus { exit_code: Some(0) })
        .expect("the run ends");

    let refused = member.renew(Trouble::Hang, "2026-11-20T19:59:59Z".parse().unwrap());

    assert!(matches!(refused, Err(RenewalError::NotAfterTheLastRun { .. })));
}

#[test]
fn a_state_whose_renewal_does_not_follow_its_run_is_refused() {
    let mut state = old_state();
    renewed(state.find_task_mut(&task()).expect("the task").1.value);
    let mut written: serde_json::Value = serde_json::from_str(&state.to_json().expect("saves")).expect("json");
    let teams = written.get_mut("teams").expect("the teams");
    let team = teams.get_mut(0).expect("the team");
    let crew = team.get_mut("crew").expect("the crew");
    let members = crew.get_mut("members").expect("the members");
    let member = members.get_mut(0).expect("the member");
    let renewals = member.get_mut("renewals").expect("the renewals");
    // Before the first run of the task: no run went bad before it.
    *renewals.get_mut("at").expect("the renewal's time") = serde_json::Value::from("2026-11-20T14:00:00Z");

    let refused = State::from_json(&written.to_string());

    assert!(
        matches!(&refused, Err(StateError::Malformed { .. })),
        "the renewal precedes every run: {refused:?}"
    );
}

/// The old state's failed task, started anew at `renewed_at`: the ladder's decision, before its new run begins.
fn renewed_at(state: &mut State, renewed_at: Timestamp) {
    let (_, mut member) = state.find_task_mut(&task()).expect("the task");
    let renewed = member
        .renew(Trouble::NoStatus, renewed_at)
        .expect("a failed task may be started anew");
    assert!(renewed);
}

#[test]
fn the_run_of_a_new_session_records_the_session_the_harness_named_for_it() {
    let mut state = old_state();
    renewed_at(&mut state, started());
    let (_, mut member) = state.find_task_mut(&task()).expect("the task");
    member.transition(TaskStatus::Queued).expect("queued");
    member.transition(TaskStatus::Running).expect("runs");
    let named: SessionId = "00000000-0000-4000-8000-000000000003".parse().unwrap();

    let begun = member.begin_run(
        "run-20261120-200101-n2n2".parse().unwrap(),
        "2026-11-20T20:01:01Z".parse().unwrap(),
        Some(named.clone()),
    );

    assert_eq!(begun, Ok(()));
    assert_eq!(member.last_session(), Some(&named));
}

#[test]
fn a_state_saved_between_a_renewal_and_the_run_of_its_new_session_loads() {
    let mut state = old_state();
    renewed_at(&mut state, started());
    let saved = state.to_json().expect("the state saves");

    let loaded = State::from_json(&saved).expect("a restart reads it");

    let found = loaded.find_task(&task()).expect("the task");
    assert_eq!(found.member.renewals().map(Renewals::at), Some(started()));
    assert_eq!(found.member.last_session(), None, "the new session resumes nothing");
}

#[test]
fn a_renewal_keeps_why_the_task_was_started_anew_across_a_restart() {
    let mut state = old_state();
    let (_, mut member) = state.find_task_mut(&task()).expect("the task");
    member
        .renew(Trouble::Hang, started())
        .expect("a failed task may be started anew");
    let saved = state.to_json().expect("the state saves");

    let loaded = State::from_json(&saved).expect("a restart reads it");

    let found = loaded.find_task(&task()).expect("the task");
    assert_eq!(found.member.renewals().map(Renewals::reason), Some(Trouble::Hang));
}

#[test]
fn a_task_whose_bad_run_started_at_the_instant_of_the_renewal_starts_a_new_session() {
    let mut state = old_state();
    let (_, mut member) = state.find_task_mut(&task()).expect("the task");
    let last_start = member.runs().last().expect("a run").started_at();

    let renewed = member.renew(Trouble::NoStatus, last_start);

    assert_eq!(renewed, Ok(true));
    assert_eq!(member.last_session(), None, "the bad run's session is not resumed");
    assert_eq!(member.bad_runs_since_renewal(), 0);
}

#[test]
fn the_runs_of_the_session_are_those_since_the_task_was_last_started_anew() {
    let mut state = old_state();
    let (_, mut member) = state.find_task_mut(&task()).expect("the task");
    assert_eq!(
        member.session_runs().len(),
        7,
        "without a renewal every run is of the session"
    );
    let last_start = member.runs().last().expect("a run").started_at();
    member.renew(Trouble::NoStatus, last_start).expect("renewed");
    assert_eq!(member.session_runs().len(), 0);
    member.transition(TaskStatus::Queued).expect("queued");
    member.transition(TaskStatus::Running).expect("runs");
    member
        .begin_run("run-20261120-200101-n3n3".parse().unwrap(), started(), None)
        .expect("a run starts");

    let runs: Vec<_> = member.session_runs().iter().map(|run| run.id().to_string()).collect();

    assert_eq!(runs, ["run-20261120-200101-n3n3"]);
}

#[test]
fn a_reviewer_and_a_worker_are_never_started_anew_and_a_member_is() {
    let mut state = old_state();
    let reviewer: TaskId = "task-20261111-193007-k3x9".parse().unwrap();
    let worker: TaskId = "task-20261111-193003-k3x9".parse().unwrap();
    let late: Timestamp = "2026-11-21T00:00:00Z".parse().unwrap();

    let refused = state.renew_task(&reviewer, Trouble::NoStatus, late);
    let refused_worker = state.renew_task(&worker, Trouble::NoStatus, late);
    let renewed = state.renew_task(&task(), Trouble::NoStatus, late);

    assert!(
        matches!(&refused, Err(RenewTaskError::Role { task, role: crate::kinds::Role::Reviewer }) if *task == reviewer),
        "{refused:?}"
    );
    assert!(
        matches!(
            &refused_worker,
            Err(RenewTaskError::Role {
                role: crate::kinds::Role::Worker,
                ..
            })
        ),
        "{refused_worker:?}"
    );
    assert!(matches!(renewed, Ok(true)), "{renewed:?}");
    let found = state.find_task(&reviewer).expect("the reviewer");
    assert_eq!(found.member.renewals(), None);
}
