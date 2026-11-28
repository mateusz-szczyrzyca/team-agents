use std::collections::BTreeMap;

use agent_core::decide::limit::Limit;
use agent_core::limit_reading::{LimitReading, LimitWindow};
use agent_core::state::{LimitReadings, ReadingOwner};

use super::*;

fn account(name: &str) -> AccountName {
    name.parse().expect("a valid account name")
}

fn at(text: &str) -> Timestamp {
    text.parse().expect("a valid moment")
}

fn lines(paused: &BTreeMap<AccountName, AccountPause>, now: &str) -> Option<String> {
    account_lines(paused.iter(), at(now), Zone::Utc)
}

#[test]
fn paused_account_status_line() {
    let paused = BTreeMap::from([(
        account("comp-claude"),
        AccountPause {
            until: at("2026-11-27T15:30:00Z"),
            reason: Limit::Quota { reset_in: None },
        },
    )]);

    assert_eq!(
        lines(&paused, "2026-11-27T14:00:00Z"),
        Some("account comp-claude paused until 15:30 (quota)\n".to_owned())
    );
}

#[test]
fn no_line_when_no_account_is_paused() {
    assert_eq!(lines(&BTreeMap::new(), "2026-11-27T14:00:00Z"), None);
}

#[test]
fn a_pause_that_has_ended_has_no_line() {
    let paused = BTreeMap::from([(
        account("comp-claude"),
        AccountPause {
            until: at("2026-11-27T15:30:00Z"),
            reason: Limit::RateLimit,
        },
    )]);

    assert_eq!(lines(&paused, "2026-11-27T15:30:00Z"), None);
}

#[test]
fn each_paused_account_has_its_own_line_in_name_order_and_a_far_reset_shows_its_date() {
    let paused = BTreeMap::from([
        (
            account("mateusz-codex"),
            AccountPause {
                until: at("2026-12-01T09:05:00Z"),
                reason: Limit::Quota { reset_in: None },
            },
        ),
        (
            account("comp-claude"),
            AccountPause {
                until: at("2026-11-27T14:02:00Z"),
                reason: Limit::RateLimit,
            },
        ),
    ]);

    let shown = lines(&paused, "2026-11-27T14:00:00Z").expect("two accounts are paused");

    let mut each = shown.lines();
    assert_eq!(each.next(), Some("account comp-claude paused until 14:02 (rate limit)"));
    let second = each.next().expect("the second account");
    assert!(second.starts_with("account mateusz-codex paused until "), "{second}");
    assert!(second.contains("2026-12-01"), "{second}");
    assert!(second.ends_with(" (quota)"), "{second}");
    assert_eq!(each.next(), None);
}

// ---- the readings of the limit windows ------------------------------------------------------------------------------

fn five_hour(used: f64, resets_at: &str) -> LimitReading {
    LimitReading::new(LimitWindow::FiveHour, used, at(resets_at)).unwrap()
}

fn seven_day(used: f64, resets_at: &str) -> LimitReading {
    LimitReading::new(LimitWindow::SevenDay, used, at(resets_at)).unwrap()
}

/// The readings `shown` of `owner`, read at `read_at`, as the state keeps them.
fn kept(owner: &ReadingOwner, shown: &[LimitReading], read_at: &str) -> LimitReadings {
    let mut readings = LimitReadings::default();
    readings.keep(owner, shown, at(read_at), at(read_at));
    readings
}

fn reading_text(readings: &LimitReadings, now: &str) -> Option<String> {
    reading_lines(readings, at(now), Zone::Fixed(time::UtcOffset::UTC))
}

#[test]
fn reading_line_of_an_account_has_both_windows_with_their_reset_and_age() {
    let readings = kept(
        &ReadingOwner::Account(account("comp-claude")),
        &[
            five_hour(0.62, "2026-11-27T15:30:00Z"),
            seven_day(0.41, "2026-12-01T09:05:00Z"),
        ],
        "2026-11-27T13:57:00Z",
    );

    assert_eq!(
        reading_text(&readings, "2026-11-27T14:00:00Z"),
        Some(
            "comp-claude: 5h 62% (resets 15:30, read 3 min ago), 7d 41% (resets 2026-12-01 09:05, read 3 min ago)\n"
                .to_owned()
        )
    );
}

#[test]
fn reading_line_of_an_agent_without_an_account_names_the_agent() {
    let readings = kept(
        &ReadingOwner::Agent("claude-1".parse().unwrap()),
        &[five_hour(0.05, "2026-11-27T15:30:00Z")],
        "2026-11-27T14:00:00Z",
    );

    assert_eq!(
        reading_text(&readings, "2026-11-27T14:00:00Z"),
        Some("claude-1: 5h 5% (resets 15:30, read just now)\n".to_owned())
    );
}

#[test]
fn the_age_of_a_reading_is_in_minutes_then_hours_then_days() {
    let readings = kept(
        &ReadingOwner::Account(account("comp-claude")),
        &[seven_day(1.0, "2026-12-09T00:00:00Z")],
        "2026-11-27T00:00:00Z",
    );

    let ages: Vec<String> = [
        "2026-11-27T00:59:59Z",
        "2026-11-27T01:00:00Z",
        "2026-11-27T23:59:00Z",
        "2026-11-30T00:00:00Z",
    ]
    .iter()
    .map(|now| reading_text(&readings, now).unwrap())
    .map(|line| line.rsplit_once("read ").unwrap().1.trim_end_matches(")\n").to_owned())
    .collect();

    assert_eq!(ages, ["59 min ago", "1 h ago", "23 h ago", "3 d ago"]);
}

#[test]
fn no_reading_line_without_a_reading_or_after_its_reset() {
    let readings = kept(
        &ReadingOwner::Account(account("comp-claude")),
        &[five_hour(0.3, "2026-11-27T15:30:00Z")],
        "2026-11-27T14:00:00Z",
    );

    assert_eq!(reading_text(&LimitReadings::default(), "2026-11-27T14:00:00Z"), None);
    assert_eq!(reading_text(&readings, "2026-11-27T15:30:00Z"), None);
}

#[test]
fn each_owner_has_a_line_accounts_first() {
    let mut readings = kept(
        &ReadingOwner::Agent("claude-1".parse().unwrap()),
        &[five_hour(0.1, "2026-11-27T15:30:00Z")],
        "2026-11-27T14:00:00Z",
    );
    readings.keep(
        &ReadingOwner::Account(account("comp-claude")),
        &[five_hour(0.9, "2026-11-27T16:00:00Z")],
        at("2026-11-27T14:00:00Z"),
        at("2026-11-27T14:00:00Z"),
    );

    let text = reading_text(&readings, "2026-11-27T14:00:00Z").unwrap();

    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(
        lines,
        [
            "comp-claude: 5h 90% (resets 16:00, read just now)",
            "claude-1: 5h 10% (resets 15:30, read just now)"
        ]
    );
}
