//! The words `agentbus serve` is given for a run: the plan of a run that keeps one, and the tier and role on the bus.
use std::path::Path;

use agent_core::kinds::{Role, Tier};

use super::bus_words;

/// The plan argument of `words`, if the bus was given one.
fn plan_argument(words: &[String]) -> Option<String> {
    let at = words.iter().position(|word| word == "--plan")?;
    Some(words.get(at + 1).cloned().unwrap_or_default())
}

/// The value of `word` in `words`.
fn value_of<'a>(words: &'a [String], word: &str) -> Option<&'a str> {
    let at = words.iter().position(|given| given == word)?;
    words.get(at + 1).map(String::as_str)
}

const ROLES: [Role; 3] = [Role::Solo, Role::Member, Role::Coordinator];

#[test]
fn a_run_of_a_plan_role_is_given_its_plan_file() {
    for role in [Role::Solo, Role::Member, Role::Coordinator] {
        let words = bus_words(role, &ROLES, Tier::Normal, Path::new("/home/worktrees/repo/task-1"));
        assert_eq!(
            plan_argument(&words).as_deref(),
            Some("/home/worktrees/repo/task-1/.agent/plan.md"),
            "{role:?}"
        );
    }
}

#[test]
fn a_run_of_another_role_is_given_no_plan() {
    for (role, tier) in [
        (Role::Worker, Tier::Worker),
        (Role::Worker, Tier::Normal),
        (Role::Reviewer, Tier::Normal),
    ] {
        let words = bus_words(role, &ROLES, tier, Path::new("/w"));
        assert_eq!(plan_argument(&words), None, "{role:?} {tier:?}");
    }
    assert_eq!(
        plan_argument(&bus_words(Role::Member, &[], Tier::Normal, Path::new("/w"))),
        None,
        "a configuration that lists no plan role holds no run to a plan"
    );
}

#[test]
fn an_investigator_on_the_bus_is_a_normal_tier_with_the_investigator_role() {
    // An investigator in a team works as a coordinator or a member and keeps that role's plan; an investigator that
    // answers an investigation request is on the bus with the investigator role and no plan of its own.
    let words = bus_words(Role::Coordinator, &ROLES, Tier::Investigator, Path::new("/w"));

    assert_eq!(value_of(&words, "--role"), Some("investigator"));
    assert_eq!(value_of(&words, "--tier"), Some("normal"));
    assert_eq!(
        plan_argument(&words).as_deref(),
        Some("/w/.agent/plan.md"),
        "an investigator that coordinates keeps its plan"
    );
}

#[test]
fn the_bus_is_told_the_tier_and_role_of_the_run() {
    let of = |role: Role, tier: Tier| bus_words(role, &ROLES, tier, Path::new("/w"));

    assert_eq!(value_of(&of(Role::Member, Tier::Normal), "--tier"), Some("normal"));
    assert_eq!(value_of(&of(Role::Member, Tier::Normal), "--role"), Some("worker"));
    assert_eq!(value_of(&of(Role::Worker, Tier::Worker), "--tier"), Some("worker"));
    assert_eq!(value_of(&of(Role::Reviewer, Tier::Normal), "--role"), Some("worker"));
    assert_eq!(
        value_of(&of(Role::Coordinator, Tier::Investigator), "--role"),
        Some("investigator")
    );
}
