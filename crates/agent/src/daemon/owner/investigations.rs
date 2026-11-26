//! Serves open investigation requests with free investigators.

use std::collections::BTreeSet;
use std::num::NonZeroUsize;

use agent_core::decide::investigator::Activity;
use agent_core::ids::{AgentName, InvestigatorName, TaskId, TeamId};
use agent_core::kinds::Role;
use agent_core::log::LogEvent;
use agent_core::state::{Member, Participant, Timestamp};
use agent_core::status::TaskStatus;
use agent_runtime::agentbus::{InvTransition, Investigation, InvestigationKind};
use agent_runtime::git::Revision;

use super::Owner;
use crate::daemon::owner::OwnerError;
use crate::daemon::review::investigator_of;

impl Owner {
    /// Serves open investigation requests with free investigators for `team_id`.
    pub(super) async fn serve_investigations(&mut self, team_id: &TeamId) -> Result<(), OwnerError> {
        let now = Timestamp::now(&self.parts.clock)?;
        let configured_keys: BTreeSet<InvestigatorName> =
            self.parts.settings.config.investigators().keys().cloned().collect();
        let Some(slots) = NonZeroUsize::new(configured_keys.len()) else {
            return Ok(());
        };
        let Some(queue_max) = NonZeroUsize::new(3) else {
            return Ok(());
        };

        loop {
            let free =
                agent_core::decide::investigator::free_for_request(&self.parts.state, &self.parts.settings.config, now);
            if free.is_empty() {
                break;
            }
            let req = match self.parts.bus.inv_next(queue_max, slots).await {
                Ok(Some(req)) => req,
                Ok(None) => break,
                Err(error) => {
                    tracing::warn!(
                        event = LogEvent::BusDown.as_str(),
                        %team_id,
                        error = %error,
                        "cannot poll investigations from bus"
                    );
                    break;
                }
            };

            if matches!(req.kind, InvestigationKind::Unknown(_)) {
                tracing::warn!(
                    event = LogEvent::InvestigationKindUnknown.as_str(),
                    id = %req.id,
                    kind = ?req.kind,
                );
                let reason = format!("unknown investigation kind: {:?}", req.kind);
                if let Err(error) = self.parts.bus.inv_set(req.id, &InvTransition::Failed { reason }).await {
                    tracing::warn!(
                        event = LogEvent::BusDown.as_str(),
                        %team_id,
                        error = %error,
                        "cannot mark investigation failed on bus"
                    );
                }
                continue;
            }

            if !self.is_task_live_in_team(team_id, &req.from_task) {
                if let Err(error) = self.parts.bus.inv_set(req.id, &InvTransition::Requeued).await {
                    tracing::warn!(
                        event = LogEvent::BusDown.as_str(),
                        %team_id,
                        error = %error,
                        "cannot requeue investigation on bus"
                    );
                }
                break;
            }

            let Some(chosen) = self.pick_investigator(team_id, &req, &configured_keys, &free) else {
                if let Err(error) = self.parts.bus.inv_set(req.id, &InvTransition::Requeued).await {
                    tracing::warn!(
                        event = LogEvent::BusDown.as_str(),
                        %team_id,
                        error = %error,
                        "cannot requeue investigation on bus"
                    );
                }
                break;
            };

            self.start_investigation(team_id, &req, &chosen, now).await?;
        }

        Ok(())
    }

    /// Returns true if `task` is a live (not stopped/failed/done) member of `team_id`.
    fn is_task_live_in_team(&self, team_id: &TeamId, task: &TaskId) -> bool {
        self.parts
            .state
            .team(team_id)
            .and_then(|team| {
                team.find(task)
                    .map(|(_, m)| !matches!(m.status(), TaskStatus::Stopped | TaskStatus::Failed | TaskStatus::Done))
            })
            .unwrap_or(false)
    }

    /// Selects an investigator for `req` using LRU selection among free investigators.
    fn pick_investigator(
        &mut self,
        team_id: &TeamId,
        req: &Investigation,
        configured_keys: &BTreeSet<InvestigatorName>,
        free: &BTreeSet<InvestigatorName>,
    ) -> Option<InvestigatorName> {
        let mut excluded = BTreeSet::new();
        if let Some(team) = self.parts.state.team(team_id)
            && let Some((_, member)) = team.find(&req.from_task)
            && let Some(inv) = investigator_of(member)
        {
            excluded.insert(inv.clone());
        }
        if let Ok(inv) = req.from_agent.as_str().parse::<InvestigatorName>() {
            excluded.insert(inv);
        }

        let activities: Vec<_> = self
            .parts
            .state
            .teams()
            .iter()
            .flat_map(|team| {
                team.activities().iter().map(|activity| Activity {
                    investigator: activity.investigator(),
                    team: team.id(),
                    kind: activity.request().kind(),
                    asked_at: activity.asked_at(),
                })
            })
            .collect();

        agent_core::decide::investigator::choose(
            configured_keys,
            free,
            &activities,
            team_id,
            &excluded,
            NonZeroUsize::MIN,
            &mut self.parts.rng,
        )
        .and_then(|list| list.first().cloned())
    }

    /// Starts an investigator task and worktree for `req`.
    async fn start_investigation(
        &mut self,
        team_id: &TeamId,
        req: &Investigation,
        chosen: &InvestigatorName,
        now: Timestamp,
    ) -> Result<(), OwnerError> {
        let task = TaskId::generate(&self.parts.clock, &mut self.parts.rng)?;
        let lead = self.lead_of(team_id)?;
        let git = self
            .worktree_git(&lead.repo, &lead.task, &lead.agent)
            .map_err(|e| OwnerError::Publish(e.to_string()))?;
        let agent: AgentName = chosen.as_str().parse()?;
        let head = git
            .rev_parse(&Revision::Branch(req.branch.clone()))
            .await
            .map_err(|e| OwnerError::Publish(e.to_string()))?;
        let (_path, _branch) = self
            .make_worktree(&git, &lead, &agent, &task, &head)
            .await
            .map_err(|e| OwnerError::Publish(e.to_string()))?;
        self.worktree_git(&lead.repo, &task, &agent)
            .map_err(|e| OwnerError::Publish(e.to_string()))?
            .detach(&head)
            .await
            .map_err(|e| OwnerError::Publish(e.to_string()))?;

        let member = Member::new(task.clone(), Participant::Investigator(chosen.clone())).with_role(Role::Investigator);
        let mut next = self.parts.state.clone();
        next.add_reviewer(team_id, member)
            .map_err(|e| OwnerError::Publish(e.to_string()))?;
        if let Err(error) = self
            .parts
            .bus
            .inv_set(req.id, &InvTransition::Running { task: task.clone() })
            .await
        {
            tracing::warn!(
                event = LogEvent::BusDown.as_str(),
                %team_id,
                %task,
                error = %error,
                "cannot mark investigation running on bus"
            );
            return Ok(());
        }
        next.record_activities(now);
        self.parts.state = next;

        tracing::info!(
            event = LogEvent::InvestigationStarted.as_str(),
            id = %req.id,
            investigator = %chosen,
            task = %task,
            from_task = %req.from_task,
        );
        self.look_at(team_id);
        Ok(())
    }
}
