use super::{FIRST_PAUSE, LONGEST_PAUSE, pause};
use std::num::NonZeroU32;
use time::Duration;

#[test]
fn the_pause_starts_at_thirty_seconds_doubles_and_stops_at_ten_minutes() {
    let cases = [
        (1, Duration::seconds(30)),
        (2, Duration::seconds(60)),
        (3, Duration::seconds(120)),
        (4, Duration::seconds(240)),
        (5, Duration::seconds(480)),
        (6, Duration::seconds(600)),
        (7, Duration::seconds(600)),
        (u32::MAX, Duration::seconds(600)),
    ];
    for (tries, expected) in cases {
        assert_eq!(pause(NonZeroU32::new(tries).unwrap()), expected, "tries {tries}");
    }
}

#[test]
fn the_first_pause_is_thirty_seconds_and_the_longest_ten_minutes() {
    assert_eq!(FIRST_PAUSE, Duration::seconds(30));
    assert_eq!(LONGEST_PAUSE, Duration::minutes(10));
    assert_eq!(pause(NonZeroU32::MIN), FIRST_PAUSE);
}
