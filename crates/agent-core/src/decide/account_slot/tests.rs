//! The slot of an account: below its limit, at it, without a limit, and for an agent without an account.

use std::num::NonZeroU32;

use super::{AccountFull, AccountSlot, account_slot, slot_of};
use crate::config::{AccountName, Config};
use crate::decide::slot::Going;
use crate::ids::AgentName;

fn account(name: &str) -> AccountName {
    name.parse().unwrap()
}

fn limit(n: u32) -> Option<NonZeroU32> {
    NonZeroU32::new(n)
}

fn full(running: u32, limit: u32) -> AccountSlot {
    AccountSlot::Full(AccountFull {
        account: account("comp-claude"),
        running,
        limit: NonZeroU32::new(limit).unwrap(),
    })
}

#[test]
fn account_slot_is_open_below_the_limit() {
    let comp = account("comp-claude");

    assert_eq!(account_slot(Some(&comp), limit(2), 1), AccountSlot::Open);
    assert_eq!(account_slot(Some(&comp), limit(1), 0), AccountSlot::Open);
}

#[test]
fn account_slot_is_full_at_the_limit_and_above_it() {
    let comp = account("comp-claude");

    assert_eq!(account_slot(Some(&comp), limit(1), 1), full(1, 1));
    assert_eq!(account_slot(Some(&comp), limit(2), 2), full(2, 2));
    assert_eq!(account_slot(Some(&comp), limit(2), 3), full(3, 2));
}

#[test]
fn account_slot_is_open_without_an_account_and_without_a_limit() {
    let comp = account("comp-claude");

    assert_eq!(account_slot(None, limit(1), 5), AccountSlot::Open);
    assert_eq!(account_slot(Some(&comp), None, 5), AccountSlot::Open);
}

#[test]
fn a_full_account_says_what_the_task_waits_for() {
    let AccountSlot::Full(waiting) = full(1, 1) else {
        panic!("expected a full account");
    };

    assert_eq!(waiting.to_string(), "waiting for account comp-claude (1/1 runs)");
}

/// A run of `agent` in task `n` of the team 0.
fn going(agent: &str, n: u32) -> Going {
    Going {
        agent: agent.parse().unwrap(),
        team: "team-20261111-153000-t5m8".parse().unwrap(),
        task: format!("task-20261111-1530{n:02}-k3x9").parse().unwrap(),
    }
}

/// A configuration where two agents and an investigator share the account `comp-claude` with `max_runs = 1`, and
/// `claude-1` has no account.
fn config(max_runs: &str) -> Config {
    Config::from_toml_str(&format!(
        r#"
[accounts.comp-claude]
owner = "comp"
{max_runs}

[agents.comp-claude-1]
cli = "claude"
account = "comp-claude"

[agents.comp-claude-2]
cli = "claude"
account = "comp-claude"

[agents.claude-1]
cli = "claude"

[investigators.comp-inv-opus]
cli = "claude"
model = "opus"
account = "comp-claude"

[tiers]
normal = ["comp-claude-1", "comp-claude-2", "claude-1"]
"#
    ))
    .unwrap()
}

fn agent(name: &str) -> AgentName {
    name.parse().unwrap()
}

#[test]
fn slot_of_counts_the_runs_of_every_agent_and_investigator_of_the_account() {
    let config = config("max_runs = 2");
    let one = [going("comp-claude-1", 1)];
    let two = [going("comp-claude-1", 1), going("comp-inv-opus", 2)];

    assert_eq!(slot_of(&config, &agent("comp-claude-2"), &one), AccountSlot::Open);
    assert_eq!(slot_of(&config, &agent("comp-claude-2"), &two), full(2, 2));
}

#[test]
fn slot_of_does_not_count_the_runs_of_agents_of_no_account() {
    let config = config("max_runs = 1");
    let others = [going("claude-1", 1)];

    assert_eq!(slot_of(&config, &agent("comp-claude-1"), &others), AccountSlot::Open);
    assert_eq!(
        slot_of(&config, &agent("claude-1"), &[going("comp-claude-1", 2)]),
        AccountSlot::Open
    );
}

#[test]
fn slot_of_is_open_for_an_account_without_max_runs() {
    let config = config("");

    assert_eq!(
        slot_of(
            &config,
            &agent("comp-claude-2"),
            &[going("comp-claude-1", 1), going("comp-inv-opus", 2)]
        ),
        AccountSlot::Open
    );
}
