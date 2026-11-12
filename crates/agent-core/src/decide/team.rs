//! Team formation: from the agents named in `agent run`, the configuration, the coordinator asked for and a random
//! source to a [`TeamPlan`], or a typed refusal.
//!
//! The rules, as `docs/architecture.md` states them:
//!
//! - One agent is a solo task; more than one is a team.
//! - The coordinator is the explicit one — `--coordinator`, which wins over the task list's `Coordinator:` line —;
//!   otherwise an investigator of the team (drawn among two or more); otherwise a normal agent, drawn. A worker never
//!   coordinates and never takes items: it serves `delegate` requests, the first free one in the configured order.
//! - Every pull request gets [`REVIEWS_REQUIRED`] reviews from investigators outside the team, so a team that leaves
//!   fewer configured investigators outside it is refused.
//!
//! The checks run in this order and the first fault found is the one reported: every requested agent is configured;
//! the explicit coordinator is configured, is not a worker and is one of the requested agents; some requested agent
//! can take items; a team has a member besides its coordinator; enough investigators stay outside.

use rand::Rng;

use super::REVIEWS_REQUIRED;
use crate::config::Config;
use crate::ids::AgentName;
use crate::protocol::AgentList;
use crate::state::Participant;

/// What `agent run` asks for: the agents and, if any, the coordinator.
#[derive(Debug, Clone, Copy)]
pub struct TeamRequest<'a> {
    /// The agents named on the command line, in the order given: at least one, none twice.
    pub agents: &'a AgentList,
    /// The coordinator named by `--coordinator`; it wins over [`TeamRequest::listed_coordinator`].
    pub flag_coordinator: Option<&'a AgentName>,
    /// The coordinator named by the `Coordinator:` line of the task list.
    pub listed_coordinator: Option<&'a AgentName>,
}

/// Who is in a team and in which role, as [`plan_team`] decided; the daemon turns it into tasks and a crew.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TeamPlan {
    /// One agent alone: a normal agent or an investigator.
    Solo {
        /// The agent that does the work and integrates it itself.
        agent: Participant,
    },
    /// A coordinator, the members who take items and the workers who serve `delegate` requests.
    Team(TeamRoster),
}

/// The people of a team.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TeamRoster {
    /// The coordinator: a normal agent or an investigator.
    pub coordinator: Participant,
    /// The members who take items, normal agents or investigators, in the order they were named; at least one.
    pub members: Vec<Participant>,
    /// The workers, in the order of the configuration, which is the order of delegation.
    pub workers: Vec<Participant>,
}

/// Why a team cannot be formed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TeamPlanError {
    /// A named agent or coordinator is not in the configuration.
    #[error("{name} is not an agent or investigator of the configuration")]
    UnknownAgent {
        /// The unknown name.
        name: AgentName,
    },
    /// The coordinator asked for is a worker, and workers never coordinate.
    #[error("{name} is a worker: workers serve delegate requests and never coordinate")]
    WorkerCoordinator {
        /// The worker named as coordinator.
        name: AgentName,
    },
    /// The coordinator asked for is not one of the agents named.
    #[error("the coordinator {name} is not among the agents named: name it as an agent too")]
    CoordinatorNotRequested {
        /// The coordinator that is not a requested agent.
        name: AgentName,
    },
    /// Only workers are named: none of them can take items or coordinate.
    #[error("only workers are named: name a normal agent or an investigator too")]
    OnlyWorkers {
        /// The workers named.
        workers: Vec<AgentName>,
    },
    /// A team needs a member besides its coordinator, and the others named are workers.
    #[error("{coordinator} would coordinate, but the others named are workers, which take no items")]
    NoMember {
        /// The coordinator the team would have.
        coordinator: AgentName,
        /// The workers named.
        workers: Vec<AgentName>,
    },
    /// The team's investigators leave too few outside it to review its pull requests.
    #[error(
        "too many investigators in the team: {in_team} of the {configured} configured leave fewer than {required} outside it to review"
    )]
    TooManyInvestigators {
        /// How many investigators the team has.
        in_team: usize,
        /// How many investigators the configuration has.
        configured: usize,
        /// How many outside investigators every pull request needs.
        required: usize,
    },
    /// The configuration has too few investigators to review a pull request, whoever is in the team.
    #[error(
        "{configured} investigators are configured; every pull request needs reviews from {required} outside the team"
    )]
    TooFewInvestigators {
        /// How many investigators the configuration has.
        configured: usize,
        /// How many outside investigators every pull request needs.
        required: usize,
    },
}

/// Forms the team `request` asks for.
///
/// `rng` draws the coordinator when the request does not name one and more than one agent could be it.
///
/// # Errors
/// A [`TeamPlanError`], by the order of checks in the module documentation.
pub fn plan_team(request: &TeamRequest<'_>, _config: &Config, _rng: &mut impl Rng) -> Result<TeamPlan, TeamPlanError> {
    let first = request.agents.as_slice().first().cloned();
    match first {
        Some(name) => Ok(TeamPlan::Solo {
            agent: Participant::Normal(name),
        }),
        None => Err(TeamPlanError::TooFewInvestigators {
            configured: 0,
            required: REVIEWS_REQUIRED,
        }),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use rand::SeedableRng;
    use rand::rngs::Xoshiro256PlusPlus;

    use super::*;
    use crate::ids::InvestigatorName;

    /// Three normal agents, two workers (`glm` first, then `qwen`) and three investigators.
    const CONFIG: &str = r#"
[agents.claude-1]
cli = "claude"
[agents.codex-1]
cli = "codex"
[agents.gemini-1]
cli = "gemini"
[agents.glm]
cli = "opencode"
[agents.qwen]
cli = "opencode"

[investigators.inv-opus]
cli = "claude"
[investigators.inv-codex]
cli = "codex"
[investigators.inv-gemini]
cli = "gemini"

[tiers]
normal = ["claude-1", "codex-1", "gemini-1"]
workers = ["glm", "qwen"]
"#;

    /// The configuration with two investigators, for the cases that leave too few outside a team.
    const TWO_INVESTIGATORS: &str = r#"
[agents.claude-1]
cli = "claude"
[agents.codex-1]
cli = "codex"
[investigators.inv-opus]
cli = "claude"
[investigators.inv-codex]
cli = "codex"
[tiers]
normal = ["claude-1", "codex-1"]
"#;

    /// The configuration with one investigator.
    const ONE_INVESTIGATOR: &str = r#"
[agents.claude-1]
cli = "claude"
[agents.codex-1]
cli = "codex"
[investigators.inv-opus]
cli = "claude"
[tiers]
normal = ["claude-1", "codex-1"]
"#;

    /// The configuration without an investigator.
    const NO_INVESTIGATOR: &str = r#"
[agents.claude-1]
cli = "claude"
[tiers]
normal = ["claude-1"]
"#;

    fn config(text: &str) -> Config {
        Config::from_toml_str(text).unwrap()
    }

    fn name(text: &str) -> AgentName {
        text.parse().unwrap()
    }

    fn normal(text: &str) -> Participant {
        Participant::Normal(name(text))
    }

    fn worker(text: &str) -> Participant {
        Participant::Worker(name(text))
    }

    fn investigator(text: &str) -> Participant {
        Participant::Investigator(text.parse::<InvestigatorName>().unwrap())
    }

    fn agents(names: &[&str]) -> AgentList {
        AgentList::try_from(names.iter().map(|n| name(n)).collect::<Vec<_>>()).unwrap()
    }

    fn rng(seed: u64) -> Xoshiro256PlusPlus {
        Xoshiro256PlusPlus::seed_from_u64(seed)
    }

    /// Plans the team of `names` in `CONFIG` with the given coordinators and the random source of `seed`.
    fn plan(names: &[&str], flag: Option<&str>, listed: Option<&str>, seed: u64) -> Result<TeamPlan, TeamPlanError> {
        plan_in(CONFIG, names, flag, listed, seed)
    }

    fn plan_in(
        text: &str,
        names: &[&str],
        flag: Option<&str>,
        listed: Option<&str>,
        seed: u64,
    ) -> Result<TeamPlan, TeamPlanError> {
        let list = agents(names);
        let flag = flag.map(name);
        let listed = listed.map(name);
        let request = TeamRequest {
            agents: &list,
            flag_coordinator: flag.as_ref(),
            listed_coordinator: listed.as_ref(),
        };
        plan_team(&request, &config(text), &mut rng(seed))
    }

    /// The roster of a plan that is a team.
    fn roster(plan: Result<TeamPlan, TeamPlanError>) -> TeamRoster {
        match plan.unwrap() {
            TeamPlan::Team(roster) => roster,
            solo @ TeamPlan::Solo { .. } => panic!("not a team: {solo:?}"),
        }
    }

    /// The coordinators that seeds 0 to 63 give for the team of `names` in `CONFIG`.
    fn coordinators_over_seeds(names: &[&str]) -> BTreeSet<String> {
        (0..64)
            .map(|seed| roster(plan(names, None, None, seed)).coordinator.name().to_owned())
            .collect()
    }

    // Rule: one agent is a solo task.
    #[test]
    fn one_agent_is_a_solo_task() {
        assert_eq!(
            plan(&["claude-1"], None, None, 0),
            Ok(TeamPlan::Solo {
                agent: normal("claude-1")
            })
        );
    }

    // Rule: an investigator may work alone as well.
    #[test]
    fn one_investigator_is_a_solo_task() {
        assert_eq!(
            plan(&["inv-opus"], None, None, 0),
            Ok(TeamPlan::Solo {
                agent: investigator("inv-opus")
            })
        );
    }

    // Rule: a solo agent that is also named as the coordinator stays solo.
    #[test]
    fn a_solo_agent_named_as_coordinator_stays_solo() {
        assert_eq!(
            plan(&["codex-1"], Some("codex-1"), None, 0),
            Ok(TeamPlan::Solo {
                agent: normal("codex-1")
            })
        );
    }

    // Rule: more than one agent is a team.
    #[test]
    fn more_than_one_agent_is_a_team() {
        let team = roster(plan(&["claude-1", "codex-1"], None, None, 0));
        assert_eq!(team.members.len(), 1);
        assert_eq!(team.workers, vec![]);
        let names: BTreeSet<_> = [&team.coordinator]
            .into_iter()
            .chain(&team.members)
            .map(|p| p.name().to_owned())
            .collect();
        assert_eq!(names, BTreeSet::from(["claude-1".to_owned(), "codex-1".to_owned()]));
    }

    // Rule: the coordinator asked for with --coordinator wins over an investigator in the team.
    #[test]
    fn the_flag_coordinator_wins_over_an_investigator() {
        let team = roster(plan(&["inv-opus", "claude-1", "codex-1"], Some("codex-1"), None, 3));
        assert_eq!(team.coordinator, normal("codex-1"));
        assert_eq!(team.members, vec![investigator("inv-opus"), normal("claude-1")]);
    }

    // Rule: the task list's Coordinator: line wins when --coordinator is absent.
    #[test]
    fn the_listed_coordinator_wins_over_an_investigator() {
        let team = roster(plan(&["inv-opus", "claude-1"], None, Some("claude-1"), 3));
        assert_eq!(team.coordinator, normal("claude-1"));
        assert_eq!(team.members, vec![investigator("inv-opus")]);
    }

    // Rule: --coordinator wins over the task list's Coordinator: line.
    #[test]
    fn the_flag_coordinator_wins_over_the_listed_one() {
        let team = roster(plan(&["claude-1", "codex-1"], Some("codex-1"), Some("claude-1"), 0));
        assert_eq!(team.coordinator, normal("codex-1"));
        assert_eq!(team.members, vec![normal("claude-1")]);
    }

    // Rule: an explicit coordinator may be an investigator.
    #[test]
    fn an_explicit_coordinator_may_be_an_investigator() {
        let team = roster(plan(&["claude-1", "inv-codex"], Some("inv-codex"), None, 0));
        assert_eq!(team.coordinator, investigator("inv-codex"));
        assert_eq!(team.members, vec![normal("claude-1")]);
    }

    // Rule: without an explicit coordinator an investigator of the team coordinates.
    #[test]
    fn the_only_investigator_of_the_team_coordinates() {
        for seed in 0..16 {
            let team = roster(plan(&["claude-1", "inv-gemini", "codex-1"], None, None, seed));
            assert_eq!(team.coordinator, investigator("inv-gemini"));
            assert_eq!(team.members, vec![normal("claude-1"), normal("codex-1")]);
        }
    }

    // Rule: among two or more investigators the coordinator is drawn, and only among them.
    #[test]
    fn the_coordinator_is_drawn_among_the_investigators_of_the_team() {
        let drawn = coordinators_over_seeds(&["claude-1", "inv-opus", "codex-1", "inv-codex"]);
        assert_eq!(drawn, BTreeSet::from(["inv-opus".to_owned(), "inv-codex".to_owned()]));
        let team = roster(plan(&["claude-1", "inv-opus", "codex-1", "inv-codex"], None, None, 1));
        assert_eq!(team.members.len(), 3);
        assert!(!team.members.contains(&team.coordinator));
    }

    // Rule: the draw is a function of the random source: the same seed gives the same coordinator.
    #[test]
    fn the_same_random_source_draws_the_same_coordinator() {
        for seed in 0..16 {
            let names = ["inv-opus", "inv-codex", "claude-1"];
            assert_eq!(plan(&names, None, None, seed), plan(&names, None, None, seed));
        }
    }

    // Rule: without investigators a normal agent coordinates, drawn among the normal agents named.
    #[test]
    fn a_normal_agent_is_drawn_when_no_investigator_is_in_the_team() {
        let drawn = coordinators_over_seeds(&["claude-1", "codex-1", "gemini-1"]);
        assert_eq!(
            drawn,
            BTreeSet::from(["claude-1".to_owned(), "codex-1".to_owned(), "gemini-1".to_owned()])
        );
    }

    // Rule: workers never coordinate, whatever the draw.
    #[test]
    fn a_worker_is_never_drawn_as_coordinator() {
        let drawn = coordinators_over_seeds(&["glm", "claude-1", "qwen", "codex-1"]);
        assert_eq!(drawn, BTreeSet::from(["claude-1".to_owned(), "codex-1".to_owned()]));
    }

    // Rule: a worker named as the coordinator is refused.
    #[test]
    fn a_worker_as_the_explicit_coordinator_is_refused() {
        assert_eq!(
            plan(&["claude-1", "glm"], Some("glm"), None, 0),
            Err(TeamPlanError::WorkerCoordinator { name: name("glm") })
        );
        assert_eq!(
            plan(&["claude-1", "glm"], None, Some("glm"), 0),
            Err(TeamPlanError::WorkerCoordinator { name: name("glm") })
        );
    }

    // Rule: an agent that is not configured is refused, the first of those named.
    #[test]
    fn an_unknown_agent_is_refused() {
        assert_eq!(
            plan(&["claude-1", "nobody", "other"], None, None, 0),
            Err(TeamPlanError::UnknownAgent { name: name("nobody") })
        );
        assert_eq!(
            plan(&["nobody"], None, None, 0),
            Err(TeamPlanError::UnknownAgent { name: name("nobody") })
        );
    }

    // Rule: a coordinator that is not configured is refused.
    #[test]
    fn an_unknown_coordinator_is_refused() {
        assert_eq!(
            plan(&["claude-1", "codex-1"], Some("nobody"), None, 0),
            Err(TeamPlanError::UnknownAgent { name: name("nobody") })
        );
    }

    // Rule: an unknown requested agent is reported before a coordinator fault.
    #[test]
    fn an_unknown_agent_is_reported_before_a_worker_coordinator() {
        assert_eq!(
            plan(&["nobody", "glm"], Some("glm"), None, 0),
            Err(TeamPlanError::UnknownAgent { name: name("nobody") })
        );
    }

    // Rule: a coordinator that is not among the agents named is refused.
    #[test]
    fn a_coordinator_that_is_not_requested_is_refused() {
        assert_eq!(
            plan(&["claude-1", "codex-1"], Some("gemini-1"), None, 0),
            Err(TeamPlanError::CoordinatorNotRequested { name: name("gemini-1") })
        );
        assert_eq!(
            plan(&["claude-1"], Some("codex-1"), None, 0),
            Err(TeamPlanError::CoordinatorNotRequested { name: name("codex-1") })
        );
    }

    // Rule: a worker alone, or workers alone, take no items and cannot coordinate: refused.
    #[test]
    fn only_workers_are_refused() {
        assert_eq!(
            plan(&["glm"], None, None, 0),
            Err(TeamPlanError::OnlyWorkers {
                workers: vec![name("glm")]
            })
        );
        assert_eq!(
            plan(&["qwen", "glm"], None, None, 0),
            Err(TeamPlanError::OnlyWorkers {
                workers: vec![name("qwen"), name("glm")]
            })
        );
    }

    // Rule: a team needs a member who takes items besides its coordinator; workers are no such member.
    #[test]
    fn a_coordinator_with_only_workers_is_refused() {
        assert_eq!(
            plan(&["claude-1", "glm", "qwen"], None, None, 0),
            Err(TeamPlanError::NoMember {
                coordinator: name("claude-1"),
                workers: vec![name("glm"), name("qwen")]
            })
        );
    }

    // Rule: workers serve delegate requests in the order of the configuration, not the order named.
    #[test]
    fn workers_are_in_the_configured_order() {
        let team = roster(plan(&["qwen", "claude-1", "glm", "codex-1"], Some("claude-1"), None, 0));
        assert_eq!(team.workers, vec![worker("glm"), worker("qwen")]);
        assert_eq!(team.members, vec![normal("codex-1")]);
    }

    // Rule: the members are in the order they were named, without the coordinator and the workers.
    #[test]
    fn members_keep_the_order_they_were_named() {
        let team = roster(plan(&["gemini-1", "codex-1", "claude-1"], Some("codex-1"), None, 0));
        assert_eq!(team.members, vec![normal("gemini-1"), normal("claude-1")]);
    }

    // Rule: every pull request needs two reviews from investigators outside the team: a team with one of three
    // configured investigators leaves two.
    #[test]
    fn one_investigator_of_three_leaves_two_outside() {
        let team = roster(plan(&["claude-1", "inv-opus"], None, None, 0));
        assert_eq!(team.coordinator, investigator("inv-opus"));
        assert_eq!(team.members, vec![normal("claude-1")]);
    }

    // Rule: a team with two of three configured investigators is refused: "too many investigators in the team".
    #[test]
    fn two_investigators_of_three_are_too_many() {
        assert_eq!(
            plan(&["claude-1", "inv-opus", "inv-codex"], None, None, 0),
            Err(TeamPlanError::TooManyInvestigators {
                in_team: 2,
                configured: 3,
                required: 2
            })
        );
        assert_eq!(
            plan(&["inv-opus", "inv-codex", "inv-gemini"], None, None, 0),
            Err(TeamPlanError::TooManyInvestigators {
                in_team: 3,
                configured: 3,
                required: 2
            })
        );
    }

    // Rule: the investigators counted are those of the team, the coordinator among them.
    #[test]
    fn an_investigator_coordinator_counts_as_in_the_team() {
        assert_eq!(
            plan(&["claude-1", "inv-opus", "inv-codex"], Some("inv-opus"), None, 0),
            Err(TeamPlanError::TooManyInvestigators {
                in_team: 2,
                configured: 3,
                required: 2
            })
        );
    }

    // Rule: a solo investigator leaves the others outside, so it needs two others.
    #[test]
    fn a_solo_investigator_needs_two_others_outside() {
        assert_eq!(
            plan_in(CONFIG, &["inv-opus"], None, None, 0),
            Ok(TeamPlan::Solo {
                agent: investigator("inv-opus")
            })
        );
        assert_eq!(
            plan_in(TWO_INVESTIGATORS, &["inv-opus"], None, None, 0),
            Err(TeamPlanError::TooManyInvestigators {
                in_team: 1,
                configured: 2,
                required: 2
            })
        );
    }

    // Rule: a team with an investigator in a configuration of two is refused.
    #[test]
    fn one_investigator_of_two_is_too_many() {
        assert_eq!(
            plan_in(TWO_INVESTIGATORS, &["claude-1", "inv-codex"], None, None, 0),
            Err(TeamPlanError::TooManyInvestigators {
                in_team: 1,
                configured: 2,
                required: 2
            })
        );
    }

    // Rule: a configuration with fewer than two investigators cannot review any team's pull request.
    #[test]
    fn too_few_configured_investigators_are_refused_even_without_one_in_the_team() {
        assert_eq!(
            plan_in(ONE_INVESTIGATOR, &["claude-1", "codex-1"], None, None, 0),
            Err(TeamPlanError::TooFewInvestigators {
                configured: 1,
                required: 2
            })
        );
        assert_eq!(
            plan_in(NO_INVESTIGATOR, &["claude-1"], None, None, 0),
            Err(TeamPlanError::TooFewInvestigators {
                configured: 0,
                required: 2
            })
        );
        let team = roster(plan_in(TWO_INVESTIGATORS, &["claude-1", "codex-1"], None, None, 0));
        assert_eq!(team.members.len(), 1);
    }

    // Rule: the shape of the team is checked before the reviewer count.
    #[test]
    fn a_missing_member_is_reported_before_the_reviewer_count() {
        assert_eq!(
            plan_in(TWO_INVESTIGATORS, &["inv-opus", "glm"], None, None, 0),
            Err(TeamPlanError::NoMember {
                coordinator: name("inv-opus"),
                workers: vec![name("glm")]
            })
        );
    }

    // Rule: a worker in the team is no investigator, whatever its model: only configured investigators count.
    #[test]
    fn workers_do_not_count_as_investigators_in_the_team() {
        let team = roster(plan(&["claude-1", "codex-1", "glm", "inv-opus"], None, None, 0));
        assert_eq!(team.coordinator, investigator("inv-opus"));
        assert_eq!(team.workers, vec![worker("glm")]);
    }
}
