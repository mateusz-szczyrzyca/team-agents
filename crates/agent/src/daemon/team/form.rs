//! Forming a team: from a `run` request to the team, the workspaces of its members and its team on the bus.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::str::FromStr;

use agent_core::config::{AbsolutePath, Config, InvalidValue};
use agent_core::decide::team::{TeamPlan, TeamPlanError, TeamRequest, plan_team};
use agent_core::ids::{
    AgentName, BranchName, Clock, GenerateIdError, InvalidBranchName, InvalidName, ItemId, RoundId, TaskId, TeamId,
};
use agent_core::kinds::{CliKind, Role};
use agent_core::protocol::{Failure, FailureKind, RunRequest};
use agent_core::state::{
    AdvanceError, Crew, CrewError, InvalidTimestamp, Member, OpenRoundError, Participant, Team, TeamMembers, Timestamp,
};
use agent_core::task_list::Item;
use agent_runtime::agentbus::{BusTier, MemberState, NewMember, NewTeam};
use agent_runtime::store::Home;
use rand::Rng;

use super::assign::{Assigned, AssignmentError, agent_name, assignable, start_item, task_branch};

/// How many times a task ID is drawn again when it is one the team has already: two IDs of one second differ in four
/// characters drawn at random, so a second draw is all but certain to be free.
const ID_ATTEMPTS: usize = 8;

/// The worktree and branch a member's task starts with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::daemon) struct Workspace {
    /// The task.
    pub(in crate::daemon) task: TaskId,
    /// The agent that works in it.
    pub(in crate::daemon) agent: AgentName,
    /// Where the worktree is made.
    pub(in crate::daemon) path: AbsolutePath,
    /// The branch the worktree has out.
    pub(in crate::daemon) branch: BranchName,
}

/// A team as formed from a request, before anything is made.
#[derive(Debug, Clone)]
pub(in crate::daemon) struct Formed {
    /// The team: crew, task list and the first assignments.
    pub(in crate::daemon) team: Team,
    /// A workspace for every task of the crew, the coordinator's (or the solo agent's) first.
    pub(in crate::daemon) workspaces: Vec<Workspace>,
    /// The team on the bus; none for one agent alone.
    pub(in crate::daemon) bus: Option<NewTeam>,
    /// The branch the workspaces start from.
    pub(in crate::daemon) base: BranchName,
}

/// Why a request does not form a team.
#[derive(Debug, thiserror::Error)]
pub(in crate::daemon) enum FormError {
    /// The repository is not in the configuration.
    #[error("the repository {repo} is not in the configuration: add a [repos.{repo}] table to agents.toml")]
    UnknownRepo {
        /// The repository.
        repo: String,
    },
    /// Every item of the list is done already.
    #[error("every item of the task list is marked done: there is nothing to do")]
    NothingToDo,
    /// The agents cannot form a team.
    #[error(transparent)]
    Plan(#[from] TeamPlanError),
    /// An ID cannot be made.
    #[error("cannot make the ID of the team or of a task")]
    Id(#[from] GenerateIdError),
    /// The draws of a task ID gave an ID the team has already, every time.
    #[error("cannot draw a task ID that the team does not have already")]
    IdCollision,
    /// The clock gives a time that cannot be recorded.
    #[error("the clock gives a time that cannot be recorded")]
    Clock(#[from] InvalidTimestamp),
    /// The people do not make a crew.
    #[error("the people of the team do not make a crew")]
    Crew(#[from] CrewError),
    /// The first assignments cannot be decided.
    #[error("cannot decide the first assignments")]
    Assign(#[from] AssignmentError),
    /// An item that was given to a member is not in the team: a mistake of the daemon.
    #[error("item {item} is not in the team")]
    NoSuchItem {
        /// The item.
        item: ItemId,
    },
    /// A round of the new team cannot be opened.
    #[error("cannot open a round of the new team")]
    Round(#[from] OpenRoundError),
    /// An item cannot start.
    #[error("item {item} cannot start")]
    Advance {
        /// The item.
        item: ItemId,
        /// What the state said.
        source: AdvanceError,
    },
    /// A name of the team is not an agent name.
    #[error("the name of a member of the team is not an agent name")]
    Name(#[from] InvalidName),
    /// A branch name cannot be made.
    #[error("a branch name cannot be made")]
    Branch(#[from] InvalidBranchName),
    /// A path of the home cannot be used.
    #[error("the path {path:?} cannot be used")]
    Path {
        /// The path.
        path: PathBuf,
        /// What is wrong with it, if the text was refused.
        source: Option<InvalidValue>,
    },
    /// A member of the team is not in the configuration.
    #[error("{name} is not in the configuration")]
    NotConfigured {
        /// The name.
        name: String,
    },
}

impl FormError {
    /// How the daemon refuses the request: a team that is too crowded with investigators is `refused`, the rest of what
    /// the request names wrongly is `invalid`, and what the daemon itself got wrong is `internal`, with no detail: the
    /// detail is in its log.
    pub(in crate::daemon) fn failure(&self) -> Failure {
        let kind = match self {
            Self::Plan(TeamPlanError::TooManyInvestigators { .. } | TeamPlanError::TooFewInvestigators { .. }) => {
                FailureKind::Refused
            }
            Self::UnknownRepo { .. } | Self::NothingToDo | Self::Plan(_) => FailureKind::Invalid,
            Self::Id(_)
            | Self::IdCollision
            | Self::Clock(_)
            | Self::Crew(_)
            | Self::Assign(_)
            | Self::NoSuchItem { .. }
            | Self::Round(_)
            | Self::Advance { .. }
            | Self::Name(_)
            | Self::Branch(_)
            | Self::Path { .. }
            | Self::NotConfigured { .. } => {
                return Failure {
                    kind: FailureKind::Internal,
                    message: "the daemon could not form the team: see its log".to_owned(),
                };
            }
        };
        Failure {
            kind,
            message: self.to_string(),
        }
    }
}

/// Forms the team `request` asks for, in `home`, at the time `clock` shows.
///
/// # Errors
/// [`FormError`].
pub(in crate::daemon) fn form(
    request: &RunRequest,
    config: &Config,
    home: &Home,
    clock: &impl Clock,
    rng: &mut impl Rng,
) -> Result<Formed, FormError> {
    let repo = config.repo(&request.repo).ok_or_else(|| FormError::UnknownRepo {
        repo: request.repo.to_string(),
    })?;
    let list = request.task_list.list();
    if list.items().next().is_some() && list.items().all(Item::is_done) {
        return Err(FormError::NothingToDo);
    }
    let plan = plan_team(
        &TeamRequest {
            agents: &request.agents,
            flag_coordinator: request.coordinator.as_ref(),
            listed_coordinator: list.coordinator(),
        },
        config,
        rng,
    )?;
    let id = TeamId::generate(clock, rng)?;
    let created_at = Timestamp::now(clock)?;
    let crew = make_crew(plan, clock, rng)?;
    let mut team = Team::new(
        id,
        request.repo.clone(),
        created_at,
        request.merge,
        request.task_list.clone(),
        crew,
    );
    let given = assignable(&team)?;
    let mut rounds: Vec<RoundId> = Vec::new();
    for assigned in &given {
        // The item is given out together with its round: an item has one as soon as it leaves `pending`.
        let round = fresh_round(clock, rng, &rounds)?;
        rounds.push(round.clone());
        team.open_round(round, Some(assigned.item), created_at)?;
        let mut item = team
            .item_mut(assigned.item)
            .ok_or(FormError::NoSuchItem { item: assigned.item })?;
        start_item(&mut item, assigned).map_err(|source| FormError::Advance {
            item: assigned.item,
            source,
        })?;
    }
    if team.items().is_empty() {
        // A list without items is one piece of work, and one round, from the start of the team.
        team.open_round(fresh_round(clock, rng, &rounds)?, None, created_at)?;
    }
    let workspaces = workspaces(&team, &given, home)?;
    let bus = bus_team(&team, &workspaces, config)?;
    Ok(Formed {
        team,
        workspaces,
        bus,
        base: repo.base_branch.clone(),
    })
}

/// The crew of `plan`, with a task for each of its people.
fn make_crew(plan: TeamPlan, clock: &impl Clock, rng: &mut impl Rng) -> Result<Crew, FormError> {
    let mut used: Vec<TaskId> = Vec::new();
    let mut member = |participant: Participant| -> Result<Member, FormError> {
        let task = fresh_task(clock, rng, &used)?;
        used.push(task.clone());
        Ok(Member::new(task, participant))
    };
    match plan {
        TeamPlan::Solo { agent } => Ok(Crew::solo(member(agent)?)?),
        TeamPlan::Team(roster) => {
            let coordinator = member(roster.coordinator)?;
            let members = roster
                .members
                .into_iter()
                .map(&mut member)
                .collect::<Result<Vec<_>, _>>()?;
            let workers = roster
                .workers
                .into_iter()
                .map(&mut member)
                .collect::<Result<Vec<_>, _>>()?;
            Ok(Crew::team(TeamMembers {
                coordinator,
                members,
                workers,
            })?)
        }
    }
}

/// A task ID that is not in `used`.
fn fresh_task(clock: &impl Clock, rng: &mut impl Rng, used: &[TaskId]) -> Result<TaskId, FormError> {
    for _ in 0..ID_ATTEMPTS {
        let task = TaskId::generate(clock, rng)?;
        if !used.contains(&task) {
            return Ok(task);
        }
    }
    Err(FormError::IdCollision)
}

/// A round ID that is not in `used`.
fn fresh_round(clock: &impl Clock, rng: &mut impl Rng, used: &[RoundId]) -> Result<RoundId, FormError> {
    for _ in 0..ID_ATTEMPTS {
        let round = RoundId::generate(clock, rng)?;
        if !used.contains(&round) {
            return Ok(round);
        }
    }
    Err(FormError::IdCollision)
}

/// The workspace of every task of `team`: on the branch of its item when it has one, else on a branch of its task.
fn workspaces(team: &Team, given: &[Assigned], home: &Home) -> Result<Vec<Workspace>, FormError> {
    team.crew()
        .roster()
        .into_iter()
        .map(|(_, member)| {
            let agent = agent_name(member.agent())?;
            let branch = match given.iter().find(|assigned| assigned.task == *member.task()) {
                Some(assigned) => assigned.branch.clone(),
                None => task_branch(&agent, member.task())?,
            };
            let path = home.worktree_dir(team.repo(), member.task());
            let text = path.to_str().ok_or_else(|| FormError::Path {
                path: path.clone(),
                source: None,
            })?;
            let path = AbsolutePath::from_str(text).map_err(|source| FormError::Path {
                path: path.clone(),
                source: Some(source),
            })?;
            Ok(Workspace {
                task: member.task().clone(),
                agent,
                path,
                branch,
            })
        })
        .collect()
}

/// The team on the bus, for a crew that has a coordinator; none for one agent alone.
fn bus_team(team: &Team, workspaces: &[Workspace], config: &Config) -> Result<Option<NewTeam>, FormError> {
    let Some(coordinator) = team.crew().coordinator() else {
        return Ok(None);
    };
    let mut members = BTreeMap::new();
    for ((role, member), workspace) in team.crew().roster().into_iter().zip(workspaces) {
        members.insert(
            workspace.agent.clone(),
            NewMember {
                task: workspace.task.clone(),
                branch: workspace.branch.clone(),
                worktree: workspace.path.clone(),
                tier: if role == Role::Worker {
                    BusTier::Worker
                } else {
                    BusTier::Normal
                },
                // Every task starts at once and the bus puts a member to sleep when its run ends: a member made `sleeping`
                // before it has read the message of the team's creation would be woken by it at once.
                state: MemberState::Active,
                provider: Some(cli_of(config, member.agent())?.to_string()),
            },
        );
    }
    Ok(Some(NewTeam {
        id: team.id().clone(),
        repo: team.repo().clone(),
        lead: Some(agent_name(coordinator.agent())?),
        members,
    }))
}

/// The CLI the configuration gives `participant`.
fn cli_of(config: &Config, participant: &Participant) -> Result<CliKind, FormError> {
    let found = match participant {
        Participant::Normal(name) | Participant::Worker(name) => config.agent(name).map(|agent| agent.cli),
        Participant::Investigator(name) => config.investigator(name).map(|agent| agent.cli),
    };
    found.ok_or_else(|| FormError::NotConfigured {
        name: participant.name().to_owned(),
    })
}

#[cfg(test)]
mod tests;
