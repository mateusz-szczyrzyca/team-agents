//! The decision whether an agent may start a run: the limits by tier and the run that holds an agent at its limit.

use std::num::NonZeroU32;

use super::{Going, RunLimit, busy_run, limit_for};
use crate::config::Limits;
use crate::ids::AgentName;
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
    let going = [run(
        "gemini-1",
        "team-20261122-140915-aaaa",
        "task-20261122-140915-a1a1",
    )];
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
    assert_eq!(busy_run(&agent("codex-1"), at_most(2), &going), Some(&going[0]));
    assert_eq!(busy_run(&agent("codex-1"), at_most(3), &going), None);
}

#[test]
fn an_agent_with_two_runs_going_is_held_at_a_limit_of_two() {
    let going = [
        run("codex-1", "team-20261122-140915-aaaa", "task-20261122-140915-a1a1"),
        run("codex-1", "team-20261122-140915-bbbb", "task-20261122-140915-b1b1"),
    ];
    assert_eq!(busy_run(&agent("codex-1"), at_most(2), &going), Some(&going[0]));
}

#[test]
fn an_agent_with_no_limit_is_never_held() {
    let going = [
        run("codex-1", "team-20261122-140915-aaaa", "task-20261122-140915-a1a1"),
        run("codex-1", "team-20261122-140915-bbbb", "task-20261122-140915-b1b1"),
    ];
    assert_eq!(busy_run(&agent("codex-1"), RunLimit::Unlimited, &going), None);
}

use super::{Queued, Slot, plan_starts, runs_going};
use crate::ids::TaskId;
use crate::state::State;
use crate::state::fixtures::{inv, member, team, team_id};

/// A queued task of `name` (`tier`) in `team_name`, with ID `task_name`.
fn queued(name: &str, tier: Tier, team_name: &str, task_name: &str) -> Queued {
    Queued {
        task: task_name.parse().expect("a valid task ID"),
        team: team_name.parse().expect("a valid team ID"),
        agent: agent(name),
        tier,
    }
}

/// The task ID `name`.
fn task_id(name: &str) -> TaskId {
    name.parse().expect("a valid task ID")
}

#[test]
#[expect(clippy::type_complexity, reason = "table test tuple of input and expected slots")]
#[expect(clippy::too_many_lines, reason = "table test of every tier and limit case")]
fn plan_starts_table_covers_every_limit_and_queue_outcome() {
    let mut state = State::new();
    state.insert_team(team(1)).expect("sample team inserts");
    let mut reviewer = member(9, inv("inv-codex"));
    reviewer
        .transition(crate::status::TaskStatus::Running)
        .expect("queued reviewer transitions to running");
    let reviewer_team = team_id(1).to_string();
    let reviewer_task = reviewer.task().to_string();
    state
        .add_reviewer(&team_id(1), reviewer)
        .expect("outside reviewer is added");
    let reviewer_going = runs_going(state.teams());

    let t_a = "team-20261122-140915-aaaa";
    let t_b = "team-20261122-140915-bbbb";
    let k_1 = "task-20261122-140915-a1a1";
    let k_2 = "task-20261122-140915-b1b1";
    let k_3 = "task-20261122-140915-c1c1";

    let limit_0 = Limits {
        runs_per_agent: 0,
        ..Limits::default()
    };
    let limit_1 = Limits::default();
    let limit_2 = Limits {
        runs_per_agent: 2,
        ..Limits::default()
    };

    let cases: Vec<(&str, Vec<Queued>, Vec<Going>, Limits, Vec<(TaskId, Slot)>)> = vec![
        (
            "the agent is free",
            vec![queued("codex-1", Tier::Normal, t_a, k_1)],
            vec![],
            limit_1.clone(),
            vec![(task_id(k_1), Slot::Start)],
        ),
        (
            "running in the same team",
            vec![queued("codex-1", Tier::Normal, t_a, k_2)],
            vec![run("codex-1", t_a, k_1)],
            limit_1.clone(),
            vec![(task_id(k_2), Slot::Wait(run("codex-1", t_a, k_1)))],
        ),
        (
            "running in another team",
            vec![queued("codex-1", Tier::Normal, t_b, k_2)],
            vec![run("codex-1", t_a, k_1)],
            limit_1.clone(),
            vec![(task_id(k_2), Slot::Wait(run("codex-1", t_a, k_1)))],
        ),
        (
            "running as a reviewer",
            vec![queued("inv-codex", Tier::Investigator, t_b, k_2)],
            reviewer_going,
            limit_1.clone(),
            vec![(
                task_id(k_2),
                Slot::Wait(run("inv-codex", &reviewer_team, &reviewer_task)),
            )],
        ),
        (
            "the limit 0",
            vec![queued("codex-1", Tier::Normal, t_b, k_2)],
            vec![run("codex-1", t_a, k_1)],
            limit_0,
            vec![(task_id(k_2), Slot::Start)],
        ),
        (
            "the limit 2 with one run",
            vec![queued("codex-1", Tier::Normal, t_b, k_2)],
            vec![run("codex-1", t_a, k_1)],
            limit_2.clone(),
            vec![(task_id(k_2), Slot::Start)],
        ),
        (
            "the limit 2 with two runs",
            vec![queued("codex-1", Tier::Normal, t_b, k_3)],
            vec![run("codex-1", t_a, k_1), run("codex-1", t_b, k_2)],
            limit_2,
            vec![(task_id(k_3), Slot::Wait(run("codex-1", t_a, k_1)))],
        ),
        (
            "two tasks of one agent queued at once (one starts)",
            vec![
                queued("codex-1", Tier::Normal, t_a, k_1),
                queued("codex-1", Tier::Normal, t_b, k_2),
            ],
            vec![],
            limit_1,
            vec![
                (task_id(k_1), Slot::Start),
                (task_id(k_2), Slot::Wait(run("codex-1", t_a, k_1))),
            ],
        ),
        (
            "a worker waits at runs_per_worker = 1",
            vec![queued("worker-1", Tier::Worker, t_b, k_2)],
            vec![run("worker-1", t_a, k_1)],
            Limits::default(),
            vec![(task_id(k_2), Slot::Wait(run("worker-1", t_a, k_1)))],
        ),
        (
            "a worker starts at runs_per_worker = 0",
            vec![queued("worker-1", Tier::Worker, t_b, k_2)],
            vec![run("worker-1", t_a, k_1)],
            Limits {
                runs_per_worker: 0,
                ..Limits::default()
            },
            vec![(task_id(k_2), Slot::Start)],
        ),
    ];

    for (name, queue, going, limits, expected) in cases {
        assert_eq!(plan_starts(&queue, &going, &limits), expected, "{name}");
    }
}
