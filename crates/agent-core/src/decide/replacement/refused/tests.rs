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

/// A point in time one hour before `now`.
fn earlier() -> Timestamp {
    "2026-11-25T11:00:00Z".parse().unwrap()
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

// --- Empty lists ---

#[test]
fn returns_none_free_when_no_candidates_are_given() {
    let result = pick(Role::Member, &[], "openrouter", &[], &[], &[], now(), false);

    assert_eq!(
        result,
        Pick::NoneFree {
            earliest_return: None
        }
    );
}

#[test]
fn returns_none_free_when_replacements_are_exhausted() {
    let agent = name("claude-2");
    let replacement = [free(&agent, "claude", Tier::Normal)];

    let result = pick(Role::Member, &[], "openrouter", &replacement, &[], &[], now(), true);

    assert_eq!(
        result,
        Pick::NoneFree {
            earliest_return: None
        }
    );
}

// --- [replacement] agents ---

#[test]
fn picks_the_first_free_agent_of_the_replacement_list() {
    let first = name("claude-2");
    let second = name("gemini-2");
    let replacement = [
        free(&first, "claude", Tier::Normal),
        free(&second, "gemini", Tier::Normal),
    ];

    let result = pick(Role::Member, &[], "openrouter", &replacement, &[], &[], now(), false);

    assert_eq!(result, Pick::Agent(&first));
}

#[test]
fn skips_a_replacement_agent_of_the_same_provider() {
    let same = name("claude-2");
    let other = name("gemini-2");
    let replacement = [
        free(&same, "openrouter", Tier::Normal),
        free(&other, "gemini", Tier::Normal),
    ];

    let result = pick(
        Role::Member,
        &[],
        "openrouter",
        &replacement,
        &[],
        &[],
        now(),
        false,
    );

    // Skips claude-2 (same provider openrouter) and picks gemini-2.
    assert_eq!(result, Pick::Agent(&other));
}

#[test]
fn skips_a_busy_replacement_agent() {
    let busy_agent = name("claude-2");
    let free_agent = name("gemini-2");
    let replacement = [
        busy(&busy_agent, "claude", Tier::Normal),
        free(&free_agent, "gemini", Tier::Normal),
    ];

    let result = pick(
        Role::Member,
        &[],
        "openrouter",
        &replacement,
        &[],
        &[],
        now(),
        false,
    );

    assert_eq!(result, Pick::Agent(&free_agent));
}

#[test]
fn skips_a_refused_replacement_agent() {
    let refused_agent = name("claude-2");
    let free_agent = name("gemini-2");
    let replacement = [
        refused(&refused_agent, "claude", Tier::Normal, later()),
        free(&free_agent, "gemini", Tier::Normal),
    ];

    let result = pick(
        Role::Member,
        &[],
        "openrouter",
        &replacement,
        &[],
        &[],
        now(),
        false,
    );

    assert_eq!(result, Pick::Agent(&free_agent));
}

#[test]
fn an_agent_refused_until_a_past_time_is_not_refused_now() {
    let past = name("claude-2");
    let replacement = [refused(&past, "claude", Tier::Normal, earlier())];

    let result = pick(
        Role::Member,
        &[],
        "openrouter",
        &replacement,
        &[],
        &[],
        now(),
        false,
    );

    assert_eq!(result, Pick::Agent(&past));
}

#[test]
fn skips_a_replacement_agent_in_the_team() {
    let teammate = name("claude-2");
    let outsider = name("gemini-2");
    let team = [teammate.clone()];
    let replacement = [
        free(&teammate, "claude", Tier::Normal),
        free(&outsider, "gemini", Tier::Normal),
    ];

    let result = pick(
        Role::Member,
        &team,
        "openrouter",
        &replacement,
        &[],
        &[],
        now(),
        false,
    );

    assert_eq!(result, Pick::Agent(&outsider));
}

#[test]
fn deepinfra_and_openrouter_count_as_two_providers() {
    let deepinfra_agent = name("deepinfra-gemini");
    let openrouter_agent = name("openrouter-gemini");
    let replacement = [
        free(&deepinfra_agent, "deepinfra", Tier::Normal),
        free(&openrouter_agent, "openrouter", Tier::Normal),
    ];

    // The refused provider is deepinfra; openrouter is a different provider.
    let result = pick(
        Role::Member,
        &[],
        "deepinfra",
        &replacement,
        &[],
        &[],
        now(),
        false,
    );

    assert_eq!(result, Pick::Agent(&openrouter_agent));
}

// --- Fallback to tier normal ---

#[test]
fn falls_back_to_tier_normal_when_no_replacement_agent_fits() {
    let replacement_agent = name("claude-2");
    let normal_agent = name("codex-1");
    // All replacement agents are of the same provider → skipped.
    let replacement = [free(&replacement_agent, "openrouter", Tier::Normal)];
    let normal = [free(&normal_agent, "codex", Tier::Normal)];

    let result = pick(
        Role::Member,
        &[],
        "openrouter",
        &replacement,
        &normal,
        &[],
        now(),
        false,
    );

    assert_eq!(result, Pick::Agent(&normal_agent));
}

#[test]
fn falls_back_to_tier_normal_when_replacement_list_is_empty() {
    let normal_agent = name("codex-1");
    let normal = [free(&normal_agent, "codex", Tier::Normal)];

    let result = pick(Role::Member, &[], "openrouter", &[], &normal, &[], now(), false);

    assert_eq!(result, Pick::Agent(&normal_agent));
}

#[test]
fn tier_normal_respects_the_same_skip_rules() {
    let same_provider = name("codex-1");
    let good = name("gemini-2");
    let normal = [
        free(&same_provider, "openrouter", Tier::Normal),
        free(&good, "gemini", Tier::Normal),
    ];

    let result = pick(Role::Member, &[], "openrouter", &[], &normal, &[], now(), false);

    assert_eq!(result, Pick::Agent(&good));
}

// --- Tier rules ---

#[test]
fn a_member_never_goes_to_a_worker() {
    let worker_agent = name("glm");
    let normal_agent = name("codex-1");
    let replacement = [
        free(&worker_agent, "opencode", Tier::Worker),
        free(&normal_agent, "codex", Tier::Normal),
    ];

    let result = pick(
        Role::Member,
        &[],
        "claude",
        &replacement,
        &[],
        &[],
        now(),
        false,
    );

    // Skips the worker, picks the normal agent.
    assert_eq!(result, Pick::Agent(&normal_agent));
}

#[test]
fn a_member_also_skips_workers_in_tier_normal_fallback() {
    let worker_agent = name("glm");
    let normal_agent = name("codex-1");
    let normal = [
        free(&worker_agent, "opencode", Tier::Worker),
        free(&normal_agent, "codex", Tier::Normal),
    ];

    let result = pick(Role::Member, &[], "claude", &[], &normal, &[], now(), false);

    assert_eq!(result, Pick::Agent(&normal_agent));
}

#[test]
fn a_coordinator_goes_to_an_investigator() {
    let normal_agent = name("codex-1");
    let investigator_agent = name("inv-opus");
    let replacement = [
        free(&normal_agent, "codex", Tier::Normal),
        free(&investigator_agent, "claude", Tier::Investigator),
    ];

    let result = pick(
        Role::Coordinator,
        &[],
        "openrouter",
        &replacement,
        &[],
        &[],
        now(),
        false,
    );

    // Skips the normal agent (coordinator needs an investigator), picks the investigator.
    assert_eq!(result, Pick::Agent(&investigator_agent));
}

#[test]
fn a_coordinator_falls_back_to_investigators_not_tier_normal() {
    let normal_agent = name("codex-1");
    let investigator_agent = name("inv-opus");
    let normal = [free(&normal_agent, "codex", Tier::Normal)];
    let investigators = [free(&investigator_agent, "claude", Tier::Investigator)];

    let result = pick(
        Role::Coordinator,
        &[],
        "openrouter",
        &[],
        &normal,
        &investigators,
        now(),
        false,
    );

    // Falls back to investigators, not tier normal.
    assert_eq!(result, Pick::Agent(&investigator_agent));
}

#[test]
fn a_solo_task_can_go_to_a_normal_agent_in_the_fallback() {
    let normal_agent = name("codex-1");
    let normal = [free(&normal_agent, "codex", Tier::Normal)];

    let result = pick(Role::Solo, &[], "openrouter", &[], &normal, &[], now(), false);

    assert_eq!(result, Pick::Agent(&normal_agent));
}

// --- None free with earliest return ---

#[test]
fn returns_earliest_return_when_all_are_refused() {
    let first = name("claude-2");
    let second = name("gemini-2");
    let early = now(); // Return at now is in the past → not refused. Let's use times in the future.
    let early_return: Timestamp = "2026-11-25T12:30:00Z".parse().unwrap();
    let late_return: Timestamp = "2026-11-25T13:00:00Z".parse().unwrap();
    let replacement = [
        refused(&first, "claude", Tier::Normal, late_return),
        refused(&second, "gemini", Tier::Normal, early_return),
    ];

    let result = pick(
        Role::Member,
        &[],
        "openrouter",
        &replacement,
        &[],
        &[],
        now(),
        false,
    );

    assert_eq!(
        result,
        Pick::NoneFree {
            earliest_return: Some(early_return)
        }
    );
}

#[test]
fn earliest_return_considers_all_candidate_groups() {
    let early_return: Timestamp = "2026-11-25T12:30:00Z".parse().unwrap();
    let late_return: Timestamp = "2026-11-25T13:00:00Z".parse().unwrap();
    let repl_agent = name("claude-2");
    let normal_agent = name("codex-1");
    let replacement = [refused(&repl_agent, "openrouter", Tier::Normal, late_return)];
    let normal = [refused(&normal_agent, "openrouter", Tier::Normal, early_return)];

    let result = pick(
        Role::Member,
        &[],
        "deepinfra",
        &replacement,
        &normal,
        &[],
        now(),
        false,
    );

    // Both are refused (same provider as refused but from different provider... wait,
    // they're refused AND same provider as refused? No: refused_provider is "deepinfra",
    // both candidates have "openrouter" provider, so they're NOT skipped for same provider.
    // But they ARE refused. So the earliest return comes from the normal agent.
    assert_eq!(
        result,
        Pick::NoneFree {
            earliest_return: Some(early_return)
        }
    );
}

#[test]
fn none_free_when_every_candidate_is_filtered_out() {
    let same_provider_agent = name("claude-2");
    let busy_agent = name("gemini-2");
    let replacement = [
        free(&same_provider_agent, "openrouter", Tier::Normal),
        busy(&busy_agent, "gemini", Tier::Normal),
    ];

    let result = pick(
        Role::Member,
        &[],
        "openrouter",
        &replacement,
        &[],
        &[],
        now(),
        false,
    );

    // First: same provider → skipped. Second: busy → skipped. None left.
    assert_eq!(
        result,
        Pick::NoneFree {
            earliest_return: None
        }
    );
}

// --- Replacements exhausted returns earliest return among refused ---

#[test]
fn replacements_exhausted_returns_earliest_refused_return() {
    let early_return: Timestamp = "2026-11-25T12:30:00Z".parse().unwrap();
    let agent = name("claude-2");
    let replacement = [refused(&agent, "claude", Tier::Normal, early_return)];

    let result = pick(
        Role::Member,
        &[],
        "openrouter",
        &replacement,
        &[],
        &[],
        now(),
        true,
    );

    assert_eq!(
        result,
        Pick::NoneFree {
            earliest_return: Some(early_return)
        }
    );
}

// --- Order: replacement agents before fallback ---

#[test]
fn a_fitting_agent_in_the_fallback_does_not_beat_a_fitting_replacement_agent() {
    let repl_agent = name("claude-2");
    let normal_agent = name("codex-1");
    let replacement = [free(&repl_agent, "claude", Tier::Normal)];
    let normal = [free(&normal_agent, "codex", Tier::Normal)];

    let result = pick(
        Role::Member,
        &[],
        "openrouter",
        &replacement,
        &normal,
        &[],
        now(),
        false,
    );

    assert_eq!(result, Pick::Agent(&repl_agent));
}
