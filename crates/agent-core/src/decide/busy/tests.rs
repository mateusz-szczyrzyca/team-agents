//! The decision for a queued task whose agent is busy: the wait, the hand-over and to whom, and the waits that are kept.

use time::Duration;

use super::{Availability, Decision, Offer, Waiting, decide};
use crate::ids::AgentName;
use crate::kinds::{Role, Tier};

/// The agent named `name`.
fn agent(name: &str) -> AgentName {
    name.parse().expect("a valid agent name")
}

/// A free agent of `tier` on a CLI with `cli_runs` runs, outside the team.
fn free(name: &AgentName, tier: Tier, cli_runs: u32) -> Offer<'_> {
    Offer {
        agent: name,
        tier,
        in_team: false,
        availability: Availability::Free,
        cli_runs,
    }
}

/// The waiting task of a solo agent held by `held`, which waited `minutes` minutes with `busy_after` seconds.
fn waiting<'a>(held: Tier, minutes: i64, busy_after: u32, offers: &'a [Offer<'a>]) -> Waiting<'a> {
    Waiting {
        role: Role::Solo,
        member_runs: false,
        held,
        waited: Duration::minutes(minutes),
        busy_after,
        offers,
    }
}

#[test]
fn a_task_keeps_waiting_until_it_has_waited_busy_after_seconds() {
    let claude_2 = agent("claude-2");
    let offers = [free(&claude_2, Tier::Normal, 0)];
    // Four minutes against 300 seconds: still waiting.
    assert_eq!(decide(&waiting(Tier::Normal, 4, 300, &offers)), Decision::Keep);
    // Five minutes: handed over.
    assert_eq!(
        decide(&waiting(Tier::Normal, 5, 300, &offers)),
        Decision::HandOver(&claude_2)
    );
}

#[test]
fn zero_busy_after_never_hands_a_task_over() {
    let claude_2 = agent("claude-2");
    let offers = [free(&claude_2, Tier::Normal, 0)];
    assert_eq!(decide(&waiting(Tier::Normal, 600, 0, &offers)), Decision::Keep);
}

#[test]
fn a_candidate_of_the_same_or_a_higher_tier_takes_over_and_a_lower_one_does_not() {
    let same = agent("claude-2");
    let higher = agent("inv-opus");
    let lower = agent("glm-1");
    let only_lower = [Offer {
        agent: &lower,
        ..free(&lower, Tier::Worker, 0)
    }];
    assert_eq!(decide(&waiting(Tier::Normal, 10, 300, &only_lower)), Decision::Keep);

    let only_same = [free(&same, Tier::Normal, 0)];
    assert_eq!(
        decide(&waiting(Tier::Normal, 10, 300, &only_same)),
        Decision::HandOver(&same)
    );

    let only_higher = [free(&higher, Tier::Investigator, 0)];
    assert_eq!(
        decide(&waiting(Tier::Normal, 10, 300, &only_higher)),
        Decision::HandOver(&higher)
    );
}

#[test]
fn an_agent_of_the_team_running_or_paused_is_never_given_the_task() {
    let in_team = agent("claude-2");
    let running = agent("claude-3");
    let paused = agent("claude-4");
    let offers = [
        Offer {
            in_team: true,
            ..free(&in_team, Tier::Normal, 0)
        },
        Offer {
            availability: Availability::Busy,
            ..free(&running, Tier::Normal, 0)
        },
        Offer {
            availability: Availability::Paused,
            ..free(&paused, Tier::Normal, 0)
        },
    ];
    assert_eq!(decide(&waiting(Tier::Normal, 10, 300, &offers)), Decision::Keep);
}

#[test]
fn with_no_candidate_the_task_waits_on() {
    assert_eq!(decide(&waiting(Tier::Normal, 10, 300, &[])), Decision::Keep);
}

#[test]
fn the_candidate_whose_cli_runs_least_is_taken_and_among_equals_the_first_of_the_list() {
    let first = agent("claude-2");
    let second = agent("codex-1");
    let busy_cli = [free(&first, Tier::Normal, 2), free(&second, Tier::Normal, 1)];
    assert_eq!(
        decide(&waiting(Tier::Normal, 10, 300, &busy_cli)),
        Decision::HandOver(&second)
    );

    let equal = [free(&first, Tier::Normal, 1), free(&second, Tier::Normal, 1)];
    assert_eq!(
        decide(&waiting(Tier::Normal, 10, 300, &equal)),
        Decision::HandOver(&first)
    );
}

#[test]
fn the_tasks_that_are_never_handed_over_keep_waiting() {
    let claude_2 = agent("claude-2");
    let offers = [free(&claude_2, Tier::Normal, 0)];
    for role in [Role::Worker, Role::Reviewer] {
        let task = Waiting {
            role,
            ..waiting(Tier::Normal, 10, 300, &offers)
        };
        assert_eq!(decide(&task), Decision::Keep, "{role:?}");
    }
    let coordinator_with_a_member = Waiting {
        role: Role::Coordinator,
        member_runs: true,
        ..waiting(Tier::Normal, 10, 300, &offers)
    };
    assert_eq!(decide(&coordinator_with_a_member), Decision::Keep);
    let idle_coordinator = Waiting {
        role: Role::Coordinator,
        member_runs: false,
        ..waiting(Tier::Normal, 10, 300, &offers)
    };
    assert_eq!(decide(&idle_coordinator), Decision::HandOver(&claude_2));
}
