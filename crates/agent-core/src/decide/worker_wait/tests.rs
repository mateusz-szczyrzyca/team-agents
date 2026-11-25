//! The rule of how long a queued request waits for a worker.
use super::*;

fn at(text: &str) -> Timestamp {
    text.parse().unwrap()
}

#[test]
fn a_request_waits_until_the_limit_is_reached_and_is_then_returned() {
    let since = at("2026-11-25T10:00:00Z");
    let rows = [
        ("2026-11-25T10:09:59Z", 600, WaitVerdict::Waits { seconds: 599 }),
        ("2026-11-25T10:10:00Z", 600, WaitVerdict::Return { seconds: 600 }),
        ("2026-11-25T10:10:01Z", 600, WaitVerdict::Return { seconds: 601 }),
        ("2026-11-25T10:00:00Z", 600, WaitVerdict::Waits { seconds: 0 }),
    ];
    for (now, limit, expected) in rows {
        assert_eq!(wait_verdict(since, at(now), limit), expected, "{now} / {limit}");
    }
}

#[test]
fn a_limit_of_zero_never_returns_the_request() {
    let since = at("2026-11-25T10:00:00Z");
    assert_eq!(
        wait_verdict(since, at("2026-11-25T12:46:40Z"), 0),
        WaitVerdict::Waits { seconds: 10_000 }
    );
}

#[test]
fn a_clock_that_went_back_counts_no_wait() {
    let since = at("2026-11-25T10:00:00Z");
    assert_eq!(
        wait_verdict(since, at("2026-11-25T09:00:00Z"), 600),
        WaitVerdict::Waits { seconds: 0 }
    );
}
