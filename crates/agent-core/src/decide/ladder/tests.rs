use super::*;

/// A configuration that allows one new session per task.
const ONE: Renewals = Renewals { used: 0, allowed: 1 };

/// The retries of a hang when the watchdog gives three strikes.
const STRIKES: u32 = 3;

/// The turn-limit resumes of the default configuration.
const TURN_LIMIT_RESUMES: u32 = 3;

/// The roles that work a task of their own and may be started in a new session.
const WORKING_ROLES: [Role; 3] = [Role::Solo, Role::Coordinator, Role::Member];

/// The step after `count` runs of `role` that went bad with `trouble`, in `session` or not.
fn step(role: Role, trouble: Trouble, count: u32, session: Option<&str>) -> LadderStep {
    _stub_next_step(role, trouble, count, session, ONE, STRIKES, TURN_LIMIT_RESUMES)
}

#[test]
fn no_status_retries_the_same_session_up_to_the_retries() {
    for role in WORKING_ROLES {
        assert_eq!(
            step(role, Trouble::NoStatus, 1, Some("s")),
            LadderStep::SameSession,
            "{role:?}"
        );
        assert_eq!(
            step(role, Trouble::NoStatus, 2, Some("s")),
            LadderStep::SameSession,
            "{role:?}"
        );
    }
}

#[test]
fn no_status_beyond_the_retries_starts_a_new_session() {
    for role in WORKING_ROLES {
        assert_eq!(
            step(role, Trouble::NoStatus, 3, Some("s")),
            LadderStep::NewSession,
            "{role:?}"
        );
    }
}

#[test]
fn a_hang_resumes_the_same_session_until_the_last_strike() {
    for role in WORKING_ROLES {
        assert_eq!(
            step(role, Trouble::Hang, 1, Some("s")),
            LadderStep::SameSession,
            "{role:?}"
        );
        assert_eq!(
            step(role, Trouble::Hang, 2, Some("s")),
            LadderStep::SameSession,
            "{role:?}"
        );
        assert_eq!(
            step(role, Trouble::Hang, 3, Some("s")),
            LadderStep::NewSession,
            "{role:?}"
        );
    }
}

#[test]
fn a_worker_or_a_reviewer_is_never_started_in_a_new_session() {
    for role in [Role::Worker, Role::Reviewer] {
        for trouble in [Trouble::NoStatus, Trouble::Hang] {
            assert_eq!(
                step(role, trouble, 1, Some("s")),
                LadderStep::GiveUp,
                "{role:?} {trouble:?}"
            );
            assert_eq!(
                step(role, trouble, 3, Some("s")),
                LadderStep::GiveUp,
                "{role:?} {trouble:?}"
            );
        }
    }
}

#[test]
fn a_task_without_a_session_starts_a_new_one_at_once() {
    for role in WORKING_ROLES {
        for trouble in [Trouble::NoStatus, Trouble::Hang] {
            assert_eq!(
                step(role, trouble, 1, None),
                LadderStep::NewSession,
                "{role:?} {trouble:?}"
            );
        }
    }
}

#[test]
fn the_new_session_is_started_once_per_task() {
    let used = Renewals { used: 1, allowed: 1 };

    assert_eq!(
        _stub_next_step(
            Role::Member,
            Trouble::NoStatus,
            4,
            Some("s"),
            used,
            STRIKES,
            TURN_LIMIT_RESUMES
        ),
        LadderStep::GiveUp
    );
    assert_eq!(
        _stub_next_step(
            Role::Solo,
            Trouble::NoStatus,
            3,
            None,
            used,
            STRIKES,
            TURN_LIMIT_RESUMES
        ),
        LadderStep::GiveUp
    );
}

#[test]
fn a_limit_of_zero_keeps_the_retries_of_the_same_session_only() {
    let off = Renewals { used: 0, allowed: 0 };
    let after = |trouble, count, session| {
        _stub_next_step(
            Role::Coordinator,
            trouble,
            count,
            session,
            off,
            STRIKES,
            TURN_LIMIT_RESUMES,
        )
    };

    assert_eq!(after(Trouble::NoStatus, 2, Some("s")), LadderStep::SameSession);
    assert_eq!(after(Trouble::NoStatus, 3, Some("s")), LadderStep::GiveUp);
    assert_eq!(after(Trouble::Hang, 2, Some("s")), LadderStep::SameSession);
    assert_eq!(after(Trouble::Hang, 3, Some("s")), LadderStep::GiveUp);
    assert_eq!(after(Trouble::NoStatus, 1, None), LadderStep::GiveUp);
}

#[test]
fn a_longer_limit_allows_more_new_sessions() {
    let twice = Renewals { used: 1, allowed: 2 };

    assert_eq!(
        _stub_next_step(
            Role::Member,
            Trouble::NoStatus,
            3,
            Some("s"),
            twice,
            STRIKES,
            TURN_LIMIT_RESUMES
        ),
        LadderStep::NewSession
    );
    assert_eq!(
        _stub_next_step(
            Role::Member,
            Trouble::NoStatus,
            5,
            Some("s"),
            Renewals { used: 2, allowed: 2 },
            STRIKES,
            TURN_LIMIT_RESUMES
        ),
        LadderStep::GiveUp
    );
}

#[test]
fn a_turn_limit_is_resumed_up_to_its_limit_and_then_starts_a_new_session() {
    assert_eq!(
        step(Role::Member, Trouble::TurnLimit, 3, Some("s")),
        LadderStep::SameSession
    );
    assert_eq!(
        step(Role::Member, Trouble::TurnLimit, 4, Some("s")),
        LadderStep::NewSession
    );
    assert_eq!(step(Role::Worker, Trouble::TurnLimit, 4, Some("s")), LadderStep::GiveUp);
}
