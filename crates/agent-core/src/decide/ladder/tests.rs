use std::num::NonZeroU32;

use super::*;

/// A configuration that allows one new session per task.
const ONE: Renewals = Renewals { used: 0, allowed: 1 };

/// A watchdog that gives three strikes before a hang is the last one.
fn watchdog() -> Watchdog {
    Watchdog {
        hang_strikes: NonZeroU32::new(3).unwrap(),
        ..Watchdog::default()
    }
}

/// A configuration whose turn budget is resumed three times.
fn limits() -> Limits {
    Limits {
        turn_limit_resumes: 3,
        ..Limits::default()
    }
}

/// The roles that work a task of their own and may be started in a new session.
const WORKING_ROLES: [Role; 3] = [Role::Solo, Role::Coordinator, Role::Member];

/// The step after `count` runs of `role` that went bad with `trouble`, in its session or not.
fn step(role: Role, trouble: Trouble, count: u32, has_session: bool) -> LadderStep {
    let session = has_session.then_some(session());
    next_step(role, trouble, count, session.as_ref(), ONE, &watchdog(), &limits())
}

/// The session these tests call `s`.
fn session() -> SessionId {
    "00000000-0000-4000-8000-000000000001".parse().unwrap()
}

#[test]
fn no_status_retries_the_same_session_up_to_the_retries() {
    for role in WORKING_ROLES {
        assert_eq!(
            step(role, Trouble::NoStatus, 1, true),
            LadderStep::SameSession,
            "{role:?}"
        );
        assert_eq!(
            step(role, Trouble::NoStatus, 2, true),
            LadderStep::SameSession,
            "{role:?}"
        );
    }
}

#[test]
fn no_status_beyond_the_retries_starts_a_new_session() {
    for role in WORKING_ROLES {
        assert_eq!(
            step(role, Trouble::NoStatus, 3, true),
            LadderStep::NewSession,
            "{role:?}"
        );
    }
}

#[test]
fn a_hang_resumes_the_same_session_until_the_last_strike() {
    for role in WORKING_ROLES {
        assert_eq!(step(role, Trouble::Hang, 1, true), LadderStep::SameSession, "{role:?}");
        assert_eq!(step(role, Trouble::Hang, 2, true), LadderStep::SameSession, "{role:?}");
        assert_eq!(step(role, Trouble::Hang, 3, true), LadderStep::NewSession, "{role:?}");
    }
}

#[test]
fn a_worker_or_a_reviewer_is_never_started_in_a_new_session() {
    for role in [Role::Worker, Role::Reviewer] {
        // A run without a status is never asked again for them, as before.
        assert_eq!(step(role, Trouble::NoStatus, 1, true), LadderStep::GiveUp, "{role:?}");
        assert_eq!(step(role, Trouble::NoStatus, 3, true), LadderStep::GiveUp, "{role:?}");
        // A hang they keep resuming in their session, which the ladder does not take from them.
        assert_eq!(step(role, Trouble::Hang, 1, true), LadderStep::SameSession, "{role:?}");
        assert_eq!(step(role, Trouble::Hang, 2, true), LadderStep::SameSession, "{role:?}");
        assert_eq!(step(role, Trouble::Hang, 3, true), LadderStep::GiveUp, "{role:?}");
    }
}

#[test]
fn a_task_without_a_session_starts_a_new_one_at_once() {
    for role in WORKING_ROLES {
        for trouble in [Trouble::NoStatus, Trouble::Hang] {
            assert_eq!(
                step(role, trouble, 1, false),
                LadderStep::NewSession,
                "{role:?} {trouble:?}"
            );
        }
    }
}

#[test]
fn the_new_session_is_started_once_per_task() {
    let used = Renewals { used: 1, allowed: 1 };
    let session = session();

    assert_eq!(
        next_step(
            Role::Member,
            Trouble::NoStatus,
            4,
            Some(&session),
            used,
            &watchdog(),
            &limits()
        ),
        LadderStep::GiveUp
    );
    assert_eq!(
        next_step(Role::Solo, Trouble::NoStatus, 3, None, used, &watchdog(), &limits()),
        LadderStep::GiveUp
    );
}

#[test]
fn a_limit_of_zero_keeps_the_retries_of_the_same_session_only() {
    let off = Renewals { used: 0, allowed: 0 };
    let session = session();
    let after = |trouble, count, has_session: bool| {
        let session = has_session.then(|| session.clone());
        next_step(
            Role::Coordinator,
            trouble,
            count,
            session.as_ref(),
            off,
            &watchdog(),
            &limits(),
        )
    };

    assert_eq!(after(Trouble::NoStatus, 2, true), LadderStep::SameSession);
    assert_eq!(after(Trouble::NoStatus, 3, true), LadderStep::GiveUp);
    assert_eq!(after(Trouble::Hang, 2, true), LadderStep::SameSession);
    assert_eq!(after(Trouble::Hang, 3, true), LadderStep::GiveUp);
    assert_eq!(after(Trouble::NoStatus, 1, false), LadderStep::GiveUp);
}

#[test]
fn a_longer_limit_allows_more_new_sessions() {
    let twice = Renewals { used: 1, allowed: 2 };
    let session = session();

    assert_eq!(
        next_step(
            Role::Member,
            Trouble::NoStatus,
            3,
            Some(&session),
            twice,
            &watchdog(),
            &limits()
        ),
        LadderStep::NewSession
    );
    assert_eq!(
        next_step(
            Role::Member,
            Trouble::NoStatus,
            5,
            Some(&session),
            Renewals { used: 2, allowed: 2 },
            &watchdog(),
            &limits()
        ),
        LadderStep::GiveUp
    );
}

#[test]
fn a_turn_limit_is_resumed_up_to_its_limit_and_then_starts_a_new_session() {
    assert_eq!(step(Role::Member, Trouble::TurnLimit, 3, true), LadderStep::SameSession);
    assert_eq!(step(Role::Member, Trouble::TurnLimit, 4, true), LadderStep::NewSession);
    assert_eq!(step(Role::Worker, Trouble::TurnLimit, 4, true), LadderStep::GiveUp);
}
