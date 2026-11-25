use crate::decide::replacement::refused::{Candidate, Pick, pick};
use crate::ids::AgentName;
use crate::kinds::{Role, Tier};
use crate::state::Timestamp;

/// An agent name for tests.
fn name(text: &str) -> AgentName {
    text.parse().unwrap()
}

/// A point in time for tests: 2026-11-25T12:00:00Z.
fn now() -> Timestamp {
    "2026-11-25T12:00:00Z".parse().unwrap()
}

/// A point in time one hour past `now`.
fn later() -> Timestamp {
    "2026-11-25T13:00:00Z".parse().unwrap()
}

/// A candidate that is free (not busy, not refused).
fn free<'a>(name: &'a AgentName, provider: &'a str, tier: Tier) -> Candidate<'a> {
    Candidate {
        name,
        provider,
        tier,
        busy: false,
        refused_until: None,
    }
}

/// A candidate that is busy (at its run limit).
fn busy<'a>(name: &'a AgentName, provider: &'a str, tier: Tier) -> Candidate<'a> {
    Candidate {
        name,
        provider,
        tier,
        busy: true,
        refused_until: None,
    }
}

/// A candidate that is refused until a time.
fn refused<'a>(name: &'a AgentName, provider: &'a str, tier: Tier, until: Timestamp) -> Candidate<'a> {
    Candidate {
        name,
        provider,
        tier,
        busy: false,
        refused_until: Some(until),
    }
}

/// The first replacement agent.
fn first_replacement() -> AgentName {
    name("claude-2")
}

/// The second replacement agent.
fn second_replacement() -> AgentName {
    name("gemini-2")
}

// --- RED: the first test expects a pick from the replacement list, but the stub returns NoneFree ---

#[test]
fn picks_the_first_free_agent_of_the_replacement_list() {
    let first = first_replacement();
    let second = second_replacement();
    let replacement = [free(&first, "claude", Tier::Normal), free(&second, "gemini", Tier::Normal)];
    let normal = [];
    let investigators = [];

    let result = pick(
        Role::Member,
        &[],
        "openrouter",
        &replacement,
        &normal,
        &investigators,
        now(),
        false,
    );

    assert_eq!(result, Pick::Agent(&first));
}
