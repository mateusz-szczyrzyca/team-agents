//! Boundary and exemption contracts for idle removal.

use super::*;

#[test]
fn idle_decision_table_covers_the_boundary_and_every_exemption() {
    let last: Timestamp = "2026-11-23T00:00:00Z".parse().unwrap();
    for (clock, minutes, hold, expected) in [
        ("2026-11-23T01:59:59Z", 120, Hold::None, Decision::Keep),
        ("2026-11-23T02:00:00Z", 120, Hold::None, Decision::Remove),
        ("2026-11-23T02:00:01Z", 120, Hold::None, Decision::Remove),
        ("2026-11-24T00:00:00Z", 0, Hold::None, Decision::Keep),
        ("2026-11-23T03:00:00Z", 120, Hold::Running, Decision::Keep),
        ("2026-11-23T03:00:00Z", 120, Hold::Human, Decision::Keep),
        ("2026-11-23T03:00:00Z", 120, Hold::Provider, Decision::Keep),
        ("2026-11-23T03:00:00Z", 120, Hold::ExternalMerge, Decision::Keep),
        ("2026-11-22T23:59:59Z", 120, Hold::None, Decision::Keep),
    ] {
        assert_eq!(
            decide(clock.parse().unwrap(), last, Duration::from_secs(minutes * 60), hold),
            expected,
            "{clock}: {hold:?}"
        );
    }
}

#[test]
fn a_new_commit_moves_the_idle_boundary_forward() {
    let now = "2026-11-23T03:00:00Z".parse().unwrap();
    let limit = Duration::from_secs(7200);
    assert_eq!(
        decide(now, "2026-11-23T00:00:00Z".parse().unwrap(), limit, Hold::None),
        Decision::Remove
    );
    assert_eq!(
        decide(now, "2026-11-23T02:30:00Z".parse().unwrap(), limit, Hold::None),
        Decision::Keep
    );
}
