use crate::decide::replacement::refused::{Candidate, Pick, Request, pick};
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

/// A request for a member with an empty team, an empty normal list and no investigators.
fn default_request<'a>(
    refused_provider: &'a str,
    replacement: &'a [Candidate<'a>],
    normal: &'a [Candidate<'a>],
    investigators: &'a [Candidate<'a>],
) -> Request<'a> {
    Request {
        role: Role::Member,
        team: &[],
        refused_provider,
        replacement_agents: replacement,
        normal_agents: normal,
        investigators,
        now: now(),
        replacements_exhausted: false,
    }
}

#[test]
#[expect(
    clippy::too_many_lines,
    reason = "the shared decision table keeps all replacement scenarios together"
)]
fn chooses_replacements_by_role_availability_provider_and_return_time() {
    let first = name("claude-2");
    let second = name("gemini-2");
    let normal = name("codex-1");
    let investigator = name("inv-opus");
    let worker = name("glm");
    let team = [first.clone()];
    let early_return: Timestamp = "2026-11-25T12:30:00Z".parse().unwrap();
    let first_free = free(&first, "claude", Tier::Normal);
    let second_free = free(&second, "gemini", Tier::Normal);
    let normal_free = free(&normal, "codex", Tier::Normal);
    let investigator_free = free(&investigator, "claude", Tier::Investigator);
    let worker_free = free(&worker, "opencode", Tier::Worker);
    let same = free(&first, "openrouter", Tier::Normal);
    let occupied = busy(&first, "claude", Tier::Normal);
    let future = refused(&first, "claude", Tier::Normal, later());
    let past = refused(&first, "claude", Tier::Normal, earlier());
    let exact = refused(&first, "claude", Tier::Normal, now());
    let soon = refused(&second, "gemini", Tier::Normal, early_return);
    let none = Pick::NoneFree { earliest_return: None };
    let returns_soon = Pick::NoneFree {
        earliest_return: Some(early_return),
    };
    let primary = [first_free];
    let ordered_free = [first_free, second_free];
    let same_then_free = [same, second_free];
    let busy_then_free = [occupied, second_free];
    let refused_then_free = [future, second_free];
    let expired = [past];
    let opencode_providers = [
        free(&first, "deepinfra", Tier::Normal),
        free(&second, "openrouter", Tier::Normal),
    ];
    let same_provider = [same];
    let normal_only = [normal_free];
    let worker_then_normal = [worker_free, normal_free];
    let normal_then_investigator = [normal_free, investigator_free];
    let investigator_only = [investigator_free];
    let all_refused = [future, soon];
    let late_refusal = [future];
    let early_refusal = [soon];
    let all_unavailable = [same, busy(&second, "gemini", Tier::Normal)];
    let exact_then_free = [exact, second_free];
    let expired_busy = [Candidate { busy: true, ..past }];
    let exact_busy = [Candidate { busy: true, ..exact }];
    let investigator_refused = [refused(&investigator, "claude", Tier::Investigator, early_return)];
    let cases = [
        (
            "returns_none_free_when_no_candidates_are_given",
            default_request("openrouter", &[], &[], &[]),
            none,
        ),
        (
            "returns_none_free_when_replacements_are_exhausted",
            Request {
                replacements_exhausted: true,
                ..default_request("openrouter", &primary, &[], &[])
            },
            none,
        ),
        (
            "picks_the_first_free_agent_of_the_replacement_list",
            default_request("openrouter", &ordered_free, &[], &[]),
            Pick::Agent(&first),
        ),
        (
            "skips_a_replacement_agent_of_the_same_provider",
            default_request("openrouter", &same_then_free, &[], &[]),
            Pick::Agent(&second),
        ),
        (
            "skips_a_busy_replacement_agent",
            default_request("openrouter", &busy_then_free, &[], &[]),
            Pick::Agent(&second),
        ),
        (
            "skips_a_refused_replacement_agent",
            default_request("openrouter", &refused_then_free, &[], &[]),
            Pick::Agent(&second),
        ),
        (
            "an_agent_refused_until_a_past_time_is_not_refused_now",
            default_request("openrouter", &expired, &[], &[]),
            Pick::Agent(&first),
        ),
        (
            "skips_a_replacement_agent_in_the_team",
            Request {
                team: &team,
                ..default_request("openrouter", &ordered_free, &[], &[])
            },
            Pick::Agent(&second),
        ),
        (
            "deepinfra_and_openrouter_count_as_two_providers",
            default_request("deepinfra", &opencode_providers, &[], &[]),
            Pick::Agent(&second),
        ),
        (
            "falls_back_to_tier_normal_when_no_replacement_agent_fits",
            default_request("openrouter", &same_provider, &normal_only, &[]),
            Pick::Agent(&normal),
        ),
        (
            "falls_back_to_tier_normal_when_replacement_list_is_empty",
            default_request("openrouter", &[], &normal_only, &[]),
            Pick::Agent(&normal),
        ),
        (
            "tier_normal_respects_the_same_skip_rules",
            default_request("openrouter", &[], &same_then_free, &[]),
            Pick::Agent(&second),
        ),
        (
            "a_member_never_goes_to_a_worker",
            default_request("claude", &worker_then_normal, &[], &[]),
            Pick::Agent(&normal),
        ),
        (
            "a_member_also_skips_workers_in_tier_normal_fallback",
            default_request("claude", &[], &worker_then_normal, &[]),
            Pick::Agent(&normal),
        ),
        (
            "a_coordinator_goes_to_an_investigator",
            Request {
                role: Role::Coordinator,
                ..default_request("openrouter", &normal_then_investigator, &[], &[])
            },
            Pick::Agent(&investigator),
        ),
        (
            "a_coordinator_falls_back_to_investigators_not_tier_normal",
            Request {
                role: Role::Coordinator,
                ..default_request("openrouter", &[], &normal_only, &investigator_only)
            },
            Pick::Agent(&investigator),
        ),
        (
            "a_solo_task_can_go_to_a_normal_agent_in_the_fallback",
            Request {
                role: Role::Solo,
                ..default_request("openrouter", &[], &normal_only, &[])
            },
            Pick::Agent(&normal),
        ),
        (
            "returns_earliest_return_when_all_are_refused",
            default_request("openrouter", &all_refused, &[], &[]),
            returns_soon,
        ),
        (
            "earliest_return_considers_all_candidate_groups",
            default_request("openrouter", &late_refusal, &early_refusal, &[]),
            returns_soon,
        ),
        (
            "none_free_when_every_candidate_is_filtered_out",
            default_request("openrouter", &all_unavailable, &[], &[]),
            none,
        ),
        (
            "replacements_exhausted_returns_earliest_refused_return",
            Request {
                replacements_exhausted: true,
                ..default_request("openrouter", &early_refusal, &[], &[])
            },
            returns_soon,
        ),
        (
            "a_fitting_agent_in_the_fallback_does_not_beat_a_fitting_replacement_agent",
            default_request("openrouter", &primary, &normal_only, &[]),
            Pick::Agent(&first),
        ),
        (
            "exact_return_is_eligible",
            default_request("openrouter", &exact_then_free, &[], &[]),
            Pick::Agent(&first),
        ),
        (
            "expired_only_refusals_do_not_supply_a_return",
            default_request("openrouter", &expired_busy, &[], &[]),
            none,
        ),
        (
            "exact_expiry_does_not_supply_a_return",
            default_request("openrouter", &exact_busy, &[], &[]),
            none,
        ),
        (
            "coordinator_earliest_return_comes_from_investigators",
            Request {
                role: Role::Coordinator,
                ..default_request("openrouter", &late_refusal, &[], &investigator_refused)
            },
            returns_soon,
        ),
    ];
    for (label, request, expected) in cases {
        assert_eq!(pick(&request), expected, "{label}");
    }
}
