//! Who carries on a task the ladder has no step left for: another agent of `[replacement] agents`, which holds the
//! task from now on and starts it briefed in a new session, in its worktree and on its branch; or, with nobody fitting,
//! the team's lead, which is given a member's work; or, as before, nobody, and the human decides.

use agent_core::decide::busy::{self, Availability, Decision, Offer, TaskWait, Waiting};
use agent_core::decide::ladder::Trouble;
use agent_core::decide::replacement::{Activity, Candidate, Handover, ReplacementMode, choose};
use agent_core::history::CheckpointCause;
use agent_core::ids::{AgentName, ItemId, TaskId};
use agent_core::kinds::{CliKind, Role, Tier};
use agent_core::state::{ItemPhase, Participant, Team, Timestamp};
use agent_core::status::TaskStatus;
use agent_runtime::history::log_decision;

use super::{Owner, OwnerError};
use crate::daemon::team::BusJob;

/// Why a busy hand-over notice of one item cannot be rendered; the owed notice stays queued for diagnosis.
#[derive(Debug, thiserror::Error)]
pub(crate) enum BusyNoticeError {
    /// The task disappeared from the state before its notice was sent.
    #[error("task {task} for the busy hand-over notice is not in the state")]
    MissingTask {
        /// The task whose hand-over was to be announced.
        task: TaskId,
    },
    /// The item disappeared from its team before its notice was sent.
    #[error("item {item} for the busy hand-over notice is not in the task's team")]
    MissingItem {
        /// The item whose hand-over was to be announced.
        item: ItemId,
    },
    /// An item in the phase carries no branch to name in the message.
    #[error("item {item} has no branch for its busy hand-over notice")]
    MissingBranch {
        /// The item whose branch cannot be told.
        item: ItemId,
    },
    /// The task has no former holder to name in the message.
    #[error("task {task} has no previous holder for its busy hand-over notice")]
    MissingPreviousHolder {
        /// The task whose previous holder was to be announced.
        task: TaskId,
    },
    /// The prompt variables cannot be built.
    #[error("cannot make the variables of item {item}'s busy hand-over notice")]
    Variables {
        /// The item whose text was being built.
        item: ItemId,
        /// The invalid name or value.
        source: agent_core::prompt::VariablesError,
    },
    /// The prompt cannot be rendered.
    #[error("cannot render item {item}'s busy hand-over notice")]
    Render {
        /// The item whose text was being rendered.
        item: ItemId,
        /// The renderer's reason, kept as the cause.
        source: agent_runtime::team_prompts::RenderTeamTextError,
    },
}

/// Who carries on a task whose new sessions went bad too.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Takeover {
    /// Another agent holds the task now; its next run is briefed, in a new session of its own.
    Agent,
    /// The member's task stops and the team's lead is given its work.
    Lead,
    /// Nobody: the task fails and the human decides, as before `[replacement] agents`.
    Nobody,
}

/// Whether the agent `name` can take a task over now: busy when one of its tasks runs or waits to start, paused when one
/// is paused, and free otherwise, in any team.
fn availability_of(teams: &[Team], name: &AgentName) -> Availability {
    let members = teams.iter().flat_map(|team| {
        let crew = team.crew().roster().into_iter().map(|(_, member)| member);
        crew.chain(team.reviewers().iter())
    });
    let mut availability = Availability::Free;
    for member in members.filter(|member| member.agent().agent_name() == *name) {
        match member.status() {
            TaskStatus::Running | TaskStatus::Queued => return Availability::Busy,
            TaskStatus::Paused => availability = Availability::Paused,
            TaskStatus::Waiting
            | TaskStatus::Questions
            | TaskStatus::Done
            | TaskStatus::Stopped
            | TaskStatus::Failed => {}
        }
    }
    availability
}

impl Owner {
    /// Who carries on `task` of `role`, whose runs went bad with `trouble` and which the ladder cannot start anew. A
    /// hand-over to an agent is recorded in the state at once; the lead is told of its work when the end of the run is
    /// settled (`after_run`).
    ///
    /// # Errors
    /// [`OwnerError`] when the task is not in the state or the state refuses the hand-over.
    pub(super) fn take_over(&mut self, task: &TaskId, role: Role, trouble: Trouble) -> Result<Takeover, OwnerError> {
        let Some(chosen) = self.choose_for(task, role)? else {
            return Ok(Takeover::Nobody);
        };
        let at = self.after_last_run(task)?;
        let (held, team) = {
            let found = self
                .parts
                .state
                .find_task(task)
                .ok_or_else(|| OwnerError::LostTask { task: task.clone() })?;
            (found.member.agent().clone(), found.team.id().clone())
        };
        let takeover = match chosen {
            Chosen::Agent(name) => {
                self.hand_task_over(task, &name, &held, trouble, at)?;
                log_decision(
                    "takeover",
                    &format!("task={task}, from={}", held.name()),
                    &format!("agent={name}"),
                );
                handed_over(&team, task, held.name(), name.as_str(), trouble);
                Takeover::Agent
            }
            Chosen::Lead => {
                // The state owes the work to the lead until it is told, so that a restart before then tells it all the same.
                self.parts.state.owe_lead(task, trouble, at)?;
                log_decision("takeover", &format!("task={task}, from={}", held.name()), LEAD);
                handed_over(&team, task, held.name(), LEAD, trouble);
                self.queue_for_task(task, CheckpointCause::HolderChanged);
                Takeover::Lead
            }
        };
        Ok(takeover)
    }

    /// The choice of `[replacement]` for `task` of `role`; `None` when the task fails as before: mode `off`, the task
    /// taken over as often as `replacements` allows, or nobody but the human fits. With no agents listed, `auto` gives a
    /// member's work to its lead, as `lead` does.
    fn choose_for(&self, task: &TaskId, role: Role) -> Result<Option<Chosen>, OwnerError> {
        let replacement = self.parts.settings.config.replacement();
        let found = self
            .parts
            .state
            .find_task(task)
            .ok_or_else(|| OwnerError::LostTask { task: task.clone() })?;
        let member = found.member;
        let taken_over = u32::try_from(member.past_holders().len()).unwrap_or(u32::MAX);
        if replacement.mode() == ReplacementMode::Off || taken_over >= replacement.replacements().get() {
            return Ok(None);
        }
        let held = member.agent();
        // An agent of the team is never offered a task of it: its own place in the team would be lost.
        let in_team: Vec<&str> = found
            .team
            .crew()
            .roster()
            .into_iter()
            .map(|(_, member)| member)
            .chain(found.team.reviewers())
            .map(|member| member.agent().name())
            .collect();
        let busy = self.agents_in_runs();
        let now = Timestamp::now(&self.parts.clock)?;
        let refused = self.unavailable_agents_at(now);
        let listed: Vec<(&AgentName, Tier, Activity)> = replacement
            .agents()
            .iter()
            .filter(|name| name.as_str() != held.name() && !in_team.contains(&name.as_str()) && !refused.contains(name))
            .filter_map(|name| {
                let activity = if busy.contains(&name.as_str()) {
                    Activity::InRun
                } else {
                    Activity::Idle
                };
                self.tier_of(name).map(|tier| (name, tier, activity))
            })
            .collect();
        let candidates: Vec<Candidate<'_>> = listed
            .iter()
            .map(|(agent, tier, activity)| Candidate {
                agent,
                tier: *tier,
                activity: *activity,
            })
            .collect();
        let chosen = match choose(role, held.tier(), &candidates, replacement.mode()) {
            Handover::Agent(name) => Some(Chosen::Agent(name.clone())),
            Handover::Lead => agent_core::decide::assign::dispatch::coordinator_can_take_over(found.team, member)
                .then_some(Chosen::Lead),
            // The question to the human after the last step is not asked yet: the task fails as before.
            Handover::Human => None,
        };
        Ok(chosen)
    }

    /// Gives `task`, held by `held`, to the agent `name` at `at`, with the items it works on, and starts it anew after
    /// `trouble`: the next run is briefed, in a session of the new agent, and its retries are its own.
    pub(super) fn hand_task_over(
        &mut self,
        task: &TaskId,
        name: &AgentName,
        held: &Participant,
        trouble: Trouble,
        at: Timestamp,
    ) -> Result<(), OwnerError> {
        let holder = self
            .participant_of(name)
            .ok_or_else(|| OwnerError::LostTask { task: task.clone() })?;
        let found = self
            .parts
            .state
            .find_task(task)
            .ok_or_else(|| OwnerError::LostTask { task: task.clone() })?;
        let team_id = found.team.id().clone();
        let role = found.role;
        let items: Vec<(ItemId, agent_core::ids::BranchName)> = self
            .parts
            .state
            .team(&team_id)
            .map(|team| {
                team.items()
                    .iter()
                    .filter_map(|progress| match progress.phase() {
                        ItemPhase::InProgress { assignee, branch } | ItemPhase::Fixing { assignee, branch, .. } => {
                            (assignee.as_str() == held.name()).then(|| (progress.id(), branch.clone()))
                        }
                        // An item nobody works on, or one under review or finished, is not the task's to carry.
                        ItemPhase::Pending
                        | ItemPhase::InReview { .. }
                        | ItemPhase::Approved { .. }
                        | ItemPhase::Done { .. } => None,
                    })
                    .collect()
            })
            .unwrap_or_default();
        self.parts
            .state
            .hand_over_task(task, holder, at)
            .map_err(|source| OwnerError::HandOver {
                task: task.clone(),
                source,
            })?;
        if matches!(role, Role::Coordinator | Role::Member) {
            self.parts
                .state
                .record_pending_team_replace(team_id.clone(), held.agent_name(), name.clone());
        }
        // The items follow the task: the agent that took it over holds them, on the branches they were on.
        let mut team = self
            .parts
            .state
            .team_mut(&team_id)
            .ok_or_else(|| OwnerError::LostTeam { team: team_id.clone() })?;
        for (id, branch) in items {
            team.item_mut(id)
                .ok_or_else(|| OwnerError::LostItem {
                    team: team_id.clone(),
                    item: id,
                })?
                .reassign(name.clone(), branch)
                .map_err(|source| OwnerError::Reassign {
                    task: task.clone(),
                    item: id,
                    source,
                })?;
        }
        // The new agent's own session: no session of the agent before it is resumed, the next run is briefed, and the bad
        // runs before the hand-over are not counted against it; no new session of the task's agent is counted. A task
        // without a run has no session to resume either, so its next run is a first one all the same.
        let begun = self.parts.state.begin_holder_session(task, trouble, at)?;
        if !begun {
            tracing::debug!(task = %task, "a task handed over before any run of it begins no session");
        }
        self.queue_for_task(task, CheckpointCause::HolderChanged);
        Ok(())
    }

    /// The tier the configuration gives the agent `name`; `None` for a name it does not have.
    pub(super) fn tier_of(&self, name: &AgentName) -> Option<Tier> {
        let config = &self.parts.settings.config;
        if config
            .investigators()
            .keys()
            .any(|investigator| investigator.as_str() == name.as_str())
        {
            return Some(Tier::Investigator);
        }
        if config.normal_agents().contains(name) {
            return Some(Tier::Normal);
        }
        config.worker_agents().contains(name).then_some(Tier::Worker)
    }

    /// The agent `name` as the participant of a task, by its tier in the configuration.
    pub(super) fn participant_of(&self, name: &AgentName) -> Option<Participant> {
        match self.tier_of(name)? {
            Tier::Investigator => name.as_str().parse().ok().map(Participant::Investigator),
            Tier::Normal => Some(Participant::Normal(name.clone())),
            Tier::Worker => Some(Participant::Worker(name.clone())),
        }
    }

    /// The hand-over of a queued `task` that waits for its busy agent: once it has waited `[replacement] busy_after`
    /// seconds, a free agent of `[replacement] agents` takes it over, as [`Owner::hand_task_over`] does, and the task
    /// starts with it. Returns whether the task was handed over; the moment its wait began is kept in the state.
    ///
    /// # Errors
    /// [`OwnerError`] when the task is not in the state or the state cannot be saved.
    pub(super) async fn hand_over_busy_wait(&mut self, task: &TaskId) -> Result<bool, OwnerError> {
        let now = Timestamp::now(&self.parts.clock)?;
        let was_waiting = self
            .parts
            .state
            .find_task(task)
            .ok_or_else(|| OwnerError::LostTask { task: task.clone() })?
            .member
            .waiting_since()
            .is_some();
        let since = self.parts.state.start_waiting(task, now)?;
        if !was_waiting {
            self.save().await?;
        }
        let Some((name, held)) = self.busy_choice(task, since, now)? else {
            return Ok(false);
        };
        let team = self
            .parts
            .state
            .find_task(task)
            .ok_or_else(|| OwnerError::LostTask { task: task.clone() })?
            .team
            .id()
            .clone();
        self.hand_task_over(task, &name, &held, Trouble::Busy, now)?;
        tracing::info!(
            event = agent_core::log::LogEvent::TaskHandedOver.as_str(),
            team = %team,
            task = %task,
            from = held.name(),
            to = name.as_str(),
            reason = "busy",
            "a task that waited too long for its busy agent is handed over to a free agent"
        );
        if self
            .parts
            .state
            .team(&team)
            .is_some_and(|found| found.crew().coordinator().is_some())
        {
            self.parts.state.owe_busy_notice(task)?;
        }
        self.save().await?;
        Ok(true)
    }

    /// Queues every still-owed team notice once; recovery rebuilds the outbox from the same persisted obligations.
    pub(super) async fn queue_busy_handover_notices(&mut self) -> Result<(), OwnerError> {
        let pending: Vec<(agent_core::ids::TeamId, TaskId)> = self
            .parts
            .state
            .teams()
            .iter()
            .flat_map(|team| {
                let team_id = team.id().clone();
                team.crew()
                    .roster()
                    .into_iter()
                    .filter(|(_, member)| member.busy_notice_owed())
                    .map(move |(_, member)| (team_id.clone(), member.task().clone()))
            })
            .collect();
        for (team_id, task) in pending {
            let Some(found) = self.parts.state.find_task(&task) else {
                continue;
            };
            let linked: std::collections::BTreeSet<ItemId> = found
                .member
                .runs()
                .iter()
                .flat_map(agent_core::state::Run::rounds)
                .filter_map(|round| found.team.round(round))
                .filter_map(agent_core::state::Round::item)
                .collect();
            let current = found.member.agent().name();
            let items: Vec<ItemId> = found
                .team
                .items()
                .iter()
                .filter(|progress| match progress.phase() {
                    ItemPhase::InProgress { assignee, .. } | ItemPhase::Fixing { assignee, .. } => {
                        assignee.as_str() == current || linked.contains(&progress.id())
                    }
                    ItemPhase::InReview { .. } | ItemPhase::Approved { .. } | ItemPhase::Done { .. } => {
                        linked.contains(&progress.id())
                    }
                    ItemPhase::Pending => false,
                })
                .map(agent_core::state::ItemProgress::id)
                .collect();
            if items.is_empty() {
                self.parts.state.busy_notice_told(&task)?;
                self.save().await?;
            }
            for item in items {
                let queued = self.outbox.iter().any(|outbound| {
                    outbound.team == team_id
                        && matches!(
                            &outbound.job,
                            BusJob::BusyHandOver { task: pending_task, item: pending_item }
                                if *pending_task == task && *pending_item == item
                        )
                });
                if !queued {
                    self.tell_bus(
                        &team_id,
                        BusJob::BusyHandOver {
                            task: task.clone(),
                            item,
                        },
                    );
                }
            }
        }
        Ok(())
    }

    /// The text of the team notice of `task`'s `item`, rendered only when the outbox is ready to send it.
    pub(super) fn busy_handover_text(&self, task: &TaskId, item: ItemId) -> Result<String, BusyNoticeError> {
        let found = self
            .parts
            .state
            .find_task(task)
            .ok_or_else(|| BusyNoticeError::MissingTask { task: task.clone() })?;
        let progress = found.team.item(item).ok_or(BusyNoticeError::MissingItem { item })?;
        let branch = match progress.phase() {
            ItemPhase::InProgress { branch, .. } | ItemPhase::Fixing { branch, .. } => branch,
            ItemPhase::InReview { pr, .. }
            | ItemPhase::Approved { pr, .. }
            | ItemPhase::Done {
                completion: agent_core::state::Completion::Merged { pr, .. },
            } => &pr.branch,
            ItemPhase::Pending
            | ItemPhase::Done {
                completion: agent_core::state::Completion::Listed,
            } => return Err(BusyNoticeError::MissingBranch { item }),
        };
        let from = found
            .member
            .past_holders()
            .last()
            .map(|holding| holding.agent.name())
            .ok_or_else(|| BusyNoticeError::MissingPreviousHolder { task: task.clone() })?;
        let values = agent_core::prompt::Variables::new()
            .with("item", item.to_string())
            .and_then(|values| values.with("from", from))
            .and_then(|values| values.with("to", found.member.agent().name()))
            .and_then(|values| values.with("task", task.as_str()))
            .and_then(|values| values.with("branch", branch.as_str()))
            .map_err(|source| BusyNoticeError::Variables { item, source })?;
        self.parts
            .team_texts
            .render(agent_core::prompt::TeamPrompt::ItemHandedOver, &values)
            .map_err(|source| BusyNoticeError::Render { item, source })
    }

    /// The free agent that takes the waiting `task` over now, and the participant that holds it now; `None` when the task
    /// keeps waiting: its wait is shorter than `[replacement] busy_after`, no agent of `[replacement] agents` fits, or the
    /// task was taken over as often as `[replacement] replacements` allows.
    fn busy_choice(
        &self,
        task: &TaskId,
        since: Timestamp,
        now: Timestamp,
    ) -> Result<Option<(AgentName, Participant)>, OwnerError> {
        let replacement = self.parts.settings.config.replacement();
        let found = self
            .parts
            .state
            .find_task(task)
            .ok_or_else(|| OwnerError::LostTask { task: task.clone() })?;
        let held = found.member.agent().clone();
        let taken_over = u32::try_from(found.member.past_holders().len()).unwrap_or(u32::MAX);
        let names: Vec<AgentName> = replacement.agents().to_vec();
        let teams = self.parts.state.teams();
        let going = self.all_runs_going();
        let busy: std::collections::HashSet<_> = going.iter().map(|run| &run.agent).collect();
        // An agent its provider refused is not free until its return: it takes no task over.
        let refused = self.unavailable_agents_at(Timestamp::now(&self.parts.clock)?);
        let in_team: Vec<&str> = found
            .team
            .crew()
            .roster()
            .into_iter()
            .map(|(_, member)| member)
            .chain(found.team.reviewers())
            .map(|member| member.agent().name())
            .collect();
        let mut offers_data = Vec::new();
        for name in &names {
            let Some(participant) = self.participant_of(name) else {
                continue;
            };
            let cli = self.cli_of_participant(&participant);
            let cli_runs = going.iter().filter(|run| self.cli_of_name(&run.agent) == cli).count();
            offers_data.push((
                name,
                participant.tier(),
                in_team.contains(&name.as_str()),
                if busy.contains(name) {
                    Availability::Busy
                } else if refused.contains(name) {
                    Availability::Paused
                } else {
                    availability_of(teams, name)
                },
                u32::try_from(cli_runs).unwrap_or(u32::MAX),
            ));
        }
        let offers: Vec<Offer<'_>> = offers_data
            .iter()
            .map(|(agent, tier, in_team, availability, cli_runs)| Offer {
                agent,
                tier: *tier,
                in_team: *in_team,
                availability: *availability,
                cli_runs: *cli_runs,
            })
            .collect();
        let offers: &[Offer<'_>] = if taken_over >= replacement.replacements().get() {
            &[]
        } else {
            &offers
        };
        let member_runs = found
            .team
            .crew()
            .roster()
            .into_iter()
            .any(|(_, member)| member.status() == TaskStatus::Running);
        let in_review = found.team.items().iter().any(|progress| {
            if !matches!(progress.phase(), ItemPhase::InReview { .. }) {
                return false;
            }
            found
                .member
                .runs()
                .iter()
                .flat_map(agent_core::state::Run::rounds)
                .filter_map(|round| found.team.round(round))
                .any(|round| round.item() == Some(progress.id()))
        });
        let wait = if found.member.status() == TaskStatus::Running {
            TaskWait::Running
        } else if found.member.open_question().is_some() || found.member.status() == TaskStatus::Questions {
            TaskWait::HumanAnswer
        } else if in_review {
            TaskWait::Review
        } else {
            TaskWait::BusyAgent
        };
        let waiting = Waiting {
            role: found.role,
            wait,
            member_runs,
            held: held.tier(),
            waited: now.as_utc() - since.as_utc(),
            busy_after: replacement.busy_after(),
            offers,
        };
        Ok(match busy::decide(&waiting) {
            Decision::Keep => None,
            Decision::HandOver(agent) => Some((agent.clone(), held)),
        })
    }

    /// The CLI of the agent `participant` is, by the configuration; `None` for an agent the configuration does not name.
    fn cli_of_participant(&self, participant: &Participant) -> Option<CliKind> {
        let config = &self.parts.settings.config;
        match participant {
            Participant::Normal(name) | Participant::Worker(name) => config.agent(name).map(|agent| agent.cli),
            Participant::Investigator(name) => config.investigator(name).map(|agent| agent.cli),
        }
    }

    /// The CLI of the agent named `name`, as [`Owner::cli_of_participant`] gives it for its participant.
    fn cli_of_name(&self, name: &AgentName) -> Option<CliKind> {
        self.participant_of(name)
            .and_then(|participant| self.cli_of_participant(&participant))
    }

    /// The names of the agents in a run now, in any team: a task of theirs runs, or a run of it is going.
    pub(super) fn agents_in_runs(&self) -> Vec<&str> {
        self.parts
            .state
            .teams()
            .iter()
            .flat_map(|team| {
                let crew = team.crew().roster().into_iter().map(|(_, member)| member);
                crew.chain(team.reviewers())
            })
            .filter(|member| member.status() == TaskStatus::Running || member.current_run().is_some())
            .map(|member| member.agent().name())
            .chain(
                self.serving
                    .iter()
                    .filter(|(task, _)| self.parts.state.find_task(task).is_none())
                    .map(|(_, served)| served.worker.as_str()),
            )
            .collect()
    }
}

/// What `task_handed_over` names as the new holder when a member's work goes to its team's lead.
const LEAD: &str = "lead";

/// Logs that `task` of `team` was handed over from the agent `from` to `to` (an agent's name, or [`LEAD`]) for
/// `trouble`, whose word is the record's `reason`.
fn handed_over(team: &agent_core::ids::TeamId, task: &TaskId, from: &str, to: &str, trouble: Trouble) {
    tracing::info!(
        event = agent_core::log::LogEvent::TaskHandedOver.as_str(),
        team = %team,
        task = %task,
        from,
        to,
        reason = super::briefing::reason_word(trouble),
        "a task whose runs went bad is handed over"
    );
}

/// What `[replacement]` chose for a task, before the state is changed.
enum Chosen {
    /// The agent that takes the task over.
    Agent(AgentName),
    /// The team's lead, for a member's work.
    Lead,
}
