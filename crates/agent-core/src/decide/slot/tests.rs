//! The decision whether an agent may start a run: the limits by tier and the run that holds an agent at its limit.

use std::num::NonZeroU32;

use super::{Going, RunLimit, busy_run, limit_for};
use crate::config::Limits;
use crate::ids::{AgentName, TaskId, TeamId};
use crate::kinds::Tier;

/// The agent named `name`.
fn agent(name: &str) -> AgentName {
    name.parse().expect("a valid agent name")
}

/// A run of `name` in the team `team`, for the task `task`.
fn run(name: &str, team: &str, task: &str) -> Going {
    Going {
        agent: agent(name),
        team: team.parse().expect("a valid team ID"),
        task: task.parse().expect("a valid task ID"),
    }
}

/// `limit` as a number of runs.
fn at_most(runs: u32) -> RunLimit {
    RunLimit::AtMost(NonZeroU32::new(runs).expect("a positive limit"))
}

#[test]
fn a_zero_configured_limit_is_no_limit_and_any_other_is_the_most_runs() {
    assert_eq!(RunLimit::of(0), RunLimit::Unlimited);
    assert_eq!(RunLimit::of(1), at_most(1));
    assert_eq!(RunLimit::of(3), at_most(3));
}

#[test]
fn the_limit_of_a_tier_is_the_one_its_key_names() {
    let limits = Limits {
        runs_per_agent: 2,
        runs_per_worker: 5,
        ..Limits::default()
    };
    assert_eq!(limit_for(&limits, Tier::Normal), at_most(2));
    assert_eq!(limit_for(&limits, Tier::Investigator), at_most(2));
    assert_eq!(limit_for(&limits, Tier::Worker), at_most(5));
    let unlimited = Limits {
        runs_per_agent: 0,
        runs_per_worker: 0,
        ..Limits::default()
    };
    assert_eq!(limit_for(&unlimited, Tier::Normal), RunLimit::Unlimited);
    assert_eq!(limit_for(&unlimited, Tier::Worker), RunLimit::Unlimited);
}

#[test]
fn an_agent_with_no_run_going_is_free_at_every_limit() {
    for limit in [RunLimit::Unlimited, at_most(1), at_most(2)] {
        assert_eq!(busy_run(&agent("codex-1"), limit, &[]), None, "{limit:?}");
    }
}

#[test]
fn a_run_of_another_agent_never_holds_an_agent() {
    let going = [run("gemini-1", "team-20261122-140915-aaaa", "task-20261122-140915-a1a1")];
    assert_eq!(busy_run(&agent("codex-1"), at_most(1), &going), None);
}

#[test]
fn an_agent_at_its_limit_is_held_by_its_first_run_whatever_the_team() {
    let going = [
        run("codex-1", "team-20261122-140915-aaaa", "task-20261122-140915-a1a1"),
        run("gemini-1", "team-20261122-140915-bbbb", "task-20261122-140915-b1b1"),
        run("codex-1", "team-20261122-140915-cccc", "task-20261122-140915-c1c1"),
    ];
    assert_eq!(busy_run(&agent("codex-1"), at_most(1), &going), Some(&going[0]));
    assert_eq!(busy_run(&agent("codex-1"), at_most(2), &going), None);
}

#[test]
fn an_agent_with_two_runs_going_is_held_at_a_limit_of_two() {
    let going = [run("codex-1", "team-20261122-140915-aaaa", "task-20261122-140915-a1a1"), run("codex-1", "team-20261122-140915-bbbb", "task-20261122-140915-b1b1")];
    assert_eq!(busy_run(&agent("codex-1"), at_most(2), &going), Some(&going[0]));
}

#[test]
fn an_agent_with_no_limit_is_never_held() {
    let going = [run("codex-1", "team-20261122-140915-aaaa", "task-20261122-140915-a1a1"), run("codex-1", "team-20261122-140915-bbbb", "task-20261122-140915-b1b1")];
    assert_eq!(busy_run(&agent("codex-1"), RunLimit::Unlimited, &going), None);
}
