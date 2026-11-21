use std::str::FromStr;

use time::{OffsetDateTime, UtcOffset};

use super::TypedTime;
use super::{TypedTimeError, Zone};
use crate::state::Timestamp;

/// The Unix time of 2026-03-29T01:00:00Z, when the test zone changes to summer time.
const TO_SUMMER: i64 = 1_774_746_000;

/// The Unix time of 2026-10-25T01:00:00Z, when the test zone changes back to winter time.
const TO_WINTER: i64 = 1_792_890_000;

/// A zone of the test that is one hour east of UTC, changing to two hours east between [`TO_SUMMER`] and [`TO_WINTER`].
fn central_european(at: OffsetDateTime) -> Option<UtcOffset> {
    let moment = at.unix_timestamp();
    let hours = if (TO_SUMMER..TO_WINTER).contains(&moment) { 2 } else { 1 };
    UtcOffset::from_hms(hours, 0, 0).ok()
}

/// A zone whose offset cannot be found for any moment.
fn unknown(_at: OffsetDateTime) -> Option<UtcOffset> {
    None
}

/// The moment written as RFC 3339, as the files hold it.
fn stamp(text: &str) -> Timestamp {
    Timestamp::from_str(text).expect("the test moment is valid RFC 3339")
}

#[test]
fn utc_zone_writes_the_rfc3339_text_of_the_files() {
    assert_eq!(Zone::Utc.stamp(stamp("2026-11-21T12:50:02Z")), "2026-11-21T12:50:02Z");
}

#[test]
fn fixed_zones_write_the_local_date_time_and_the_offset() {
    let at = stamp("2026-11-21T12:50:02Z");
    let cases = [
        (UtcOffset::from_hms(1, 0, 0), "2026-11-21 13:50:02 +01:00"),
        (UtcOffset::from_hms(-5, 0, 0), "2026-11-21 07:50:02 -05:00"),
        (UtcOffset::from_hms(5, 30, 0), "2026-11-21 18:20:02 +05:30"),
        (UtcOffset::from_hms(0, 0, 0), "2026-11-21 12:50:02 +00:00"),
    ];
    for (offset, expected) in cases {
        let offset = offset.expect("the test offset is valid");
        assert_eq!(Zone::Fixed(offset).stamp(at), expected, "offset {offset}");
    }
}

#[test]
fn a_lookup_writes_the_offset_valid_at_each_moment() {
    let zone = Zone::Lookup(central_european);
    let cases = [
        // Just before the change to summer time, and just after it.
        ("2026-03-29T00:59:59Z", "2026-03-29 01:59:59 +01:00"),
        ("2026-03-29T01:00:00Z", "2026-03-29 03:00:00 +02:00"),
        // Just before the change back to winter time, and just after it.
        ("2026-10-25T00:59:59Z", "2026-10-25 02:59:59 +02:00"),
        ("2026-10-25T01:00:00Z", "2026-10-25 02:00:00 +01:00"),
        // Midnight crossing a date, in winter and in summer.
        ("2026-11-20T23:30:00Z", "2026-11-21 00:30:00 +01:00"),
        ("2026-07-01T22:30:00Z", "2026-07-02 00:30:00 +02:00"),
    ];
    for (moment, expected) in cases {
        assert_eq!(zone.stamp(stamp(moment)), expected, "moment {moment}");
    }
}

#[test]
fn a_zone_that_cannot_be_found_writes_utc_marked_as_utc() {
    let zone = Zone::Lookup(unknown);
    assert_eq!(zone.stamp(stamp("2026-11-21T12:50:02Z")), "2026-11-21 12:50:02 UTC");
}

#[test]
fn a_text_with_its_zone_is_read_as_written_in_any_zone() {
    let zones = [
        Zone::Utc,
        Zone::Fixed(UtcOffset::from_hms(1, 0, 0).expect("valid")),
        Zone::Lookup(central_european),
    ];
    for zone in zones {
        assert_eq!(
            zone.parse_typed("2026-11-21T12:50:02Z").map(|at| at.to_string()),
            Ok("2026-11-21T12:50:02Z".to_owned())
        );
        assert_eq!(
            zone.parse_typed("2026-11-21T14:50:02+02:00").map(|at| at.to_string()),
            Ok("2026-11-21T12:50:02Z".to_owned())
        );
    }
}

#[test]
fn a_time_without_a_zone_is_read_in_the_zone_given() {
    let cases = [
        (Zone::Utc, "2026-11-21 12:50:02", "2026-11-21T12:50:02Z"),
        (
            Zone::Fixed(UtcOffset::from_hms(1, 0, 0).expect("valid")),
            "2026-11-21 13:50",
            "2026-11-21T12:50:00Z",
        ),
        (
            Zone::Fixed(UtcOffset::from_hms(-5, 0, 0).expect("valid")),
            "2026-11-21T07:50:02",
            "2026-11-21T12:50:02Z",
        ),
        (
            Zone::Lookup(central_european),
            "2026-07-01 02:30",
            "2026-07-01T00:30:00Z",
        ),
        (
            Zone::Lookup(central_european),
            "2026-11-21 13:50:02",
            "2026-11-21T12:50:02Z",
        ),
        (Zone::Lookup(central_european), "2026-11-21", "2026-11-20T23:00:00Z"),
        (
            Zone::Lookup(central_european),
            "2026-11-21 13:50:02.5",
            "2026-11-21T12:50:02.5Z",
        ),
    ];
    for (zone, text, expected) in cases {
        assert_eq!(
            zone.parse_typed(text).map(|at| at.to_string()),
            Ok(expected.to_owned()),
            "text {text}"
        );
    }
}

#[test]
fn a_skipped_hour_is_read_with_the_offset_before_the_change() {
    // 02:30 does not exist on 2026-03-29 in the zone: it is read as 01:30 UTC, which the zone shows as 03:30.
    let zone = Zone::Lookup(central_european);
    let at = zone.parse_typed("2026-03-29 02:30").expect("the time is read");
    assert_eq!(at.to_string(), "2026-03-29T01:30:00Z");
    assert_eq!(zone.stamp(at), "2026-03-29 03:30:00 +02:00");
}

#[test]
fn a_repeated_hour_is_read_at_its_first_occurrence() {
    // 02:30 comes twice on 2026-10-25: the first time is at +02:00, 00:30 UTC.
    let zone = Zone::Lookup(central_european);
    assert_eq!(
        zone.parse_typed("2026-10-25 02:30").map(|at| at.to_string()),
        Ok("2026-10-25T00:30:00Z".to_owned())
    );
}

#[test]
fn a_zone_that_cannot_be_found_reads_a_time_as_utc() {
    assert_eq!(
        Zone::Lookup(unknown)
            .parse_typed("2026-11-21 12:50")
            .map(|at| at.to_string()),
        Ok("2026-11-21T12:50:00Z".to_owned())
    );
}

#[test]
fn a_text_that_is_not_a_time_is_refused_with_the_text() {
    let zone = Zone::Utc;
    assert_eq!(
        zone.parse_typed("21.11.2026"),
        Err(TypedTimeError::NotATime {
            value: "21.11.2026".to_owned()
        })
    );
    assert_eq!(
        zone.parse_typed("2026-11-21 1x:00"),
        Err(TypedTimeError::NotATime {
            value: "2026-11-21 1x:00".to_owned()
        })
    );
}

#[test]
fn a_time_the_calendar_does_not_have_is_refused() {
    let zone = Zone::Utc;
    assert_eq!(
        zone.parse_typed("2026-13-01 10:00"),
        Err(TypedTimeError::NoSuchMoment {
            value: "2026-13-01 10:00".to_owned()
        })
    );
    assert_eq!(
        zone.parse_typed("2026-11-31"),
        Err(TypedTimeError::NoSuchMoment {
            value: "2026-11-31".to_owned()
        })
    );
    assert_eq!(
        zone.parse_typed("2026-11-21 25:00"),
        Err(TypedTimeError::NoSuchMoment {
            value: "2026-11-21 25:00".to_owned()
        })
    );
}

#[test]
fn a_typed_time_is_checked_for_its_form_when_typed_and_read_in_a_zone_when_used() {
    assert_eq!(
        "tomorrow".parse::<TypedTime>(),
        Err(TypedTimeError::NotATime {
            value: "tomorrow".to_owned()
        })
    );
    let typed: TypedTime = "2026-03-29 02:30".parse().expect("the form is valid");
    assert_eq!(
        typed.read_in(Zone::Lookup(central_european)).map(|at| at.to_string()),
        Ok("2026-03-29T01:30:00Z".to_owned())
    );
    assert_eq!(
        typed.read_in(Zone::Utc).map(|at| at.to_string()),
        Ok("2026-03-29T02:30:00Z".to_owned())
    );
}

#[test]
fn a_moment_at_the_edge_of_the_year_range_falls_back_to_utc_marked_when_the_offset_leaves_four_digits() {
    let east = Zone::Fixed(UtcOffset::from_hms(1, 0, 0).expect("valid"));
    let west = Zone::Fixed(UtcOffset::from_hms(-1, 0, 0).expect("valid"));
    assert_eq!(east.stamp(stamp("9999-12-31T23:30:00Z")), "9999-12-31 23:30:00 UTC");
    assert_eq!(west.stamp(stamp("0000-01-01T00:30:00Z")), "0000-01-01 00:30:00 UTC");
}
