//! How the people of a team got their roles, explained from the request and the configuration: the plan the history
//! records at the start of a team. The coordinator `plan_team` draws is always one of the candidates the explanation
//! names; the draws come from a seeded source and a failure names its seed.

use std::num::NonZeroU32;

use agent_core::config::Config;
use agent_core::decide::team::{TeamPlan, TeamRequest, plan_team};
use agent_core::history::{ExplainError, HowChosen, PlanEntry, explain_plan};
use agent_core::ids::AgentName;
use agent_core::kinds::Role;
use agent_core::protocol::AgentList;
use rand::SeedableRng;
use rand::rngs::Xoshiro256PlusPlus;

/// Two normal agents, a worker and three investigators.
const CONFIG: &str = "\
[agents.a1]\ncli = \"claude\"\n[agents.a2]\ncli = \"codex\"\n[agents.w1]\ncli = \"opencode\"\n\
[investigators.i1]\ncli = \"claude\"\n[investigators.i2]\ncli = \"claude\"\n[investigators.i3]\ncli = \"claude\"\n\
[tiers]\nnormal = [\"a1\", \"a2\"]\nworkers = [\"w1\"]\n";

fn config() -> Config {
    Config::from_toml_str(CONFIG).unwrap()
}

fn name(text: &str) -> AgentName {
    text.parse().unwrap()
}

fn names(texts: &[&str]) -> Vec<AgentName> {
    texts.iter().map(|text| name(text)).collect()
}

fn list(texts: &[&str]) -> AgentList {
    AgentList::try_from(names(texts)).unwrap()
}

fn position(n: u32) -> NonZeroU32 {
    NonZeroU32::new(n).unwrap()
}

fn entry(agent: &str, n: u32, role: Role, how: HowChosen) -> PlanEntry {
    PlanEntry {
        agent: name(agent),
        position: position(n),
        role,
        how,
    }
}

fn crew(people: &[(Role, &str)]) -> Vec<(Role, String)> {
    people.iter().map(|(role, who)| (*role, (*who).to_owned())).collect()
}

#[test]
fn a_solo_agent_is_named() {
    let agents = list(&["a1"]);
    let request = TeamRequest {
        agents: &agents,
        flag_coordinator: None,
        listed_coordinator: None,
    };

    let plan = explain_plan(&request, &config(), &crew(&[(Role::Solo, "a1")])).unwrap();

    assert_eq!(plan, [entry("a1", 1, Role::Solo, HowChosen::Named)]);
}

#[test]
fn the_flag_names_the_coordinator_and_wins_over_the_line() {
    let agents = list(&["a1", "a2"]);
    let (flag, line) = (name("a2"), name("a1"));
    let request = TeamRequest {
        agents: &agents,
        flag_coordinator: Some(&flag),
        listed_coordinator: Some(&line),
    };

    let plan = explain_plan(
        &request,
        &config(),
        &crew(&[(Role::Coordinator, "a2"), (Role::Member, "a1")]),
    )
    .unwrap();

    assert_eq!(
        plan,
        [
            entry("a2", 2, Role::Coordinator, HowChosen::NamedByFlag),
            entry("a1", 1, Role::Member, HowChosen::Named),
        ]
    );
}

#[test]
fn the_line_names_the_coordinator_when_there_is_no_flag() {
    let agents = list(&["a1", "a2"]);
    let line = name("a1");
    let request = TeamRequest {
        agents: &agents,
        flag_coordinator: None,
        listed_coordinator: Some(&line),
    };

    let plan = explain_plan(
        &request,
        &config(),
        &crew(&[(Role::Coordinator, "a1"), (Role::Member, "a2")]),
    )
    .unwrap();

    assert_eq!(
        plan,
        [
            entry("a1", 1, Role::Coordinator, HowChosen::NamedByLine),
            entry("a2", 2, Role::Member, HowChosen::Named),
        ]
    );
}

#[test]
fn a_coordinator_nobody_named_is_drawn_among_the_investigators_of_the_team() {
    let agents = list(&["a1", "i1", "a2", "i2"]);
    let request = TeamRequest {
        agents: &agents,
        flag_coordinator: None,
        listed_coordinator: None,
    };

    let plan = explain_plan(
        &request,
        &config(),
        &crew(&[
            (Role::Coordinator, "i2"),
            (Role::Member, "a1"),
            (Role::Member, "i1"),
            (Role::Member, "a2"),
        ]),
    )
    .unwrap();

    assert_eq!(
        plan,
        [
            entry("a1", 1, Role::Member, HowChosen::Named),
            entry("i1", 2, Role::Member, HowChosen::Named),
            entry("a2", 3, Role::Member, HowChosen::Named),
            entry(
                "i2",
                4,
                Role::Coordinator,
                HowChosen::Drawn {
                    among: names(&["i1", "i2"])
                }
            ),
        ]
    );
}

#[test]
fn a_coordinator_is_drawn_among_the_agents_that_take_items_when_no_investigator_is_named() {
    let agents = list(&["w1", "a1", "a2"]);
    let request = TeamRequest {
        agents: &agents,
        flag_coordinator: None,
        listed_coordinator: None,
    };

    let plan = explain_plan(
        &request,
        &config(),
        &crew(&[(Role::Coordinator, "a2"), (Role::Member, "a1"), (Role::Worker, "w1")]),
    )
    .unwrap();

    assert_eq!(
        plan,
        [
            entry("w1", 1, Role::Worker, HowChosen::ConfiguredOrder),
            entry("a1", 2, Role::Member, HowChosen::Named),
            entry(
                "a2",
                3,
                Role::Coordinator,
                HowChosen::Drawn {
                    among: names(&["a1", "a2"])
                }
            ),
        ]
    );
}

#[test]
fn someone_the_request_did_not_name_cannot_be_explained() {
    let agents = list(&["a1"]);
    let request = TeamRequest {
        agents: &agents,
        flag_coordinator: None,
        listed_coordinator: None,
    };

    let error = explain_plan(&request, &config(), &crew(&[(Role::Solo, "a2")])).unwrap_err();

    assert_eq!(error, ExplainError::NotNamed { agent: "a2".to_owned() });
}

#[test]
fn a_reviewer_is_not_part_of_the_plan_of_a_team() {
    let agents = list(&["a1", "i1"]);
    let request = TeamRequest {
        agents: &agents,
        flag_coordinator: None,
        listed_coordinator: None,
    };

    let error = explain_plan(&request, &config(), &crew(&[(Role::Reviewer, "i1")])).unwrap_err();

    assert_eq!(
        error,
        ExplainError::NotFormed {
            agent: "i1".to_owned(),
            role: Role::Reviewer
        }
    );
}

/// The crew of a plan `plan_team` made, as `(role, name)`.
fn crew_of(plan: &TeamPlan) -> Vec<(Role, String)> {
    match plan {
        TeamPlan::Solo { agent } => vec![(Role::Solo, agent.name().to_owned())],
        TeamPlan::Team(roster) => std::iter::once((Role::Coordinator, roster.coordinator.name().to_owned()))
            .chain(
                roster
                    .members
                    .iter()
                    .map(|person| (Role::Member, person.name().to_owned())),
            )
            .chain(
                roster
                    .workers
                    .iter()
                    .map(|person| (Role::Worker, person.name().to_owned())),
            )
            .collect(),
    }
}

#[test]
fn the_coordinator_plan_team_draws_is_one_the_explanation_names_for_any_request_and_seed() {
    let config = config();
    let pool = ["a1", "a2", "w1", "i1", "i2"];
    for seed in 0..300_u64 {
        let mut rng = Xoshiro256PlusPlus::seed_from_u64(seed);
        // A request drawn from the pool: each agent is in or out by the bits of the seed, at least two agents.
        let chosen: Vec<&str> = pool
            .iter()
            .enumerate()
            .filter(|(index, _)| (seed.rotate_right(3) >> index) & 1 == 1)
            .map(|(_, who)| *who)
            .collect();
        if chosen.len() < 2 {
            continue;
        }
        let agents = list(&chosen);
        let request = TeamRequest {
            agents: &agents,
            flag_coordinator: None,
            listed_coordinator: None,
        };
        let Ok(plan) = plan_team(&request, &config, &mut rng) else {
            continue;
        };

        let explained = explain_plan(&request, &config, &crew_of(&plan)).unwrap();

        let coordinator = explained
            .iter()
            .find(|each| matches!(each.role, Role::Coordinator | Role::Solo))
            .unwrap();
        match &coordinator.how {
            HowChosen::Drawn { among } => {
                assert!(
                    among.contains(&coordinator.agent),
                    "seed {seed}: {coordinator:?} is not in {among:?}"
                );
            }
            HowChosen::Named => assert_eq!(coordinator.role, Role::Solo, "seed {seed}"),
            other => panic!("seed {seed}: a request with no coordinator named gave {other:?}"),
        }
        assert_eq!(explained.len(), chosen.len(), "seed {seed}");
    }
}
