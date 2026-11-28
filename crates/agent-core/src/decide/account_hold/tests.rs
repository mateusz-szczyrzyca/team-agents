//! Table tests of the hold of an account: which readings hold it and until when.

use std::collections::{BTreeMap, BTreeSet};

use crate::config::{AccountName, Config, HoldAt};
use crate::decide::investigator::{free_for_request, refused_investigators};
use crate::ids::InvestigatorName;
use crate::limit_reading::{LimitReading, LimitWindow};
use crate::state::{KeptReading, ReadingOwner, State, Timestamp};

use super::{Hold, held_accounts, held_until, hold, investigators_of_held_accounts};

/// A moment of these tests.
fn at(text: &str) -> Timestamp {
    text.parse().unwrap()
}

/// The moment the hold is decided.
fn now() -> Timestamp {
    at("2026-11-28T10:00:00Z")
}

/// The `hold_at` of these tests: 0.9.
fn hold_at() -> HoldAt {
    HoldAt::new(0.9).unwrap()
}

/// A reading of `window` used by `used`, that resets at `resets_at`, read a minute before [`now`].
fn kept(window: LimitWindow, used: f64, resets_at: &str) -> KeptReading {
    KeptReading::new(
        LimitReading::new(window, used, at(resets_at)).unwrap(),
        at("2026-11-28T09:59:00Z"),
    )
}

#[test]
fn hold_table() {
    let five = LimitWindow::FiveHour;
    let seven = LimitWindow::SevenDay;
    let cases: Vec<(&str, Option<HoldAt>, Vec<KeptReading>, Hold)> = vec![
        (
            "below hold_at",
            Some(hold_at()),
            vec![kept(five, 0.89, "2026-11-28T12:00:00Z")],
            Hold::Open,
        ),
        (
            "at hold_at",
            Some(hold_at()),
            vec![kept(five, 0.9, "2026-11-28T12:00:00Z")],
            Hold::Held {
                window: five,
                used: 0.9,
                until: at("2026-11-28T12:00:00Z"),
            },
        ),
        (
            "above hold_at",
            Some(hold_at()),
            vec![kept(seven, 0.95, "2026-11-30T00:00:00Z")],
            Hold::Held {
                window: seven,
                used: 0.95,
                until: at("2026-11-30T00:00:00Z"),
            },
        ),
        (
            "no hold_at key",
            None,
            vec![kept(five, 1.0, "2026-11-28T12:00:00Z")],
            Hold::Open,
        ),
        ("no reading", Some(hold_at()), vec![], Hold::Open),
        (
            "a reading past its reset",
            Some(hold_at()),
            vec![kept(five, 0.99, "2026-11-28T09:30:00Z")],
            Hold::Open,
        ),
        (
            "a reading that resets this very moment",
            Some(hold_at()),
            vec![kept(five, 0.99, "2026-11-28T10:00:00Z")],
            Hold::Open,
        ),
        (
            "two windows of which one is above",
            Some(hold_at()),
            vec![
                kept(five, 0.2, "2026-11-28T12:00:00Z"),
                kept(seven, 0.93, "2026-11-30T00:00:00Z"),
            ],
            Hold::Held {
                window: seven,
                used: 0.93,
                until: at("2026-11-30T00:00:00Z"),
            },
        ),
        (
            "two windows above: held until the later reset",
            Some(hold_at()),
            vec![
                kept(five, 0.95, "2026-11-28T12:00:00Z"),
                kept(seven, 0.93, "2026-11-30T00:00:00Z"),
            ],
            Hold::Held {
                window: seven,
                used: 0.93,
                until: at("2026-11-30T00:00:00Z"),
            },
        ),
        (
            "the window above has reset, the other is below",
            Some(hold_at()),
            vec![
                kept(five, 0.99, "2026-11-28T09:00:00Z"),
                kept(seven, 0.5, "2026-11-30T00:00:00Z"),
            ],
            Hold::Open,
        ),
        (
            "hold_at 0 holds on any live reading",
            Some(HoldAt::new(0.0).unwrap()),
            vec![kept(five, 0.0, "2026-11-28T12:00:00Z")],
            Hold::Held {
                window: five,
                used: 0.0,
                until: at("2026-11-28T12:00:00Z"),
            },
        ),
    ];
    for (name, hold_at, readings, expected) in cases {
        assert_eq!(hold(hold_at, &readings, now()), expected, "{name}");
    }
}

/// A configuration with two accounts, `comp-claude` held at 0.9 and `other-claude` held at 0.5, and an agent for each.
fn two_accounts() -> Config {
    Config::from_toml_str(
        r#"
[accounts.comp-claude]
owner = "comp"
hold_at = 0.9

[accounts.other-claude]
owner = "other"
hold_at = 0.5

[agents.comp-claude-1]
cli = "claude"
account = "comp-claude"

[agents.other-claude-1]
cli = "claude"
account = "other-claude"

[tiers]
normal = ["comp-claude-1", "other-claude-1"]
"#,
    )
    .unwrap()
}

/// The state with `reading` kept for the account `account`, read a minute before [`now`].
fn state_with(account: &str, reading: LimitReading) -> State {
    let mut state = State::new();
    state.keep_limit_readings(
        &ReadingOwner::Account(account.parse().unwrap()),
        &[reading],
        at("2026-11-28T09:59:00Z"),
        now(),
    );
    state
}

#[test]
fn held_accounts_lists_only_the_account_whose_own_hold_at_is_reached() {
    let reading = LimitReading::new(LimitWindow::FiveHour, 0.6, at("2026-11-28T12:00:00Z")).unwrap();
    let state = state_with("other-claude", reading);

    let held = held_accounts(&state, &two_accounts(), now());

    assert_eq!(
        held.into_iter().collect::<Vec<_>>(),
        vec![(
            "other-claude".parse().unwrap(),
            Hold::Held {
                window: LimitWindow::FiveHour,
                used: 0.6,
                until: at("2026-11-28T12:00:00Z"),
            }
        )]
    );
}

#[test]
fn held_accounts_does_not_hold_an_account_for_the_reading_of_another() {
    let reading = LimitReading::new(LimitWindow::FiveHour, 0.95, at("2026-11-28T12:00:00Z")).unwrap();
    let state = state_with("comp-claude", reading);
    let mut other = State::new();
    other.keep_limit_readings(
        &ReadingOwner::Agent("comp-claude-1".parse().unwrap()),
        &[reading],
        at("2026-11-28T09:59:00Z"),
        now(),
    );

    assert_eq!(held_accounts(&state, &two_accounts(), now()).len(), 1);
    assert_eq!(held_accounts(&other, &two_accounts(), now()), BTreeMap::new());
}

#[test]
fn held_accounts_lets_go_of_an_account_when_its_window_has_reset() {
    let reading = LimitReading::new(LimitWindow::FiveHour, 0.95, at("2026-11-28T12:00:00Z")).unwrap();
    let state = state_with("comp-claude", reading);

    let before = held_accounts(&state, &two_accounts(), at("2026-11-28T11:59:59Z"));
    let at_reset = held_accounts(&state, &two_accounts(), at("2026-11-28T12:00:00Z"));

    assert_eq!(before.len(), 1);
    assert_eq!(at_reset, BTreeMap::new());
}

/// An account of these tests.
fn account(name: &str) -> AccountName {
    name.parse().unwrap()
}

/// A configuration with an investigator for each of the accounts `comp-claude` (held at 0.9) and `other-claude`, and one
/// investigator without an account.
fn with_investigators() -> Config {
    Config::from_toml_str(
        r#"
[accounts.comp-claude]
owner = "comp"
hold_at = 0.9

[accounts.other-claude]
owner = "other"

[agents.comp-claude-1]
cli = "claude"
account = "comp-claude"

[investigators.comp-inv]
cli = "claude"
account = "comp-claude"

[investigators.other-inv]
cli = "claude"
account = "other-claude"

[investigators.free-inv]
cli = "claude"

[tiers]
normal = ["comp-claude-1"]
"#,
    )
    .unwrap()
}

/// The state with the reading of `used` for `comp-claude`, and a recorded hold of it until `until` when there is one.
fn comp_state(used: f64, recorded: Option<&str>) -> State {
    let reading = LimitReading::new(LimitWindow::FiveHour, used, at("2026-11-28T12:00:00Z")).unwrap();
    let mut state = state_with("comp-claude", reading);
    if let Some(until) = recorded {
        state.record_account_hold(account("comp-claude"), Some(at(until)));
    }
    state
}

#[test]
fn a_reading_that_holds_the_account_is_listed_with_its_reset() {
    let state = comp_state(0.95, None);

    assert_eq!(
        held_until(&state, &with_investigators(), now()),
        BTreeMap::from([(account("comp-claude"), at("2026-11-28T12:00:00Z"))])
    );
}

#[test]
fn a_recorded_hold_lasts_until_its_end_after_a_lower_reading() {
    let state = comp_state(0.4, Some("2026-11-28T12:00:00Z"));

    assert_eq!(
        held_until(&state, &with_investigators(), now()),
        BTreeMap::from([(account("comp-claude"), at("2026-11-28T12:00:00Z"))])
    );
}

#[test]
fn a_recorded_hold_ends_at_its_end() {
    let state = comp_state(0.4, Some("2026-11-28T12:00:00Z"));

    assert_eq!(
        held_until(&state, &with_investigators(), at("2026-11-28T11:59:59Z")).len(),
        1
    );
    assert_eq!(
        held_until(&state, &with_investigators(), at("2026-11-28T12:00:00Z")),
        BTreeMap::new()
    );
}

#[test]
fn a_recorded_hold_of_an_account_without_hold_at_or_unknown_holds_nobody() {
    let mut state = State::new();
    state.record_account_hold(account("other-claude"), Some(at("2026-11-28T12:00:00Z")));
    state.record_account_hold(account("not-in-the-config"), Some(at("2026-11-28T12:00:00Z")));

    assert_eq!(held_until(&state, &with_investigators(), now()), BTreeMap::new());
}

#[test]
fn the_later_end_wins_when_a_recorded_hold_and_a_reading_both_hold() {
    let state = comp_state(0.95, Some("2026-11-28T11:00:00Z"));

    assert_eq!(
        held_until(&state, &with_investigators(), now()),
        BTreeMap::from([(account("comp-claude"), at("2026-11-28T12:00:00Z"))])
    );
}

#[test]
fn the_investigators_of_a_held_account_are_those_that_name_it() {
    let state = comp_state(0.95, None);

    assert_eq!(
        investigators_of_held_accounts(&state, &with_investigators(), now()),
        BTreeSet::from(["comp-inv".parse::<InvestigatorName>().unwrap()])
    );
    assert_eq!(
        investigators_of_held_accounts(&State::new(), &with_investigators(), now()),
        BTreeSet::new()
    );
}

#[test]
fn an_investigator_of_a_held_account_is_refused_for_reviews_and_requests() {
    let state = comp_state(0.4, Some("2026-11-28T12:00:00Z"));
    let config = with_investigators();

    let refused = refused_investigators(&state, &config, now());
    let free = free_for_request(&state, &config, now());

    assert_eq!(refused, ["comp-inv".parse::<InvestigatorName>().unwrap()].into());
    assert_eq!(
        free,
        BTreeSet::from(["free-inv", "other-inv"].map(|name| name.parse::<InvestigatorName>().unwrap()))
    );
}
