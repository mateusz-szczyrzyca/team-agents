//! Table test of the rule: a session goes on only under the account it began under.

use crate::config::AccountName;

use super::{AccountSession, session_start};

/// The account named `name`.
fn account(name: &str) -> AccountName {
    name.parse().unwrap()
}

#[test]
fn session_start_follows_the_account_of_the_session() {
    let (a, b) = (account("comp-claude"), account("mateusz-claude"));
    let table = [
        (None, None, AccountSession::Resume),
        (Some(&a), Some(&a), AccountSession::Resume),
        (None, Some(&a), AccountSession::New),
        (Some(&a), Some(&b), AccountSession::New),
        (Some(&a), None, AccountSession::New),
    ];
    for (recorded, now, expected) in table {
        assert_eq!(session_start(recorded, now), expected, "{recorded:?} -> {now:?}");
    }
}
