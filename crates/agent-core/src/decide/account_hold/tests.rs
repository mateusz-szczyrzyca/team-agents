//! Table tests of the hold of an account: which readings hold it and until when.

use crate::config::HoldAt;
use crate::limit_reading::{LimitReading, LimitWindow};
use crate::state::{KeptReading, Timestamp};

use super::{Hold, hold};

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
