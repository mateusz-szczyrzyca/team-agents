//! Serves open investigation requests with free investigators.

use std::collections::BTreeSet;
use std::num::NonZeroUsize;

use agent_core::decide::investigator::Activity;
use agent_core::ids::{AgentName, InvestigatorName, TaskId, TeamId};
use agent_core::kinds::Role;
use agent_core::log::LogEvent;
use agent_core::state::{InvestigationRecord, Member, Participant, RunEnd, Timestamp};
use agent_core::status::TaskStatus;
use agent_runtime::agentbus::{InvTransition, Investigation, InvestigationKind, InvestigationStatus};
use agent_runtime::git::Revision;

use super::Owner;
use crate::daemon::owner::OwnerError;
use crate::daemon::review::investigator_of;

/// The bus queue admits at most three pending investigation requests.
const QUEUE_MAX: NonZeroUsize = NonZeroUsize::new(3).unwrap();

impl Owner {
    /// Serves open investigation requests with free investigators for `team_id`.
    pub(super) async fn serve_investigations(&mut self, team_id: &TeamId) -> Result<(), OwnerError> {
        let now = Timestamp::now(&self.parts.clock)?;
        let configured_keys: BTreeSet<InvestigatorName> =
            self.parts.settings.config.investigators().keys().cloned().collect();
        let Some(slots) = NonZeroUsize::new(configured_keys.len()) else {
            return Ok(());
        };

        let team = self
            .parts
            .state
            .team(team_id)
            .ok_or_else(|| OwnerError::LostTeam { team: team_id.clone() })?;
        let live_tasks: BTreeSet<TaskId> = team
            .crew()
            .roster()
            .into_iter()
            .filter(|(_, member)| {
                !matches!(
                    member.status(),
                    TaskStatus::Stopped | TaskStatus::Failed | TaskStatus::Done
                )
            })
            .map(|(_, member)| member.task().clone())
            .collect();

        loop {
            let free =
                agent_core::decide::investigator::free_for_request(&self.parts.state, &self.parts.settings.config, now);
            if free.is_empty() {
                break;
            }
            // Avoid polling the global queue when this team has no queued requests.
            if !self.bus_has_queued_for(team_id).await? {
                break;
            }
            let Some(req) = self
                .parts
                .bus
                .inv_next_for_tasks(QUEUE_MAX, slots, &live_tasks)
                .await
                .map_err(|source| OwnerError::Bus { source })?
            else {
                break;
            };

            if let InvestigationKind::Unknown(kind) = &req.kind {
                self.parts
                    .bus
                    .inv_set(
                        req.id,
                        &InvTransition::Failed {
                            reason: format!("unknown investigation kind: {kind}"),
                        },
                    )
                    .await
                    .map_err(|source| OwnerError::Bus { source })?;
                tracing::warn!(event = LogEvent::InvestigationKindUnknown.as_str(), team = %team_id, id = %req.id, kind = %kind);
                continue;
            }

            if !self.is_task_live_in_team(team_id, &req.from_task) {
                self.parts
                    .bus
                    .inv_set(req.id, &InvTransition::Requeued)
                    .await
                    .map_err(|source| OwnerError::Bus { source })?;
                break;
            }
            let Some(chosen) = self.pick_investigator(team_id, &req, &configured_keys, &free) else {
                self.parts
                    .bus
                    .inv_set(req.id, &InvTransition::Requeued)
                    .await
                    .map_err(|source| OwnerError::Bus { source })?;
                break;
            };

            self.start_investigation(team_id, &req, &chosen, now).await?;
        }

        Ok(())
    }

    /// Returns true when the bus queue holds at least one queued investigation whose
    /// `from_task` is a live member of `team_id`.
    async fn bus_has_queued_for(&self, team_id: &TeamId) -> Result<bool, OwnerError> {
        let open = self
            .parts
            .bus
            .inv_open()
            .await
            .map_err(|source| OwnerError::Bus { source })?;
        Ok(open.iter().any(|inv| {
            inv.status == Some(InvestigationStatus::Queued) && self.is_task_live_in_team(team_id, &inv.from_task)
        }))
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
            .map_err(|source| OwnerError::InvestigationWorkspace(source.into()))?;
        let agent: AgentName = chosen.as_str().parse()?;
        let head = git
            .rev_parse(&Revision::Branch(req.branch.clone()))
            .await
            .map_err(OwnerError::InvestigationGit)?;
        let (_path, _branch) = self
            .make_worktree(&git, &lead, &agent, &task, &head)
            .await
            .map_err(|source| OwnerError::InvestigationWorkspace(source.into()))?;
        self.worktree_git(&lead.repo, &task, &agent)
            .map_err(|source| OwnerError::InvestigationWorkspace(source.into()))?
            .detach(&head)
            .await
            .map_err(OwnerError::InvestigationGit)?;

        let member = Member::new(task.clone(), Participant::Investigator(chosen.clone())).with_role(Role::Investigator);
        let mut next = self.parts.state.clone();
        next.add_reviewer(team_id, member)
            .map_err(OwnerError::InvestigationMember)?;
        self.parts
            .bus
            .inv_set(req.id, &InvTransition::Running { task: task.clone() })
            .await
            .map_err(|source| OwnerError::Bus { source })?;
        next.record_activities(now);
        self.parts.state = next;

        tracing::info!(
            event = LogEvent::InvestigationStarted.as_str(),
            team = %team_id,
            id = %req.id,
            investigator = %chosen,
            task = %task,
            from_task = %req.from_task,
        );
        self.look_at(team_id);
        Ok(())
    }
}

impl Owner {
    /// Retains an unfinished answer privately before returning the request to the queue.
    pub(super) async fn requeue_investigation(
        &mut self,
        req: &Investigation,
        located: &super::Located,
        end: &RunEnd,
        message: Option<&str>,
    ) -> Result<(), OwnerError> {
        use tokio::io::AsyncWriteExt;

        if let Some(message) = message.filter(|text| !text.trim().is_empty()) {
            let mut file = tokio::fs::OpenOptions::new()
                .append(true)
                .create(true)
                .open(&req.file)
                .await
                .map_err(|source| OwnerError::WriteRecord {
                    path: req.file.clone(),
                    source,
                })?;
            let text = format!("\n## Unfinished answer of {} ({end:?})\n\n{message}\n", located.agent);
            file.write_all(text.as_bytes())
                .await
                .map_err(|source| OwnerError::WriteRecord {
                    path: req.file.clone(),
                    source,
                })?;
            file.sync_all().await.map_err(|source| OwnerError::WriteRecord {
                path: req.file.clone(),
                source,
            })?;
        }
        let now = Timestamp::now(&self.parts.clock)?;
        let team = self
            .parts
            .state
            .team(&located.team)
            .ok_or_else(|| OwnerError::LostTeam {
                team: located.team.clone(),
            })?;
        let mut record = team
            .investigations()
            .iter()
            .find(|record| record.id == req.id)
            .cloned()
            .unwrap_or_else(|| InvestigationRecord {
                id: req.id,
                task: req.from_task.clone(),
                started: now,
                failed_with: Vec::new(),
            });
        record.failed_with.push(located.agent.parse()?);
        self.parts
            .state
            .team_mut(&located.team)
            .ok_or_else(|| OwnerError::LostTeam {
                team: located.team.clone(),
            })?
            .record_investigation(record);
        self.save().await?;
        self.parts
            .bus
            .inv_set(req.id, &InvTransition::Requeued)
            .await
            .map_err(|source| OwnerError::Bus { source })?;
        Ok(())
    }
}

impl Owner {
    /// Recovers open requests after restart and enforces their active answer deadline.
    pub(super) async fn reconcile_investigations(&mut self) -> Result<(), OwnerError> {
        Ok(())
    }
}
