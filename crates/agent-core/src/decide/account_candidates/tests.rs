//! Table tests of the choice among candidates when accounts are paused.

use std::collections::BTreeSet;

use super::{
    Candidate, agents_of_paused_accounts, candidates_with_accounts, investigators_of_paused_accounts, paused_accounts,
};
use crate::config::{AccountName, Config};
use crate::decide::limit::Limit;
use crate::ids::{AgentName, InvestigatorName};
use crate::state::{AccountPause, State, Timestamp};

/// A moment of these tests.
fn at(text: &str) -> Timestamp {
    text.parse().unwrap()
}

/// The moment these tests ask at.
fn now() -> Timestamp {
    at("2026-11-27T10:00:00Z")
}

fn account(text: &str) -> AccountName {
    text.parse().unwrap()
}

/// A pause of an account that ends at `until`.
fn pause(until: &str) -> AccountPause {
    AccountPause {
        until: at(until),
        reason: Limit::Quota { reset_in: None },
    }
}

/// The names that `candidates_with_accounts` leaves of `rows` (a name and its account) when `paused` are paused.
fn picked(rows: &[(&str, Option<&str>)], paused: &[&str]) -> Vec<String> {
    let names: Vec<String> = rows.iter().map(|(name, _)| (*name).to_owned()).collect();
    let accounts: Vec<Option<AccountName>> = rows.iter().map(|(_, account)| account.map(self::account)).collect();
    let candidates: Vec<Candidate<'_, String>> = names
        .iter()
        .zip(&accounts)
        .map(|(name, account)| Candidate {
            name,
            account: account.as_ref(),
        })
        .collect();
    let paused: BTreeSet<AccountName> = paused.iter().map(|name| account(name)).collect();
    candidates_with_accounts(&candidates, &paused)
        .into_iter()
        .cloned()
        .collect()
}

#[test]
fn every_candidate_is_free_when_no_account_is_paused() {
    let rows = [("a", Some("one")), ("b", Some("two")), ("c", Some("one"))];

    assert_eq!(picked(&rows, &[]), ["a", "b", "c"]);
}

#[test]
fn the_agents_of_a_paused_account_are_not_picked() {
    let rows = [("a", Some("one")), ("b", Some("two")), ("c", Some("one"))];

    assert_eq!(picked(&rows, &["one"]), ["b"]);
}

#[test]
fn no_candidate_is_left_when_every_account_is_paused() {
    let rows = [("a", Some("one")), ("b", Some("two"))];

    assert_eq!(picked(&rows, &["one", "two"]), Vec::<String>::new());
}

#[test]
fn an_agent_without_an_account_is_picked_whatever_is_paused() {
    let rows = [("a", None), ("b", Some("one")), ("c", None)];

    assert_eq!(picked(&rows, &["one"]), ["a", "c"]);
}

#[test]
fn another_account_of_the_same_owner_is_not_paused_with_it() {
    let rows = [("a", Some("comp-claude")), ("b", Some("comp-second"))];

    assert_eq!(picked(&rows, &["comp-claude"]), ["b"]);
}

#[test]
fn an_account_is_paused_until_its_reset_and_not_after() {
    let mut state = State::default();
    state.record_account_pause(account("one"), Some(pause("2026-11-27T10:30:00Z")));
    state.record_account_pause(account("two"), Some(pause("2026-11-27T09:00:00Z")));

    assert_eq!(paused_accounts(&state, now()), BTreeSet::from([account("one")]));
    assert_eq!(paused_accounts(&state, at("2026-11-27T10:30:00Z")), BTreeSet::new());
}

/// A configuration with two agents and two investigators on the accounts `one` and `two`, and one agent and one
/// investigator without an account.
fn config() -> Config {
    let toml = r#"
[accounts.one]
owner = "comp"

[accounts.two]
owner = "comp"

[agents.comp-a1]
cli = "claude"
account = "one"

[agents.comp-a2]
cli = "claude"
account = "two"

[agents.a-3]
cli = "claude"

[investigators.comp-inv1]
cli = "claude"
account = "one"

[investigators.comp-inv2]
cli = "claude"
account = "two"

[investigators.inv-3]
cli = "claude"

[tiers]
normal = ["comp-a1", "comp-a2", "a-3"]
"#;
    Config::from_toml_str(toml).unwrap()
}

fn paused_one() -> State {
    let mut state = State::default();
    state.record_account_pause(account("one"), Some(pause("2026-11-27T10:30:00Z")));
    state
}

#[test]
fn the_agents_of_a_paused_account_are_those_that_name_it() {
    let agents = agents_of_paused_accounts(&paused_one(), &config(), now());

    assert_eq!(agents, ["comp-a1".parse::<AgentName>().unwrap()]);
}

#[test]
fn the_investigators_of_a_paused_account_are_those_that_name_it() {
    let investigators = investigators_of_paused_accounts(&paused_one(), &config(), now());

    assert_eq!(
        investigators,
        BTreeSet::from(["comp-inv1".parse::<InvestigatorName>().unwrap()])
    );
}

#[test]
fn no_agent_is_of_a_paused_account_when_none_is_paused() {
    assert_eq!(agents_of_paused_accounts(&State::default(), &config(), now()), []);
    assert_eq!(
        investigators_of_paused_accounts(&State::default(), &config(), now()),
        BTreeSet::new()
    );
}

#[test]
fn the_pause_of_an_account_ends_for_its_agents_at_the_reset() {
    let state = paused_one();

    assert_eq!(
        agents_of_paused_accounts(&state, &config(), at("2026-11-27T10:30:00Z")),
        []
    );
}

#[test]
fn a_pause_of_another_account_holds_for_its_own_agents_only() {
    let mut state = State::default();
    state.record_account_pause(account("two"), Some(pause("2026-11-27T15:00:00Z")));

    assert_eq!(
        agents_of_paused_accounts(&state, &config(), now()),
        ["comp-a2".parse::<AgentName>().unwrap()]
    );
}
