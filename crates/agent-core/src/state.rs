//! The persistent state of the harness: teams, their members and roles, the task list with the progress of each
//! item, runs, questions, pull requests, reviews and gate results (see `docs/state.md`).
//!
//! The daemon owns one [`State`], saves it as JSON and reads it back; the CLI and the dashboard see parts of it
//! through the control protocol ([`crate::protocol`]). Everything here is pure data with its rules: a value that
//! exists is a valid one, and a text that is not valid is refused when it is read.

mod base;
mod checked;
mod checks;
#[cfg(test)]
mod delivery_migration_tests;
mod findings;
#[cfg(test)]
pub(crate) mod fixtures;
mod gate;
mod holding;
#[cfg(test)]
mod holding_tests;
mod item;
#[cfg(test)]
mod item_parts_tests;
#[cfg(test)]
mod item_tests;
mod member;
mod migrate;
mod plan;
mod question;
mod recovery;
#[cfg(test)]
mod removal_tests;
pub use recovery::{
    Recovery, RecoveryDestination, RecoveryError, RecoveryInstallation, RecoveryPhase, RecoveryPreparation,
};
#[cfg(test)]
mod commit_cut_tests;
#[cfg(test)]
mod limit_tests;
mod renewal;
#[cfg(test)]
mod renewal_tests;
mod round;
mod run;
pub mod split;
mod strict;
mod team;
mod values;
#[cfg(test)]
mod waiting_tests;
mod wake_delivery;

pub use base::UnpushedBase;
pub use checked::CheckedMut;
pub use checks::{Asked, Basis, ChecksRecord, Covers, ItemChecks, Sent};
pub use findings::{
    Decision, Disputes, Explanation, ExplanationStatus, FindingNumbersExhausted, FindingStatus, Findings,
    InvalidFindingNumbers, OpinionAttempt, OpinionFailure, OpinionRequest, OpinionStage, OpinionStateError, Raise,
    RaisedFinding, Reconciled, RecordedFinding, SecondOpinion, Verification, VerifierAnswers, reconcile,
};
pub use gate::{GateOutcome, GateRecord, GateResult, GateTree};
pub use holding::{HandOverError, Holding};
pub use item::{
    AdvanceError, AskReviewsError, Completion, FixPass, FixingWithoutFixPass, InvalidAdvance, InvalidReviews, ItemMut,
    ItemPhase, ItemProgress, MergeConflict, NotWorkedOn, PhaseKind, PullRequest, Rejection, RejectionError,
    ReplaceReviewError, Review, ReviewOutcome, SubmitReviewError,
};
pub use member::{Member, MemberError, MemberMut, Participant, QuestionError};
pub use plan::{PlanFault, PlanKeeping, PlanReason, PlanStage, RunningPlanCheck, StepMismatch};
pub use question::{AnswerBeforeQuestion, AnsweredVia, Question, QuestionState};
pub use renewal::{InvalidRenewalLimit, RenewTaskError, RenewalError, RenewalLimit, Renewals};
pub use round::{ClosedBeforeOpened, Round, RoundEnd, RoundLife};
pub use run::{Run, RunEnd, RunError, RunState};
pub use team::{
    ActivateSplitError, Activity, ActivityEnd, ActivityOutcome, ActivityRequest, AdvanceItemError, BeginRunError,
    CloseRoundError, CompleteDeliveryError, Crew, CrewError, CrewMut, DeliverRoundError, EndDeliveryError, IdleWatch,
    MergeMode, NewRun, OpenRoundError, RecordSplitError, ReviewerProblem, RoundsError, TaskListText, Team, TeamError,
    TeamMembers, TeamMut,
};
pub use values::{CommitId, InvalidCommitId, InvalidTimestamp, PrNumber, SessionId, Timestamp};
pub use wake_delivery::WakeDelivery;

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::ids::{AgentName, ItemId, RoundId, TaskId, TeamId};
use crate::kinds::Role;

/// A `team-replace` call the daemon must complete before the task's next run: the bus must be told the member changed
/// so it can route messages and deliver reminders to the new agent.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Deserialize, Serialize)]
pub struct PendingTeamReplace {
    /// The team whose member is being replaced.
    pub team: TeamId,
    /// The agent name being replaced.
    pub from: AgentName,
    /// The replacement agent name.
    pub to: AgentName,
}

/// The schema version this build writes, with investigator activities reconstructed from versions 1 through 10.
pub const SCHEMA_VERSION: u32 = 15;

/// Largest state text accepted, in bytes: a bound on untrusted input, far above any real state.
pub const MAX_STATE_BYTES: usize = 64 << 20;

/// The schema version of a state file: written as the number [`SCHEMA_VERSION`], and only that number is read. A state
/// of version 1 is read by [`State::from_json`] through its migration, not as this type.
///
/// A type with one value, so that a state of another version cannot be mistaken for this one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SchemaVersion;

version_type!(SchemaVersion, SCHEMA_VERSION, UnsupportedSchema);

/// A state of a schema version this build does not read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error(
    "the state has schema version {found}; this build reads versions 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14 and {SCHEMA_VERSION}"
)]
pub struct UnsupportedSchema {
    /// The version the state declares.
    pub found: u64,
}

/// The state of the harness: every team and, in them, every task.
///
/// Task IDs are unique in the whole state, team IDs too; [`State::insert_team`] and [`State::add_reviewer`] keep
/// them so, and reading a state checks it.
#[derive(Debug, Clone, Eq, Deserialize)]
#[serde(try_from = "StateWire")]
pub struct State {
    /// Always [`SCHEMA_VERSION`]; first in the file, so that a reader sees it before anything else.
    schema_version: SchemaVersion,
    /// The teams, oldest first.
    teams: Vec<Team>,
    /// Delivery budgets by task; an absent entry means no message has a recorded wake-up budget.
    wake_deliveries: BTreeMap<TaskId, WakeDelivery>,
    /// The queued tasks in the order they entered the queue, oldest first.
    queued: Vec<TaskId>,
    /// Durable restart reservations, retained after source and replacement removal.
    recoveries: Vec<Recovery>,
    /// How many refusals by a provider in a row each task had; a task without an entry has none.
    limit_refusals: BTreeMap<TaskId, u32>,
    /// Until when an agent refused by its provider is kept out of new tasks, reviews and takeovers; an agent without an
    /// entry is not refused.
    refused_agents: BTreeMap<crate::ids::AgentName, Timestamp>,
    /// The runs of each task that the daemon cut for an order to commit and resumed in their session: the clock of a
    /// round does not start again after them, as it does after a stop of the human. A task without an entry had none.
    commit_cuts: BTreeMap<TaskId, BTreeSet<crate::ids::RunId>>,
    /// Team-replace calls the daemon has not yet completed: their task's next run waits until the bus knows
    /// the new member. A state written before 1.2 loads without this field.
    pub(crate) pending_team_replaces: BTreeSet<PendingTeamReplace>,
}

/// How a [`State`] is written as JSON: `queued` is omitted whenever it matches the natural team-creation order.
#[derive(Serialize)]
struct StateOut<'a> {
    /// See [`State`].
    schema_version: SchemaVersion,
    /// See [`State`].
    teams: &'a [Team],
    /// See [`State`].
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    wake_deliveries: &'a BTreeMap<TaskId, WakeDelivery>,
    /// See [`State`].
    #[serde(skip_serializing_if = "Vec::is_empty")]
    queued: Vec<TaskId>,
    /// See [`State`].
    #[serde(skip_serializing_if = "<[_]>::is_empty")]
    recoveries: &'a [Recovery],
    /// See [`State`]; omitted when no task had a refusal.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    limit_refusals: &'a BTreeMap<TaskId, u32>,
    /// See [`State`]; omitted when no agent is refused.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    refused_agents: &'a BTreeMap<crate::ids::AgentName, Timestamp>,
    /// See [`State`]; omitted when no run was cut for an order to commit.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    commit_cuts: &'a BTreeMap<TaskId, BTreeSet<crate::ids::RunId>>,
    /// See [`State`]; omitted when no team-replace call is pending.
    #[serde(skip_serializing_if = "BTreeSet::is_empty")]
    pending_team_replaces: &'a BTreeSet<PendingTeamReplace>,
}

impl Serialize for State {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let effective = self.effective_queued();
        let queued = if effective == self.natural_queued() {
            Vec::new()
        } else {
            effective
        };
        StateOut {
            schema_version: self.schema_version,
            teams: &self.teams,
            wake_deliveries: &self.wake_deliveries,
            recoveries: &self.recoveries,
            queued,
            limit_refusals: &self.limit_refusals,
            refused_agents: &self.refused_agents,
            commit_cuts: &self.commit_cuts,
            pending_team_replaces: &self.pending_team_replaces,
        }
        .serialize(serializer)
    }
}

impl PartialEq for State {
    fn eq(&self, other: &Self) -> bool {
        self.schema_version == other.schema_version
            && self.teams == other.teams
            && self.recoveries == other.recoveries
            && self.wake_deliveries == other.wake_deliveries
            && self.limit_refusals == other.limit_refusals
            && self.refused_agents == other.refused_agents
            && self.commit_cuts == other.commit_cuts
            && self.pending_team_replaces == other.pending_team_replaces
            && self.effective_queued() == other.effective_queued()
    }
}

/// What a [`State`] is written as, before its checks.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct StateWire {
    /// See [`State`].
    schema_version: SchemaVersion,
    /// See [`State`].
    teams: Vec<Team>,
    /// Optional in state written before wake-up delivery budgets were persisted.
    #[serde(default)]
    wake_deliveries: BTreeMap<TaskId, WakeDelivery>,
    /// Optional in state written before queue-entry order was persisted.
    #[serde(default)]
    queued: Vec<TaskId>,
    /// Optional in state written before recovery reservations were persisted.
    #[serde(default, deserialize_with = "recovery::read_ledger")]
    recoveries: Vec<Recovery>,
    /// Optional in state written before provider refusals were persisted.
    #[serde(default)]
    limit_refusals: BTreeMap<TaskId, u32>,
    /// Optional in state written before provider refusals were persisted.
    #[serde(default)]
    refused_agents: BTreeMap<crate::ids::AgentName, Timestamp>,
    /// Optional in state written before commit cuts were persisted.
    #[serde(default)]
    commit_cuts: BTreeMap<TaskId, BTreeSet<crate::ids::RunId>>,
    /// Optional in state written before team-replace calls were persisted.
    #[serde(default)]
    pending_team_replaces: BTreeSet<PendingTeamReplace>,
}

/// Only the schema version of a state file, read before the rest.
#[derive(Deserialize)]
struct SchemaProbe {
    /// The version the file declares.
    schema_version: u64,
}

/// Why a state could not be read.
#[derive(Debug, thiserror::Error)]
pub enum StateError {
    /// The text is longer than [`MAX_STATE_BYTES`].
    #[error("the state has {len} bytes; the limit is {max}")]
    TooLarge {
        /// The text's length, in bytes.
        len: usize,
        /// The limit.
        max: usize,
    },
    /// The state declares a schema version this build does not read.
    #[error(transparent)]
    UnsupportedSchema(#[from] UnsupportedSchema),
    /// The text is not a state: not JSON, a field missing or unknown, or a value that is not valid.
    #[error("the state is not valid")]
    Malformed {
        /// The parser's reason, with the line and column.
        source: serde_json::Error,
    },
    /// The parts of the state do not fit together.
    #[error(transparent)]
    Invalid(#[from] InvalidState),
}

/// Teams or tasks of a state that clash.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum InvalidState {
    /// Two teams have one ID.
    #[error("the team {team} appears twice")]
    DuplicateTeam {
        /// The repeated ID.
        team: TeamId,
    },
    /// Two tasks have one ID.
    #[error("the task {task} is in the team {first} and in the team {second}")]
    DuplicateTask {
        /// The repeated ID.
        task: TaskId,
        /// The team that has it first.
        first: TeamId,
        /// The team that has it again; the same team when it has it twice.
        second: TeamId,
    },
    /// Two rounds of different teams have one ID.
    #[error("the team {team} has a round {round} that another team has too")]
    DuplicateRound {
        /// The repeated ID.
        round: RoundId,
        /// The team that has it again, the later one.
        team: TeamId,
    },
    /// A state of version 1 gave every ID that can be derived for a round to other rounds.
    #[error("no free ID for the round of item {item} of the team {team}")]
    NoFreeRoundId {
        /// The team.
        team: TeamId,
        /// The item.
        item: ItemId,
    },
}

/// Why the start of a task's busy wait could not be recorded.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum StartWaitingError {
    /// The requested task is not in the state.
    #[error("cannot start waiting: task {task} is not in the state")]
    NoTask {
        /// The task whose busy wait was requested.
        task: TaskId,
    },
}

/// Why a round could not be opened in a state.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum StateOpenRoundError {
    /// The state has no such team.
    #[error("the state has no team {team}")]
    UnknownTeam {
        /// The team asked for.
        team: TeamId,
    },
    /// Another team has a round with this ID.
    #[error("the round {round} is a round of the team {team}")]
    DuplicateRound {
        /// The repeated ID.
        round: RoundId,
        /// The team that has it.
        team: TeamId,
    },
    /// The round does not fit the team.
    #[error(transparent)]
    Team(#[from] OpenRoundError),
}

/// Why a reviewer could not be added to a team.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AddReviewerError {
    /// The state has no such team.
    #[error("the state has no team {team}")]
    UnknownTeam {
        /// The team asked for.
        team: TeamId,
    },
    /// The reviewer does not fit the team.
    #[error(transparent)]
    Problem(#[from] ReviewerProblem),
}

/// The state could not be written as text.
#[derive(Debug, thiserror::Error)]
#[error("the state cannot be written as JSON")]
pub struct RenderError {
    /// What the serializer said.
    #[source]
    source: serde_json::Error,
}

/// A task found in a state: its team, its role and its member.
#[derive(Debug, Clone, Copy)]
pub struct TaskRef<'a> {
    /// The team the task is in.
    pub team: &'a Team,
    /// The role of the member in it.
    pub role: Role,
    /// The member.
    pub member: &'a Member,
}

impl State {
    /// How many refusals by the provider in a row the task had; `0` when none.
    #[must_use]
    pub fn limit_refusals(&self, task: &TaskId) -> u32 {
        self.limit_refusals.get(task).copied().unwrap_or(0)
    }

    /// Records how many refusals in a row a task had; `0` forgets them, as a run that ends in any other way does.
    pub fn record_limit_refusals(&mut self, task: TaskId, refusals: u32) {
        if refusals == 0 {
            self.limit_refusals.remove(&task);
        } else {
            self.limit_refusals.insert(task, refusals);
        }
    }

    /// Whether the daemon cut `run` of `task` for an order to commit and resumed it in its session: such a cut does not
    /// start the clock of a round again, as a stop of the human does.
    #[must_use]
    pub fn commit_cut(&self, task: &TaskId, run: &crate::ids::RunId) -> bool {
        self.commit_cuts.get(task).is_some_and(|runs| runs.contains(run))
    }

    /// Records that the daemon cut `run` of `task` for an order to commit.
    pub fn record_commit_cut(&mut self, task: TaskId, run: crate::ids::RunId) {
        self.commit_cuts.entry(task).or_default().insert(run);
    }

    /// Until when the agent is refused by its provider, `None` when it is not refused at all.
    #[must_use]
    pub fn refused_until(&self, agent: &crate::ids::AgentName) -> Option<Timestamp> {
        self.refused_agents.get(agent).copied()
    }

    /// Whether the agent is refused by its provider at `now`: refused before its return time, free from it on.
    #[must_use]
    pub fn refused_at(&self, agent: &crate::ids::AgentName, now: Timestamp) -> bool {
        self.refused_until(agent).is_some_and(|until| now < until)
    }

    /// The agents refused by their provider at `now`, in name order.
    #[must_use]
    pub fn refused_agents_at(&self, now: Timestamp) -> Vec<crate::ids::AgentName> {
        self.refused_agents
            .keys()
            .filter(|agent| self.refused_at(agent, now))
            .cloned()
            .collect()
    }

    /// The agents refused by their provider at `now` with the time each returns, in name order.
    #[must_use]
    pub fn refusals_at(&self, now: Timestamp) -> Vec<(crate::ids::AgentName, Timestamp)> {
        self.refused_agents
            .iter()
            .filter(|(_, until)| now < **until)
            .map(|(agent, until)| (agent.clone(), *until))
            .collect()
    }

    /// Records until when an agent is refused by its provider, or forgets the refusal with `None`.
    pub fn record_refused_until(&mut self, agent: crate::ids::AgentName, until: Option<Timestamp>) {
        match until {
            Some(until) => {
                self.refused_agents.insert(agent, until);
            }
            None => {
                self.refused_agents.remove(&agent);
            }
        }
    }

    /// Keeps an agent refused until `until`, unless it is already refused until a later time: a refusal that
    /// ends sooner never brings back an agent whose longer pause is still running.
    pub fn keep_refused_until(&mut self, agent: crate::ids::AgentName, until: Timestamp) {
        let later = self
            .refused_agents
            .get(&agent)
            .map_or(until, |known| (*known).max(until));
        self.refused_agents.insert(agent, later);
    }

    /// The wake delivery budget of a task, including historical tasks no longer in the crew.
    #[must_use]
    pub fn wake_delivery(&self, task: &TaskId) -> Option<WakeDelivery> {
        self.wake_deliveries.get(task).copied()
    }

    /// Records a delivery budget, or forgets it when the task has read what woke it.
    pub fn record_wake_delivery(&mut self, task: TaskId, delivery: Option<WakeDelivery>) {
        match delivery {
            Some(delivery) => {
                self.wake_deliveries.insert(task, delivery);
            }
            None => {
                self.wake_deliveries.remove(&task);
            }
        }
    }

    /// A state without teams.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            schema_version: SchemaVersion,
            teams: Vec::new(),
            wake_deliveries: BTreeMap::new(),
            queued: Vec::new(),
            recoveries: Vec::new(),
            limit_refusals: BTreeMap::new(),
            refused_agents: BTreeMap::new(),
            commit_cuts: BTreeMap::new(),
            pending_team_replaces: BTreeSet::new(),
        }
    }

    /// Retains investigator assignments and terminal outcomes before a daemon transition is saved.
    pub fn record_activities(&mut self, at: Timestamp) {
        for team in &mut self.teams {
            team.record_activities(at);
        }
        self.sync_queued();
    }

    /// The tasks currently in [`TaskStatus::Queued`](crate::status::TaskStatus::Queued) in team-creation and roster order.
    fn natural_queued(&self) -> Vec<TaskId> {
        self.teams
            .iter()
            .flat_map(team_members)
            .filter(|member| member.status() == crate::status::TaskStatus::Queued)
            .map(|member| member.task().clone())
            .collect()
    }

    /// The tasks currently in [`TaskStatus::Queued`](crate::status::TaskStatus::Queued) in queue-entry order, oldest first.
    fn effective_queued(&self) -> Vec<TaskId> {
        let natural = self.natural_queued();
        let natural_set: HashSet<&TaskId> = natural.iter().collect();
        let mut seen: HashSet<TaskId> = HashSet::with_capacity(natural.len());
        let mut order = Vec::with_capacity(natural.len());
        for task in &self.queued {
            if natural_set.contains(task) && seen.insert(task.clone()) {
                order.push(task.clone());
            }
        }
        for task in natural {
            if seen.insert(task.clone()) {
                order.push(task);
            }
        }
        order
    }

    /// Synchronizes the queue-entry order with the members currently in [`TaskStatus::Queued`](crate::status::TaskStatus::Queued).
    pub fn sync_queued(&mut self) {
        self.queued = self.effective_queued();
    }

    /// The task IDs currently in [`TaskStatus::Queued`](crate::status::TaskStatus::Queued) in queue-entry order.
    #[must_use]
    pub fn queued_order(&self) -> Vec<TaskId> {
        self.effective_queued()
    }

    /// Places tasks that were interrupted from [`TaskStatus::Running`](crate::status::TaskStatus::Running) into
    /// [`TaskStatus::Queued`](crate::status::TaskStatus::Queued) (in team order) ahead of the tasks in `waiting` that
    /// were already queued before the interruption (in their `waiting` order).
    pub fn place_waiting_after_interrupted(&mut self, waiting: &[TaskId]) {
        let natural = self.natural_queued();
        let natural_set: HashSet<&TaskId> = natural.iter().collect();
        let waiting_set: HashSet<&TaskId> = waiting.iter().collect();
        let mut seen: HashSet<TaskId> = HashSet::with_capacity(natural.len());
        let mut order = Vec::with_capacity(natural.len());
        for task in &natural {
            if !waiting_set.contains(task) && seen.insert(task.clone()) {
                order.push(task.clone());
            }
        }
        for task in waiting {
            if natural_set.contains(task) && seen.insert(task.clone()) {
                order.push(task.clone());
            }
        }
        self.queued = order;
    }

    /// The queued tasks in the order they entered the queue, oldest first.
    pub fn queued_tasks(&mut self) -> Vec<crate::decide::slot::Queued> {
        self.sync_queued();
        let mut by_task: HashMap<TaskId, crate::decide::slot::Queued> = self
            .teams
            .iter()
            .flat_map(|team| {
                let team_id = team.id();
                team_members(team)
                    .filter(|member| member.status() == crate::status::TaskStatus::Queued)
                    .map(move |member| {
                        let task = member.task().clone();
                        (
                            task.clone(),
                            crate::decide::slot::Queued {
                                task,
                                team: team_id.clone(),
                                agent: member.agent().agent_name(),
                                tier: member.agent().tier(),
                            },
                        )
                    })
            })
            .collect();
        self.queued.iter().filter_map(|task| by_task.remove(task)).collect()
    }

    /// The team-replace calls the daemon has not yet completed.
    pub fn pending_team_replaces(&self) -> &BTreeSet<PendingTeamReplace> {
        &self.pending_team_replaces
    }

    /// Records a team-replace call the daemon must complete before the task's next run.
    pub fn record_pending_team_replace(&mut self, team: TeamId, from: AgentName, to: AgentName) {
        self.pending_team_replaces.insert(PendingTeamReplace { team, from, to });
    }

    /// Removes a team-replace call after the bus was notified.
    pub fn remove_pending_team_replace(&mut self, team: &TeamId, from: &AgentName, to: &AgentName) -> bool {
        self.pending_team_replaces.remove(&PendingTeamReplace {
            team: team.clone(),
            from: from.clone(),
            to: to.clone(),
        })
    }

    /// The teams, oldest first.
    #[must_use]
    pub fn teams(&self) -> &[Team] {
        &self.teams
    }

    /// The team `id`.
    #[must_use]
    pub fn team(&self, id: &TeamId) -> Option<&Team> {
        self.teams.iter().find(|team| team.id() == id)
    }

    /// The team `id`, to change it through the checked operations of [`TeamMut`] only.
    ///
    /// The handle does not give out the team as a value to overwrite, so no change can bring in a second team with
    /// one ID or a member that the team's checks did not see. Each of these is refused by the compiler:
    #[doc = include_str!("../tests/docs/team-mut.md")]
    pub fn team_mut(&mut self, id: &TeamId) -> Option<TeamMut<'_>> {
        self.teams.iter_mut().find(|team| team.id() == id).map(TeamMut::new)
    }

    /// The task `task`, with its team and its role.
    #[must_use]
    pub fn find_task(&self, task: &TaskId) -> Option<TaskRef<'_>> {
        self.teams
            .iter()
            .find_map(|team| team.find(task).map(|(role, member)| TaskRef { team, role, member }))
    }

    /// The member of task `task`, to change its task through the checked operations of [`MemberMut`] only; the
    /// member cannot be overwritten with another one:
    #[doc = include_str!("../tests/docs/find-task-mut.md")]
    pub fn find_task_mut(&mut self, task: &TaskId) -> Option<(Role, MemberMut<'_>)> {
        self.teams.iter_mut().find_map(|team| team.find_mut(task))
    }

    /// Starts `task` anew in a new session after `trouble`, at `at`: the runs after now are the new session's, and no
    /// session before it is resumed. Only a task of a solo agent, a coordinator or a member is started anew — and a
    /// worker's or a reviewer's task is, exactly when `trouble` is the retry of a run that reported `STATUS: FAILED`
    /// without a tool call — and only after a run of it. Returns whether the task was started anew.
    ///
    /// # Errors
    /// [`RenewTaskError`] when the task is not in the state, is of a worker or a reviewer for any other trouble, or the
    /// renewal does not fit its runs.
    pub fn renew_task(
        &mut self,
        task: &TaskId,
        trouble: crate::decide::ladder::Trouble,
        at: Timestamp,
    ) -> Result<bool, RenewTaskError> {
        let (role, mut member) = self
            .find_task_mut(task)
            .ok_or_else(|| RenewTaskError::NoTask { task: task.clone() })?;
        // The ladder never starts a worker or a reviewer anew; the retry of a failed run without a tool call applies to
        // every role, so it is the one trouble their task may be renewed for.
        if matches!(role, Role::Worker | Role::Reviewer)
            && !matches!(trouble, crate::decide::ladder::Trouble::NoToolCall)
        {
            return Err(RenewTaskError::Role {
                task: task.clone(),
                role,
            });
        }
        member.renew(trouble, at).map_err(|source| RenewTaskError::Renewal {
            task: task.clone(),
            source,
        })
    }

    /// Starts `task` anew because its session outgrew its agent's bound at `tokens`, at `at`, and keeps that size with the
    /// renewal, so that a restart before the new session's first run still knows it.
    ///
    /// # Errors
    /// [`RenewTaskError`] when the task is not in the state or the session does not fit its runs.
    pub fn begin_context_session_with_size(
        &mut self,
        task: &TaskId,
        at: Timestamp,
        tokens: u64,
    ) -> Result<bool, RenewTaskError> {
        let begun = self.begin_context_session(task, at)?;
        if let Some((_, mut member)) = self.find_task_mut(task).filter(|_| begun) {
            member.keep_context_tokens(tokens);
        }
        Ok(begun)
    }

    /// Starts `task` anew in a new session because its session outgrew its agent's bound, at `at`: the runs after now
    /// are the new session's, and no session before it is resumed. Every role is started anew for it, and no renewal is
    /// counted. Returns whether the task was started anew: `false` for a task with no run.
    ///
    /// # Errors
    /// [`RenewTaskError`] when the task is not in the state or the session does not fit its runs.
    pub fn begin_context_session(&mut self, task: &TaskId, at: Timestamp) -> Result<bool, RenewTaskError> {
        let (_, mut member) = self
            .find_task_mut(task)
            .ok_or_else(|| RenewTaskError::NoTask { task: task.clone() })?;
        member
            .begin_holder_session(crate::decide::ladder::Trouble::ContextBound, at)
            .map_err(|source| RenewTaskError::Renewal {
                task: task.clone(),
                source,
            })
    }

    /// Begins the session of the agent that holds `task` now, after a hand-over that followed `trouble`, at `at`: no
    /// session of the agent before it is resumed, its bad runs are not counted against the new one, and the next run is
    /// briefed. Unlike [`State::renew_task`] it counts no new session of the task's agent. Returns whether a session was
    /// begun: `false` for a task with no run.
    ///
    /// # Errors
    /// [`RenewTaskError`] when the task is not in the state, is of a worker or a reviewer, or the session does not fit
    /// its runs.
    pub fn begin_holder_session(
        &mut self,
        task: &TaskId,
        trouble: crate::decide::ladder::Trouble,
        at: Timestamp,
    ) -> Result<bool, RenewTaskError> {
        let (role, mut member) = self
            .find_task_mut(task)
            .ok_or_else(|| RenewTaskError::NoTask { task: task.clone() })?;
        match role {
            Role::Solo | Role::Coordinator | Role::Member => {}
            Role::Worker | Role::Reviewer => {
                return Err(RenewTaskError::Role {
                    task: task.clone(),
                    role,
                });
            }
        }
        member
            .begin_holder_session(trouble, at)
            .map_err(|source| RenewTaskError::Renewal {
                task: task.clone(),
                source,
            })
    }

    /// Records that the human is asked, in questions `question`, what becomes of `task`, whose runs went bad with
    /// `trouble` after the last step of the ladder, at `at`: a new session begins for whatever the answer is, no new
    /// session of the agent is counted, and the task is asked so only once. Returns whether a session was begun: `false`
    /// for a task with no run.
    ///
    /// # Errors
    /// [`RenewTaskError`] when the task is not in the state, is of a worker or a reviewer, or the session does not fit
    /// its runs.
    pub fn ask_after_last_step(
        &mut self,
        task: &TaskId,
        trouble: crate::decide::ladder::Trouble,
        question: std::num::NonZeroU32,
        at: Timestamp,
    ) -> Result<bool, RenewTaskError> {
        let (role, mut member) = self
            .find_task_mut(task)
            .ok_or_else(|| RenewTaskError::NoTask { task: task.clone() })?;
        match role {
            Role::Solo | Role::Coordinator | Role::Member => {}
            Role::Worker | Role::Reviewer => {
                return Err(RenewTaskError::Role {
                    task: task.clone(),
                    role,
                });
            }
        }
        member
            .ask_after_last_step(trouble, question, at)
            .map_err(|source| RenewTaskError::Renewal {
                task: task.clone(),
                source,
            })
    }

    /// Records that the work of the member's task `task`, whose runs went bad with `trouble`, is owed to its team's lead
    /// from `at`: the task stops, and the lead is to be told until [`State::lead_told`]. A session boundary is marked as
    /// for a hand-over. Only a member's task owes its work to a lead.
    ///
    /// # Errors
    /// [`RenewTaskError`] when the task is not in the state, is not a member's, or the boundary does not fit its runs.
    pub fn owe_lead(
        &mut self,
        task: &TaskId,
        trouble: crate::decide::ladder::Trouble,
        at: Timestamp,
    ) -> Result<(), RenewTaskError> {
        let (role, mut member) = self
            .find_task_mut(task)
            .ok_or_else(|| RenewTaskError::NoTask { task: task.clone() })?;
        match role {
            Role::Member => {}
            Role::Solo | Role::Coordinator | Role::Worker | Role::Reviewer => {
                return Err(RenewTaskError::Role {
                    task: task.clone(),
                    role,
                });
            }
        }
        member
            .owe_lead(trouble, at)
            .map(|_| ())
            .map_err(|source| RenewTaskError::Renewal {
                task: task.clone(),
                source,
            })
    }

    /// Records that the lead of `task` was told that the member's work is its own: nothing is owed any more.
    ///
    /// # Errors
    /// [`RenewTaskError::NoTask`] when the task is not in the state.
    pub fn lead_told(&mut self, task: &TaskId) -> Result<(), RenewTaskError> {
        let (_, mut member) = self
            .find_task_mut(task)
            .ok_or_else(|| RenewTaskError::NoTask { task: task.clone() })?;
        member.lead_told();
        Ok(())
    }

    /// Records that the team of `task` is still owed notice of its busy hand-over.
    ///
    /// # Errors
    /// [`RenewTaskError::NoTask`] when the task is not in the state.
    pub fn owe_busy_notice(&mut self, task: &TaskId) -> Result<(), RenewTaskError> {
        let (role, mut member) = self
            .find_task_mut(task)
            .ok_or_else(|| RenewTaskError::NoTask { task: task.clone() })?;
        match role {
            Role::Solo | Role::Coordinator | Role::Member => member.owe_busy_notice(true),
            Role::Worker | Role::Reviewer => {
                return Err(RenewTaskError::Role {
                    task: task.clone(),
                    role,
                });
            }
        }
        Ok(())
    }

    /// Records that the team took the busy hand-over notice of `task` and nothing is owed any more.
    ///
    /// # Errors
    /// [`RenewTaskError::NoTask`] when the task is not in the state.
    pub fn busy_notice_told(&mut self, task: &TaskId) -> Result<(), RenewTaskError> {
        let (_, mut member) = self
            .find_task_mut(task)
            .ok_or_else(|| RenewTaskError::NoTask { task: task.clone() })?;
        member.owe_busy_notice(false);
        Ok(())
    }

    /// Records that `task` waits for its busy agent from `at`, and returns the moment the wait began: an earlier one is kept.
    ///
    /// # Errors
    /// [`StartWaitingError::NoTask`] when the task is not in the state.
    pub fn start_waiting(&mut self, task: &TaskId, at: Timestamp) -> Result<Timestamp, StartWaitingError> {
        let (_, mut member) = self
            .find_task_mut(task)
            .ok_or_else(|| StartWaitingError::NoTask { task: task.clone() })?;
        Ok(member.start_waiting(at))
    }

    /// Gives `task` to `holder` at `at`, keeping the agent that held it among its past holders: only the task of a solo
    /// agent, a coordinator or a member, not in the middle of a run, and only to an agent that is not in its team, so
    /// that the state it leaves loads again.
    ///
    /// # Errors
    /// [`HandOverError`] when the task is not in the state, is of a worker or a reviewer, is held by `holder` already,
    /// `holder` is of a lower tier than the agent that holds it or is in its team, or a run of it is going.
    pub fn hand_over_task(&mut self, task: &TaskId, holder: Participant, at: Timestamp) -> Result<(), HandOverError> {
        let no_task = || HandOverError::NoTask { task: task.clone() };
        let team = self
            .teams
            .iter_mut()
            .find(|team| team.find(task).is_some())
            .ok_or_else(no_task)?;
        let (role, held_by) = team
            .find(task)
            .map(|(role, member)| (role, member.agent().clone()))
            .ok_or_else(no_task)?;
        match role {
            Role::Solo | Role::Coordinator | Role::Member => {}
            Role::Worker | Role::Reviewer => {
                return Err(HandOverError::Role {
                    task: task.clone(),
                    role,
                });
            }
        }
        if held_by == holder {
            return Err(HandOverError::SameHolder { task: task.clone() });
        }
        // A task is never given to a lower tier; for a solo agent's, a coordinator's or a member's task, the tiers
        // left are the ones the crew allows it.
        if crate::decide::replacement::rank(holder.tier()) < crate::decide::replacement::rank(held_by.tier()) {
            return Err(HandOverError::LowerTier {
                task: task.clone(),
                holder: holder.tier(),
                held: held_by.tier(),
            });
        }
        let in_team = team
            .crew()
            .roster()
            .into_iter()
            .map(|(_, member)| member)
            .chain(team.reviewers())
            .any(|member| member.agent().name() == holder.name());
        if in_team {
            return Err(HandOverError::InTeam {
                task: task.clone(),
                agent: holder.name().to_owned(),
            });
        }
        let (_, mut member) = team.find_mut(task).ok_or_else(no_task)?;
        if member.current_run().is_some() {
            return Err(HandOverError::InRun { task: task.clone() });
        }
        member.held_task(holder, at);
        Ok(())
    }

    /// The round `round` with its team.
    #[must_use]
    pub fn find_round(&self, round: &RoundId) -> Option<(&Team, &Round)> {
        self.teams
            .iter()
            .find_map(|team| team.round(round).map(|found| (team, found)))
    }

    /// Opens the round `id` of team `team`, for `item` — or for the whole work of a team whose task list has no items —
    /// at `at`.
    ///
    /// # Errors
    /// [`StateOpenRoundError`]: the state has no such team, another team has a round with this ID, or the round does
    /// not fit the team ([`Team::open_round`]).
    pub fn open_round(
        &mut self,
        team: &TeamId,
        id: RoundId,
        item: Option<ItemId>,
        at: Timestamp,
    ) -> Result<(), StateOpenRoundError> {
        if let Some((owner, _)) = self.find_round(&id) {
            return Err(StateOpenRoundError::DuplicateRound {
                round: id,
                team: owner.id().clone(),
            });
        }
        let found = self
            .teams
            .iter_mut()
            .find(|found| found.id() == team)
            .ok_or_else(|| StateOpenRoundError::UnknownTeam { team: team.clone() })?;
        found.open_round(id, item, at)?;
        Ok(())
    }

    /// Closes the round `round` at `at`, ended as `how`.
    ///
    /// # Errors
    /// [`CloseRoundError`], as for [`Team::close_round`]; [`CloseRoundError::NoSuchRound`] when no team has the round.
    pub fn close_round(&mut self, round: &RoundId, at: Timestamp, how: RoundEnd) -> Result<(), CloseRoundError> {
        self.teams
            .iter_mut()
            .find(|team| team.round(round).is_some())
            .ok_or_else(|| CloseRoundError::NoSuchRound { round: round.clone() })?
            .close_round(round, at, how)
    }

    /// Records that a run of task `task` has started, working for the rounds of `run`.
    ///
    /// # Errors
    /// [`BeginRunError`], as for [`Team::begin_run`]; [`BeginRunError::UnknownTask`] when no team has the task. The
    /// rounds must be the task's own team's: another team's are unknown to it.
    pub fn begin_run(&mut self, task: &TaskId, run: NewRun) -> Result<(), BeginRunError> {
        self.teams
            .iter_mut()
            .find(|team| team.find(task).is_some())
            .ok_or_else(|| BeginRunError::UnknownTask { task: task.clone() })?
            .begin_run(task, run)
    }

    /// Removes the team `id` and the records the state keeps for its tasks, and returns it; `None` when there is no
    /// such team, and then nothing changes.
    pub fn remove_team(&mut self, id: &TeamId) -> Option<Team> {
        let index = self.teams.iter().position(|team| team.id() == id)?;
        let team = self.teams.remove(index);
        let tasks: HashSet<TaskId> = team_members(&team).map(|member| member.task().clone()).collect();
        self.wake_deliveries.retain(|task, _| !tasks.contains(task));
        self.limit_refusals.retain(|task, _| !tasks.contains(task));
        self.commit_cuts.retain(|task, _| !tasks.contains(task));
        self.queued.retain(|task| !tasks.contains(task));
        Some(team)
    }

    /// Adds a team.
    ///
    /// # Errors
    /// [`InvalidState`] when the state has a team with this ID, a task with one of the team's task IDs or a round with
    /// one of its round IDs.
    pub fn insert_team(&mut self, team: Team) -> Result<(), InvalidState> {
        if self.team(team.id()).is_some() {
            return Err(InvalidState::DuplicateTeam {
                team: team.id().clone(),
            });
        }
        for member in team_members(&team) {
            if let Some(owner) = self.find_task(member.task()) {
                return Err(InvalidState::DuplicateTask {
                    task: member.task().clone(),
                    first: owner.team.id().clone(),
                    second: team.id().clone(),
                });
            }
        }
        for round in team.rounds() {
            if self.find_round(round.id()).is_some() {
                return Err(InvalidState::DuplicateRound {
                    round: round.id().clone(),
                    team: team.id().clone(),
                });
            }
        }
        self.teams.push(team);
        self.sync_queued();
        Ok(())
    }

    /// Adds a reviewer to team `team`.
    ///
    /// # Errors
    /// [`AddReviewerError`] when the state has no such team, or the reviewer is not an investigator, is in the team's
    /// crew or among its reviewers already, or its task ID is in use.
    pub fn add_reviewer(&mut self, team: &TeamId, reviewer: Member) -> Result<(), AddReviewerError> {
        if self.team(team).is_none() {
            return Err(AddReviewerError::UnknownTeam { team: team.clone() });
        }
        if self.find_task(reviewer.task()).is_some() {
            return Err(ReviewerProblem::DuplicateTask {
                task: reviewer.task().clone(),
            }
            .into());
        }
        match self.teams.iter_mut().find(|found| found.id() == team) {
            Some(found) => {
                found.add_reviewer(reviewer).map_err(AddReviewerError::from)?;
                self.sync_queued();
                Ok(())
            }
            None => Err(AddReviewerError::UnknownTeam { team: team.clone() }),
        }
    }

    /// Reads a state from JSON text.
    ///
    /// # Errors
    /// [`StateError`]: the text is too large, declares an unsupported schema (checked first), is malformed,
    /// or has clashing teams, tasks or rounds. Versions 11, 12 and 13 preserve activity history and default split plans to absent.
    /// Versions 1 through 10 migrate to [`SCHEMA_VERSION`], rebuilding
    /// investigator activity history deterministically from the evidence they retain.
    pub fn from_json(text: &str) -> Result<Self, StateError> {
        if text.len() > MAX_STATE_BYTES {
            return Err(StateError::TooLarge {
                len: text.len(),
                max: MAX_STATE_BYTES,
            });
        }
        let probe: SchemaProbe = serde_json::from_str(text).map_err(|source| StateError::Malformed { source })?;
        if probe.schema_version == u64::from(migrate::LEGACY_VERSION) {
            return migrate::from_v1(text);
        }
        if matches!(probe.schema_version, 2..=10) {
            return migrate::from_v2(text);
        }
        if matches!(probe.schema_version, 11..=13) {
            return migrate::from_v11(text);
        }
        if probe.schema_version == 14 {
            return migrate::from_v14(text);
        }
        if probe.schema_version != u64::from(SCHEMA_VERSION) {
            return Err(UnsupportedSchema {
                found: probe.schema_version,
            }
            .into());
        }
        let wire: StateWire = serde_json::from_str(text).map_err(|source| StateError::Malformed { source })?;
        Ok(Self::try_from(wire)?)
    }

    /// The state as JSON text, indented, ending with a newline.
    ///
    /// # Errors
    /// [`RenderError`] when the serializer fails.
    pub fn to_json(&self) -> Result<String, RenderError> {
        let mut text = serde_json::to_string_pretty(self).map_err(|source| RenderError { source })?;
        text.push('\n');
        Ok(text)
    }
}

/// Every member of `team`: its crew, then its reviewers.
fn team_members(team: &Team) -> impl Iterator<Item = &Member> {
    team.crew()
        .roster()
        .into_iter()
        .map(|(_, member)| member)
        .chain(team.reviewers())
}

impl Default for State {
    fn default() -> Self {
        Self::new()
    }
}

impl TryFrom<StateWire> for State {
    type Error = InvalidState;

    fn try_from(wire: StateWire) -> Result<Self, Self::Error> {
        let mut teams = HashSet::new();
        let mut owners: HashMap<&TaskId, &TeamId> = HashMap::new();
        let mut round_owners: HashMap<&RoundId, &TeamId> = HashMap::new();
        for team in &wire.teams {
            if !teams.insert(team.id()) {
                return Err(InvalidState::DuplicateTeam {
                    team: team.id().clone(),
                });
            }
            for round in team.rounds() {
                if round_owners.insert(round.id(), team.id()).is_some() {
                    return Err(InvalidState::DuplicateRound {
                        round: round.id().clone(),
                        team: team.id().clone(),
                    });
                }
            }
            for member in team_members(team) {
                if let Some(first) = owners.insert(member.task(), team.id()) {
                    return Err(InvalidState::DuplicateTask {
                        task: member.task().clone(),
                        first: first.clone(),
                        second: team.id().clone(),
                    });
                }
            }
        }
        let mut state = Self {
            schema_version: wire.schema_version,
            teams: wire.teams,
            wake_deliveries: wire.wake_deliveries,
            queued: wire.queued,
            recoveries: wire.recoveries,
            limit_refusals: wire.limit_refusals,
            refused_agents: wire.refused_agents,
            commit_cuts: wire.commit_cuts,
            pending_team_replaces: wire.pending_team_replaces,
        };
        state.sync_queued();
        Ok(state)
    }
}

#[cfg(test)]
mod tests {
    use serde_json::error::Category;
    use serde_json::json;

    use super::fixtures::{assert_refused, inv, member, normal, task, team_id, team_of};
    use super::*;
    use crate::state::ItemPhase;
    use crate::status::TaskStatus;

    /// Two teams with nothing in common.
    fn two_teams() -> State {
        let mut state = State::new();
        state.insert_team(team_of(1, 1)).unwrap();
        state.insert_team(team_of(2, 11)).unwrap();
        state
    }

    // ---- building a state ---------------------------------------------------------------------------------------

    #[test]
    fn a_new_state_has_no_teams() {
        let state = State::new();

        assert_eq!(state.teams(), []);
        assert_eq!(state, State::default());
        assert!(state.team(&team_id(1)).is_none());
    }

    #[test]
    fn teams_are_added_in_order_and_found_by_id() {
        let state = two_teams();

        let ids: Vec<&TeamId> = state.teams().iter().map(Team::id).collect();

        assert_eq!(ids, [&team_id(1), &team_id(2)]);
        assert_eq!(state.team(&team_id(2)).map(Team::id), Some(&team_id(2)));
        assert!(state.team(&team_id(3)).is_none());
    }

    #[test]
    fn a_team_can_be_changed_through_the_state() {
        let mut state = two_teams();

        let item = "1.2".parse().unwrap();
        state
            .team_mut(&team_id(2))
            .unwrap()
            .item_mut(item)
            .unwrap()
            .advance(ItemPhase::InProgress {
                assignee: "codex-1".parse().unwrap(),
                branch: "agents/codex-1/item-1-2".parse().unwrap(),
            })
            .unwrap();

        let first = state.team(&team_id(1)).unwrap().item(item).unwrap().status();
        let second = state.team(&team_id(2)).unwrap().item(item).unwrap().status();
        assert_eq!(
            (first, second),
            (
                crate::status::ItemStatus::Pending,
                crate::status::ItemStatus::InProgress
            )
        );
        assert!(state.team_mut(&team_id(3)).is_none());
    }

    #[test]
    fn a_team_with_a_known_id_is_refused_and_the_state_stays_as_it_was() {
        let mut state = two_teams();
        let before = state.clone();

        let err = state.insert_team(team_of(2, 21)).unwrap_err();

        assert_eq!(err, InvalidState::DuplicateTeam { team: team_id(2) });
        assert_eq!(state, before);
    }

    #[test]
    fn a_team_with_a_task_id_the_state_has_is_refused_and_the_state_stays_as_it_was() {
        let mut state = two_teams();
        let before = state.clone();

        let err = state.insert_team(team_of(3, 12)).unwrap_err();

        assert_eq!(
            err,
            InvalidState::DuplicateTask {
                task: task(12),
                first: team_id(2),
                second: team_id(3)
            }
        );
        assert_eq!(state, before);
    }

    #[test]
    fn a_task_is_found_with_its_team_and_role_and_its_member_can_be_changed() {
        let mut state = two_teams();
        state.add_reviewer(&team_id(1), member(31, inv("inv-codex"))).unwrap();

        let member = state.find_task(&task(12)).unwrap();
        assert_eq!(
            (member.team.id(), member.role, member.member.agent().name()),
            (&team_id(2), Role::Member, "codex-1")
        );
        assert_eq!(
            state.find_task(&task(31)).map(|t| (t.team.id().clone(), t.role)),
            Some((team_id(1), Role::Reviewer))
        );
        assert!(state.find_task(&task(40)).is_none());
        let (role, mut found) = state.find_task_mut(&task(12)).unwrap();
        assert_eq!(role, Role::Member);
        found.transition(TaskStatus::Running).unwrap();
        assert_eq!(
            state.find_task(&task(12)).map(|t| t.member.status()),
            Some(TaskStatus::Running)
        );
        assert_eq!(
            state.find_task(&task(2)).map(|t| t.member.status()),
            Some(TaskStatus::Queued)
        );
        assert!(state.find_task_mut(&task(40)).is_none());
    }

    #[test]
    fn a_reviewer_is_added_to_the_team_it_reviews() {
        let mut state = two_teams();

        state.add_reviewer(&team_id(2), member(31, inv("inv-codex"))).unwrap();

        assert_eq!(state.team(&team_id(2)).map(|t| t.reviewers().len()), Some(1));
        assert_eq!(state.team(&team_id(1)).map(|t| t.reviewers().len()), Some(0));
    }

    #[test]
    fn a_reviewer_for_an_unknown_team_or_with_a_task_in_use_anywhere_is_refused() {
        let mut state = two_teams();
        state.add_reviewer(&team_id(1), member(31, inv("inv-codex"))).unwrap();
        let before = state.clone();

        assert_eq!(
            state.add_reviewer(&team_id(9), member(32, inv("inv-gemini"))),
            Err(AddReviewerError::UnknownTeam { team: team_id(9) })
        );
        for taken in [12, 31] {
            assert_eq!(
                state.add_reviewer(&team_id(1), member(taken, inv("inv-gemini"))),
                Err(AddReviewerError::Problem(ReviewerProblem::DuplicateTask {
                    task: task(taken)
                })),
                "task {taken}"
            );
        }
        assert_eq!(
            state.add_reviewer(&team_id(1), member(32, normal("claude-1"))),
            Err(AddReviewerError::Problem(ReviewerProblem::NotInvestigator {
                agent: normal("claude-1")
            }))
        );
        assert_eq!(state, before);
    }

    // ---- the schema version -------------------------------------------------------------------------------------

    #[test]
    fn the_schema_version_is_one() {
        assert_eq!(SCHEMA_VERSION, 15);
        assert_eq!(serde_json::to_value(SchemaVersion).unwrap(), json!(15));
    }

    #[test]
    fn only_the_schema_version_of_this_build_is_read() {
        serde_json::from_value::<SchemaVersion>(json!(15)).unwrap();
        for bad in [
            json!(0),
            json!(1),
            json!(u64::MAX),
            json!(-1),
            json!("2"),
            json!(1.5),
            json!(null),
        ] {
            assert_refused::<SchemaVersion>(&bad.clone());
        }
    }

    // ---- reading and writing ------------------------------------------------------------------------------------

    #[test]
    fn a_state_saved_and_loaded_back_is_equal() {
        let mut state = two_teams();
        state.add_reviewer(&team_id(1), member(31, inv("inv-codex"))).unwrap();
        state
            .find_task_mut(&task(2))
            .unwrap()
            .1
            .transition(TaskStatus::Running)
            .unwrap();

        let text = state.to_json().unwrap();

        assert_eq!(State::from_json(&text).unwrap(), state);
    }

    #[test]
    fn an_empty_state_is_written_with_its_schema_version_first() {
        let text = State::new().to_json().unwrap();

        assert_eq!(text, "{\n  \"schema_version\": 15,\n  \"teams\": []\n}\n");
        assert_eq!(State::from_json(&text).unwrap(), State::new());
    }

    #[test]
    fn a_state_is_written_indented_and_ends_with_a_newline_and_the_schema_version_comes_first() {
        let text = two_teams().to_json().unwrap();

        assert!(text.starts_with("{\n  \"schema_version\": 15,\n  \"teams\": [\n    {\n      \"id\": \"team-"));
        assert!(text.ends_with("}\n"));
        assert!(!text.ends_with("\n\n"));
    }

    #[test]
    fn a_state_of_another_schema_version_is_refused_with_that_version() {
        for found in [0_u64, 16, 99, u64::MAX] {
            let text = format!("{{\"schema_version\": {found}, \"teams\": []}}");

            let err = State::from_json(&text).unwrap_err();

            assert!(
                matches!(&err, StateError::UnsupportedSchema(UnsupportedSchema { found: f }) if *f == found),
                "{found}"
            );
        }
    }

    #[test]
    fn the_version_is_checked_before_the_rest_so_a_newer_state_is_not_called_malformed() {
        for text in [
            "{\"schema_version\": 16, \"teams\": {\"by_id\": 5}}",
            "{\"teams\": [], \"schema_version\": 16, \"future\": true}",
            "{\"schema_version\": 16}",
        ] {
            let err = State::from_json(text).unwrap_err();

            assert!(matches!(err, StateError::UnsupportedSchema(_)), "{text}");
        }
    }

    #[test]
    fn a_text_that_is_not_a_state_is_malformed() {
        for text in [
            "",
            "not json",
            "[]",
            "null",
            "{}",
            "{\"teams\": []}",
            "{\"schema_version\": \"1\", \"teams\": []}",
            "{\"schema_version\": -1, \"teams\": []}",
            "{\"schema_version\": 1.5, \"teams\": []}",
            "{\"schema_version\": 1}",
            "{\"schema_version\": 1, \"teams\": [], \"extra\": 1}",
            "{\"schema_version\": 1, \"teams\": [{}]}",
            "{\"schema_version\": 1, \"teams\": []} trailing",
        ] {
            let err = State::from_json(text).unwrap_err();

            assert!(matches!(err, StateError::Malformed { .. }), "{text:?}: {err:?}");
        }
    }

    #[test]
    fn a_malformed_state_keeps_the_parsers_reason_as_its_source() {
        let err = State::from_json("{\"schema_version\": 1, \"teams\": [{}]}").unwrap_err();

        let source = std::error::Error::source(&err).expect("a source");

        assert!(source.to_string().contains("missing field"), "{source}");
    }

    #[test]
    fn a_state_with_two_teams_of_one_id_or_two_tasks_of_one_id_is_refused_with_both_teams() {
        let one = serde_json::to_value(team_of(1, 1)).unwrap();
        let same_team = serde_json::to_value(team_of(1, 11)).unwrap();
        let same_tasks = serde_json::to_value(team_of(2, 1)).unwrap();
        let state =
            |teams: &[&serde_json::Value]| json!({"schema_version": SCHEMA_VERSION, "teams": teams}).to_string();

        let teams = State::from_json(&state(&[&one, &same_team])).unwrap_err();
        let tasks = State::from_json(&state(&[&one, &same_tasks])).unwrap_err();

        assert!(matches!(teams, StateError::Invalid(InvalidState::DuplicateTeam { team }) if team == team_id(1)));
        assert!(matches!(
            tasks,
            StateError::Invalid(InvalidState::DuplicateTask { task: t, first, second })
                if t == task(1) && first == team_id(1) && second == team_id(2)
        ));
    }

    #[test]
    fn a_reviewer_with_a_task_of_its_teams_crew_is_refused_when_read() {
        let mut value = serde_json::to_value(team_of(1, 1)).unwrap();
        value["reviewers"] = json!([{
            "task": "task-20261111-153002-k3x9",
            "agent": {"tier": "investigator", "name": "inv-codex"},
            "status": "queued"
        }]);

        let err = State::from_json(&json!({"schema_version": 1, "teams": [value]}).to_string()).unwrap_err();

        assert!(matches!(err, StateError::Malformed { .. }), "{err:?}");
    }

    #[test]
    fn a_state_over_the_limit_is_refused_before_it_is_read() {
        let text = " ".repeat(MAX_STATE_BYTES + 1);

        let err = State::from_json(&text).unwrap_err();

        assert!(
            matches!(err, StateError::TooLarge { len, max } if len == MAX_STATE_BYTES + 1 && max == MAX_STATE_BYTES)
        );
    }

    #[test]
    fn a_state_of_exactly_the_limit_is_read_not_refused_for_its_size() {
        let text = " ".repeat(MAX_STATE_BYTES);

        let err = State::from_json(&text).unwrap_err();

        assert!(matches!(err, StateError::Malformed { .. }), "{err:?}");
    }

    #[test]
    fn serde_reads_a_state_through_the_same_checks() {
        let good = State::new().to_json().unwrap();
        let other_version = "{\"schema_version\": 16, \"teams\": []}";
        let one = serde_json::to_value(team_of(1, 1)).unwrap();
        let duplicate = json!({"schema_version": 1, "teams": [one, one]}).to_string();

        assert_eq!(serde_json::from_str::<State>(&good).unwrap(), State::new());
        for text in [other_version, duplicate.as_str()] {
            let err = serde_json::from_str::<State>(text).unwrap_err();

            assert_eq!(err.classify(), Category::Data, "{text}");
        }
    }

    #[test]
    fn resuming_an_older_teams_task_places_it_after_an_already_queued_newer_task_across_save_and_load() {
        let mut state = State::new();
        for (team_no, first_task) in [(1, 1), (2, 11), (3, 21)] {
            state.insert_team(team_of(team_no, first_task)).unwrap();
        }
        for (team_no, first_task) in [(1, 1), (2, 11), (3, 21)] {
            let mut team = state.team_mut(&team_id(team_no)).unwrap();
            for offset in [0, 1, 2] {
                let (_, mut member) = team.find_mut(&task(first_task + offset)).unwrap();
                member.transition(TaskStatus::Running).unwrap();
                if !(team_no == 2 && offset == 1) {
                    member.transition(TaskStatus::Waiting).unwrap();
                }
            }
        }
        state.sync_queued();
        state
            .team_mut(&team_id(3))
            .unwrap()
            .find_mut(&task(22))
            .unwrap()
            .1
            .transition(TaskStatus::Queued)
            .unwrap();
        state.sync_queued();
        state
            .team_mut(&team_id(1))
            .unwrap()
            .find_mut(&task(2))
            .unwrap()
            .1
            .transition(TaskStatus::Queued)
            .unwrap();
        state.sync_queued();

        let json = state.to_json().unwrap();
        let mut reloaded = State::from_json(&json).unwrap();
        reloaded
            .team_mut(&team_id(2))
            .unwrap()
            .find_mut(&task(12))
            .unwrap()
            .1
            .transition(TaskStatus::Waiting)
            .unwrap();
        let queued: Vec<_> = reloaded.queued_tasks().into_iter().map(|item| item.task).collect();

        assert_eq!(queued, vec![task(22), task(2)]);
    }
}

#[cfg(test)]
#[path = "state/allowances_tests.rs"]
mod allowances_tests;

#[cfg(test)]
mod short_desc_tests;

#[cfg(test)]
mod legacy_short_desc_tests;

#[cfg(test)]
#[path = "state/split_migration_tests.rs"]
mod split_migration_tests;
