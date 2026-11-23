//! The table of the idleness decision: activity below, at and above the bound, a bound of zero, every state that keeps
//! a team alive, a wait for another team, an open question that does not, and a last activity in the future after the
//! clock went back.

use std::time::Duration;

use crate::decide::idle::{Decision, Why};
use crate::decide::idle_look::{Holds, Look};
use crate::state::Timestamp;
use crate::state::fixtures::at;

/// The moment `seconds` after the fixture's epoch.
fn when(seconds: i64) -> Timestamp {
    let moment = at(0)
        .as_utc()
        .checked_add(time::Duration::seconds(seconds))
        .expect("the time of a test fits a timestamp");
    Timestamp::from_utc(moment).expect("the time of a test fits a timestamp")
}

/// The two hours of `[limits] idle_team_minutes` by default.
fn two_hours() -> Duration {
    Duration::from_secs(120 * 60)
}

/// A team that did nothing since the epoch and has nothing that holds it.
fn quiet() -> Look {
    Look {
        team: "team-20260101-000000-aaaa".parse().expect("the team of a test is valid"),
        at: when(0),
        last: when(0),
        holds: Holds::Nothing,
        waits_for_another_team: false,
    }
}

#[test]
fn a_team_whose_last_activity_is_below_the_bound_is_kept_and_removed_at_and_above_it() {
    let below = Look {
        at: when(120 * 60),
        last: when(1),
        ..quiet()
    };
    let at_bound = Look {
        at: when(120 * 60),
        ..quiet()
    };
    let above = Look {
        at: when(120 * 60 + 1),
        ..quiet()
    };

    assert_eq!(super::decide(&below, two_hours()), Decision::Keep(Why::RecentlyActive));
    assert_eq!(super::decide(&at_bound, two_hours()), Decision::Remove);
    assert_eq!(super::decide(&above, two_hours()), Decision::Remove);
}

#[test]
fn a_bound_of_zero_never_makes_a_team_idle() {
    let long_idle = Look {
        at: when(10_000),
        ..quiet()
    };

    assert_eq!(super::decide(&long_idle, Duration::ZERO), Decision::Keep(Why::RecentlyActive));
}

#[test]
fn an_activity_ahead_of_the_look_after_the_clock_went_back_keeps_the_team() {
    let look = Look {
        at: when(0),
        last: when(60),
        ..quiet()
    };

    assert_eq!(super::decide(&look, two_hours()), Decision::Keep(Why::RecentlyActive));
}

#[test]
fn a_run_a_quota_wait_or_the_human_keeps_a_team_that_did_nothing_for_long() {
    let idle = Look {
        at: when(10_000),
        ..quiet()
    };
    let holds = [
        (Holds::Running, Why::Running),
        (Holds::QuotaWait, Why::QuotaWait),
        (Holds::HumanHold, Why::HumanHold),
    ];

    for (hold, why) in holds {
        let held = Look {
            holds: hold,
            ..idle.clone()
        };
        assert_eq!(super::decide(&held, two_hours()), Decision::Keep(why), "{why:?}");
    }
    assert_eq!(super::decide(&idle, two_hours()), Decision::Remove);
}

#[test]
fn a_team_that_waits_for_another_teams_pull_request_is_kept() {
    let waiting = Look {
        at: when(10_000),
        waits_for_another_team: true,
        ..quiet()
    };

    assert_eq!(super::decide(&waiting, two_hours()), Decision::Keep(Why::AnotherTeam));
}

#[test]
fn an_open_question_does_not_keep_a_team_that_nothing_else_holds() {
    let asked = Look {
        at: when(10_000),
        holds: Holds::Nothing,
        ..quiet()
    };

    assert_eq!(
        super::decide(&asked, two_hours()),
        Decision::Remove,
        "an agent that asks and sleeps is the stall the bound exists for"
    );
}
