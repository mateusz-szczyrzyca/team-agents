use super::*;

/// The names of the configured agents of these tests.
fn names() -> [AgentName; 3] {
    [agent("worker-1"), agent("claude-1"), agent("inv-opus")]
}

/// An agent name of the configuration.
fn agent(name: &str) -> AgentName {
    name.parse().unwrap()
}

/// A candidate that is idle.
fn idle(name: &AgentName, tier: Tier) -> Candidate<'_> {
    Candidate {
        agent: name,
        tier,
        activity: Activity::Idle,
    }
}

#[test]
fn every_tier_against_every_tier_takes_the_first_idle_agent_of_its_place_or_higher() {
    let [worker, normal, investigator] = names();
    let candidates = [
        idle(&worker, Tier::Worker),
        idle(&normal, Tier::Normal),
        idle(&investigator, Tier::Investigator),
    ];
    let cases = [
        (Tier::Worker, &worker),
        (Tier::Normal, &normal),
        (Tier::Investigator, &investigator),
    ];

    for (replaced, expected) in cases {
        assert_eq!(
            choose(Role::Member, replaced, &candidates, ReplacementMode::Auto),
            Handover::Agent(expected),
            "{replaced:?}"
        );
    }
}

#[test]
fn a_lower_tier_never_replaces_a_higher_one() {
    let [worker, normal, _] = names();
    let workers_only = [idle(&worker, Tier::Worker)];
    let workers_then_normal = [idle(&worker, Tier::Worker), idle(&normal, Tier::Normal)];

    assert_eq!(
        choose(Role::Member, Tier::Normal, &workers_only, ReplacementMode::Auto),
        Handover::Lead
    );
    assert_eq!(
        choose(Role::Member, Tier::Investigator, &workers_only, ReplacementMode::Auto),
        Handover::Lead
    );
    assert_eq!(
        choose(Role::Member, Tier::Normal, &workers_then_normal, ReplacementMode::Auto),
        Handover::Agent(&normal)
    );
}

#[test]
fn the_same_tier_may_replace() {
    let normal = agent("claude-1");

    assert_eq!(
        choose(
            Role::Member,
            Tier::Normal,
            &[idle(&normal, Tier::Normal)],
            ReplacementMode::Auto
        ),
        Handover::Agent(&normal)
    );
}

#[test]
fn the_first_fitting_agent_of_the_configuration_is_taken() {
    let first = agent("claude-1");
    let second = agent("claude-2");

    assert_eq!(
        choose(
            Role::Member,
            Tier::Normal,
            &[idle(&first, Tier::Normal), idle(&second, Tier::Normal)],
            ReplacementMode::Auto
        ),
        Handover::Agent(&first)
    );
}

#[test]
fn an_agent_in_the_middle_of_a_run_is_passed_over() {
    let busy = agent("claude-1");
    let free = agent("claude-2");
    let candidates = [
        Candidate {
            agent: &busy,
            tier: Tier::Normal,
            activity: Activity::InRun,
        },
        idle(&free, Tier::Normal),
    ];
    let all_busy = [Candidate {
        agent: &busy,
        tier: Tier::Normal,
        activity: Activity::InRun,
    }];

    assert_eq!(
        choose(Role::Member, Tier::Normal, &candidates, ReplacementMode::Auto),
        Handover::Agent(&free)
    );
    assert_eq!(
        choose(Role::Member, Tier::Normal, &all_busy, ReplacementMode::Auto),
        Handover::Lead
    );
}

#[test]
fn no_agent_to_take_a_members_work_gives_it_to_its_lead() {
    assert_eq!(
        choose(Role::Member, Tier::Normal, &[], ReplacementMode::Auto),
        Handover::Lead
    );
}

#[test]
fn a_solo_task_or_a_coordinators_work_that_nobody_takes_goes_to_the_human() {
    // A solo agent has no lead, and a coordinator is its own team's lead: neither gives the work to a lead.
    assert_eq!(
        choose(Role::Solo, Tier::Normal, &[], ReplacementMode::Auto),
        Handover::Human
    );
    assert_eq!(
        choose(Role::Coordinator, Tier::Investigator, &[], ReplacementMode::Auto),
        Handover::Human
    );
}

#[test]
fn a_workers_and_a_reviewers_task_is_the_humans() {
    for role in [Role::Worker, Role::Reviewer] {
        for mode in [ReplacementMode::Auto, ReplacementMode::Lead, ReplacementMode::Off] {
            assert_eq!(
                choose(role, Tier::Worker, &[], mode),
                Handover::Human,
                "{role:?} {mode:?}"
            );
        }
    }
}

#[test]
fn the_lead_mode_answers_the_lead_of_a_member_and_the_human_otherwise() {
    let investigator = agent("inv-opus");
    let candidates = [idle(&investigator, Tier::Investigator)];

    assert_eq!(
        choose(Role::Member, Tier::Normal, &candidates, ReplacementMode::Lead),
        Handover::Lead
    );
    for role in [Role::Solo, Role::Coordinator] {
        assert_eq!(
            choose(role, Tier::Normal, &candidates, ReplacementMode::Lead),
            Handover::Human,
            "{role:?}"
        );
    }
}

#[test]
fn the_off_mode_answers_the_human_whoever_waits() {
    let investigator = agent("inv-opus");
    let candidates = [idle(&investigator, Tier::Investigator)];

    for role in [Role::Solo, Role::Coordinator, Role::Member] {
        assert_eq!(
            choose(role, Tier::Normal, &candidates, ReplacementMode::Off),
            Handover::Human,
            "{role:?}"
        );
    }
}

#[test]
fn a_file_that_does_not_name_the_mode_has_auto_and_the_texts_name_the_three() {
    assert_eq!(ReplacementMode::default(), ReplacementMode::Auto);
    for (text, mode) in [
        ("\"auto\"", ReplacementMode::Auto),
        ("\"lead\"", ReplacementMode::Lead),
        ("\"off\"", ReplacementMode::Off),
    ] {
        assert_eq!(serde_json::from_str::<ReplacementMode>(text).unwrap(), mode, "{text}");
    }
}

#[test]
fn the_places_of_the_tiers_rank_a_worker_below_a_normal_agent_below_an_investigator() {
    assert!(rank(Tier::Worker) < rank(Tier::Normal));
    assert!(rank(Tier::Normal) < rank(Tier::Investigator));
}
