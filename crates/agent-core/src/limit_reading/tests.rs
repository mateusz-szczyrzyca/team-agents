use super::{InvalidLimitReading, LimitReading, LimitWindow, ResetTimeError};
use crate::state::Timestamp;

fn at(text: &str) -> Timestamp {
    text.parse().expect("a valid timestamp")
}

#[test]
fn a_share_from_zero_to_one_makes_a_reading_with_the_values_as_given() {
    for used in [0.0, 0.42, 1.0] {
        let reading = LimitReading::new(LimitWindow::FiveHour, used, at("2026-11-12T12:00:00Z")).expect("a share");

        assert_eq!(reading.window(), LimitWindow::FiveHour);
        assert_eq!(reading.used(), used);
        assert_eq!(reading.resets_at(), at("2026-11-12T12:00:00Z"));
    }
}

#[test]
fn a_share_outside_zero_to_one_or_not_a_number_is_refused() {
    for used in [1.7, -0.1, 1.000_001, f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        let error = LimitReading::new(LimitWindow::SevenDay, used, at("2026-11-12T12:00:00Z")).expect_err("refused");

        assert!(
            matches!(error, InvalidLimitReading::UsedOutOfRange { used: refused } if refused.to_bits() == used.to_bits()),
            "{used}: {error:?}"
        );
    }
}

#[test]
fn seconds_since_the_epoch_become_the_reset_time() {
    let reading = LimitReading::from_unix(LimitWindow::SevenDay, 0.5, 1_794_484_800).expect("a reading");

    assert_eq!(reading.window(), LimitWindow::SevenDay);
    assert_eq!(reading.used(), 0.5);
    assert_eq!(reading.resets_at(), at("2026-11-12T12:00:00Z"));
}

#[test]
fn seconds_no_timestamp_holds_are_refused_with_their_cause() {
    let calendar = LimitReading::from_unix(LimitWindow::FiveHour, 0.5, i64::MAX).expect_err("no such time");
    let year = LimitReading::from_unix(LimitWindow::FiveHour, 0.5, -62_167_219_201).expect_err("year -1");

    assert!(
        matches!(
            calendar,
            InvalidLimitReading::ResetOutOfRange {
                seconds: i64::MAX,
                source: ResetTimeError::Calendar(_)
            }
        ),
        "{calendar:?}"
    );
    assert!(
        matches!(
            year,
            InvalidLimitReading::ResetOutOfRange {
                seconds: -62_167_219_201,
                source: ResetTimeError::Year(_)
            }
        ),
        "{year:?}"
    );
}

#[test]
fn a_share_out_of_range_is_refused_before_the_reset_time() {
    let error = LimitReading::from_unix(LimitWindow::FiveHour, 1.7, i64::MAX).expect_err("refused");

    assert!(matches!(error, InvalidLimitReading::UsedOutOfRange { .. }), "{error:?}");
}

#[test]
fn the_windows_order_by_their_length() {
    assert!(LimitWindow::FiveHour < LimitWindow::SevenDay);
}
