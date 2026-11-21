//! Who carries on a task the ladder has no step left for: another agent of `[replacement] agents`, which holds the
//! task from now on and starts it briefed in a new session, in its worktree and on its branch; or, with nobody fitting,
//! the team's lead, which is given a member's work; or, as before, nobody, and the human decides.

use agent_core::decide::ladder::Trouble;
use agent_core::decide::replacement::{Activity, Candidate, Handover, ReplacementMode, choose};
use agent_core::ids::{AgentName, ItemId, TaskId};
use agent_core::kinds::{Role, Tier};
use agent_core::state::{ItemPhase, Participant, Timestamp};
use agent_core::status::TaskStatus;
use agent_runtime::history::log_decision;

use super::{Owner, OwnerError};

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
                handed_over(&team, task, held.name(), name.as_str());
                Takeover::Agent
            }
            Chosen::Lead => {
                // The state owes the work to the lead until it is told, so that a restart before then tells it all the same.
                self.parts.state.owe_lead(task, trouble, at)?;
                log_decision("takeover", &format!("task={task}, from={}", held.name()), LEAD);
                handed_over(&team, task, held.name(), LEAD);
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
        let listed: Vec<(&AgentName, Tier, Activity)> = replacement
            .agents()
            .iter()
            .filter(|name| name.as_str() != held.name() && !in_team.contains(&name.as_str()))
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
            Handover::Lead => Some(Chosen::Lead),
            // The question to the human after the last step is not asked yet: the task fails as before.
            Handover::Human => None,
        };
        Ok(chosen)
    }

    /// Gives `task`, held by `held`, to the agent `name` at `at`, with the items it works on, and starts it anew after
    /// `trouble`: the next run is briefed, in a session of the new agent, and its retries are its own.
    fn hand_task_over(
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
        let team_id = self
            .parts
            .state
            .find_task(task)
            .ok_or_else(|| OwnerError::LostTask { task: task.clone() })?
            .team
            .id()
            .clone();
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
        Ok(())
    }

    /// The tier the configuration gives the agent `name`; `None` for a name it does not have.
    fn tier_of(&self, name: &AgentName) -> Option<Tier> {
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
    fn participant_of(&self, name: &AgentName) -> Option<Participant> {
        match self.tier_of(name)? {
            Tier::Investigator => name.as_str().parse().ok().map(Participant::Investigator),
            Tier::Normal => Some(Participant::Normal(name.clone())),
            Tier::Worker => Some(Participant::Worker(name.clone())),
        }
    }

    /// The names of the agents in a run now, in any team: a task of theirs runs, or a run of it is going.
    fn agents_in_runs(&self) -> Vec<&str> {
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
            .collect()
    }
}

/// What `task_handed_over` names as the new holder when a member's work goes to its team's lead.
const LEAD: &str = "lead";

/// Logs that `task` of `team` was handed over from the agent `from` to `to`: an agent's name, or [`LEAD`].
fn handed_over(team: &agent_core::ids::TeamId, task: &TaskId, from: &str, to: &str) {
    tracing::info!(
        event = agent_core::log::LogEvent::TaskHandedOver.as_str(),
        team = %team,
        task = %task,
        from,
        to,
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
