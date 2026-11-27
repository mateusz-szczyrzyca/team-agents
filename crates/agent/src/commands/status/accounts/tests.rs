use std::collections::BTreeMap;

use agent_core::decide::limit::Limit;

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
