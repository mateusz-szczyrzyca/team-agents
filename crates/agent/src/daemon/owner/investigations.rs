//! Serves open investigation requests with free investigators.

use std::collections::BTreeSet;
use std::num::NonZeroUsize;

use agent_core::decide::investigator::Activity;
use agent_core::history::{CheckpointCause, CheckpointJob, investigation_meant};
use agent_core::ids::{AgentName, InvestigatorName, ItemId, TaskId, TeamId};
use agent_core::kinds::{Role, Tier};
use agent_core::log::LogEvent;
use agent_core::state::{InvestigationRecord, ItemPhase, Member, Participant, PullRequest, RunEnd, Team, Timestamp};
use agent_core::status::TaskStatus;
use agent_runtime::agentbus::{InvTransition, Investigation, InvestigationKind, InvestigationStatus};
use agent_runtime::git::Revision;

use super::Owner;
use super::job_checkpoint::JobFacts;
use crate::daemon::owner::OwnerError;
use crate::daemon::review::investigator_of;

/// The bus queue admits at most three pending investigation requests.
const QUEUE_MAX: NonZeroUsize = NonZeroUsize::new(3).unwrap();

/// Returns the item's open pull request, if its phase carries one.
pub(super) fn conflict_pull_request(team: &Team, item: ItemId) -> Option<&PullRequest> {
    match team.item(item)?.phase() {
        ItemPhase::InReview { pr, .. } | ItemPhase::Fixing { pr, .. } | ItemPhase::Approved { pr, .. } => Some(pr),
        ItemPhase::Pending | ItemPhase::InProgress { .. } | ItemPhase::Done { .. } => None,
    }
}

impl Owner {
    /// Serves open investigation requests with free investigators for `team_id`.
    pub(super) async fn serve_investigations(&mut self, team_id: &TeamId) -> Result<(), OwnerError> {
        let now = Timestamp::now(&self.parts.clock)?;
        let configured_keys: BTreeSet<InvestigatorName> =
            self.parts.settings.config.investigators().keys().cloned().collect();
        let Some(slots) = NonZeroUsize::new(configured_keys.len()) else {
            return Ok(());
        };

        self.reconcile_investigations().await?;
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

            self.start_investigation(team_id, &req, &chosen).await?;
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

        if let Some(record) = self
            .parts
            .state
            .team(team_id)
            .and_then(|team| team.investigations().iter().find(|record| record.id == req.id))
        {
            excluded.extend(record.failed_with.iter().cloned());
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
    ) -> Result<(), OwnerError> {
        let task = TaskId::generate(&self.parts.clock, &mut self.parts.rng)?;
        let lead = self.lead_of(team_id)?;
        let git = self
            .worktree_git(&lead.repo, &lead.task, &lead.agent)
            .map_err(|source| OwnerError::InvestigationWorkspace(source.into()))?;
        let agent: AgentName = chosen.as_str().parse()?;
        let revision = if let InvestigationKind::Conflict { item } = req.kind {
            let branch = self
                .parts
                .state
                .team(team_id)
                .and_then(|team| conflict_pull_request(team, item))
                .map(|pr| pr.branch.clone());
            let Some(branch) = branch else {
                let requester = self.locate(&req.from_task)?;
                self.fail_investigation(req, "the item has no open pull request", Some(&requester))
                    .await?;
                return Ok(());
            };
            git.fetch(&branch).await.map_err(OwnerError::InvestigationGit)?;
            Revision::RemoteBranch(branch)
        } else {
            Revision::Branch(req.branch.clone())
        };
        let head = git.rev_parse(&revision).await.map_err(OwnerError::InvestigationGit)?;
        let (_path, _branch) = self
            .make_worktree(&git, &lead, &agent, &task, &head)
            .await
            .map_err(|source| OwnerError::InvestigationWorkspace(source.into()))?;
        self.worktree_git(&lead.repo, &task, &agent)
            .map_err(|source| OwnerError::InvestigationWorkspace(source.into()))?
            .detach(&head)
            .await
            .map_err(OwnerError::InvestigationGit)?;

        let now = Timestamp::now(&self.parts.clock)?;
        let member = Member::new(task.clone(), Participant::Investigator(chosen.clone())).with_role(Role::Investigator);
        let mut next = self.parts.state.clone();
        next.add_reviewer(team_id, member)
            .map_err(OwnerError::InvestigationMember)?;
        let failed_with = next
            .team(team_id)
            .and_then(|team| team.investigations().iter().find(|record| record.id == req.id))
            .map(|record| record.failed_with.clone())
            .unwrap_or_default();
        next.team_mut(team_id)
            .ok_or_else(|| OwnerError::LostTeam { team: team_id.clone() })?
            .record_investigation(InvestigationRecord {
                id: req.id,
                task: task.clone(),
                started: now,
                failed_with,
            });
        if matches!(req.kind, InvestigationKind::Split { .. }) {
            next.team_mut(team_id)
                .ok_or_else(|| OwnerError::LostTeam { team: team_id.clone() })?
                .record_task_allocation(chosen.clone(), task.clone(), now);
        }
        self.parts.state = next;
        self.save().await?;
        self.parts
            .bus
            .inv_set(req.id, &InvTransition::Running { task: task.clone() })
            .await
            .map_err(|source| OwnerError::Bus { source })?;
        self.parts.state.record_activities(now);

        tracing::info!(
            event = LogEvent::InvestigationStarted.as_str(),
            team = %team_id,
            id = %req.id,
            investigator = %chosen,
            task = %task,
            from_task = %req.from_task,
        );
        if let Some((_, round)) = self.checkpoint_round_of(&req.from_task) {
            let facts = JobFacts {
                team: team_id.clone(),
                round,
                job: CheckpointJob::Investigation,
                tier: Tier::Investigator,
                task: task.clone(),
                worktree: self.parts.store.home().worktree_dir(&lead.repo, &task),
                meant: investigation_meant(&req.question),
                start: head,
                pull_request: None,
                review_pass: None,
                findings_file: None,
                findings_seen: String::new(),
                last_check: None,
            };
            self.job_started(facts, CheckpointCause::InvestigationStarted).await;
        }
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

        let reason = investigation_end_reason(end);

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
            let text = format!("\n## Unfinished answer of {} ({reason})\n\n{message}\n", located.agent);
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
                task: req.inv_task.clone().unwrap_or_else(|| req.from_task.clone()),
                started: now,
                failed_with: Vec::new(),
            });
        if matches!(end, RunEnd::Stopped) {
            return self
                .fail_investigation(req, "stopped by the human", Some(located))
                .await;
        }
        if !record.failed_with.is_empty() {
            return self.fail_investigation(req, &reason, Some(located)).await;
        }
        record.failed_with.push(located.agent.parse()?);
        let requester = self
            .parts
            .state
            .find_task(&req.from_task)
            .and_then(|found| investigator_of(found.member));
        let can_retry = self
            .parts
            .settings
            .config
            .investigators()
            .keys()
            .any(|candidate| !record.failed_with.contains(candidate) && requester != Some(candidate));
        if !can_retry {
            return self
                .fail_investigation(
                    req,
                    "no other configured investigator can answer this request",
                    Some(located),
                )
                .await;
        }
        let task = record.task.clone();
        self.parts
            .state
            .team_mut(&located.team)
            .ok_or_else(|| OwnerError::LostTeam {
                team: located.team.clone(),
            })?
            .record_investigation(record);
        self.close_investigation_task(&task).await?;
        self.save().await?;
        self.parts
            .bus
            .inv_set(req.id, &InvTransition::Requeued)
            .await
            .map_err(|source| OwnerError::Bus { source })?;
        tracing::info!(event = LogEvent::InvestigationRequeued.as_str(), team = %located.team, id = %req.id, investigator = %located.agent, reason = %reason);
        Ok(())
    }
}

impl Owner {
    /// Whether an investigation still has a live requester in a team.
    pub(super) fn is_investigation_requester_live(&self, req: &Investigation) -> bool {
        self.parts.state.find_task(&req.from_task).is_some_and(|found| {
            !matches!(
                found.member.status(),
                TaskStatus::Done | TaskStatus::Failed | TaskStatus::Stopped
            )
        })
    }

    /// Closes an attempt without letting a provider refusal enter the general retry ladder.
    async fn close_investigation_task(&mut self, task: &TaskId) -> Result<(), OwnerError> {
        let Some(found) = self.parts.state.find_task(task) else {
            return Ok(());
        };
        let status = found.member.status();
        let located = self.locate(task)?;
        if self.cancels.contains_key(task) {
            self.halt(task, super::Halt::Stop);
        }
        if !matches!(status, TaskStatus::Done | TaskStatus::Failed | TaskStatus::Stopped) {
            let mut events = Vec::new();
            let closed = if status == TaskStatus::Running {
                TaskStatus::Failed
            } else {
                TaskStatus::Stopped
            };
            self.move_to(task, &located.team, closed, &mut events)?;
            let now = Timestamp::now(&self.parts.clock)?;
            self.commit(task, &located, now, events, None).await?;
        }
        Ok(())
    }

    /// Ends a request and optionally tells its live requester what to do next.
    pub(super) async fn fail_investigation(
        &mut self,
        req: &Investigation,
        reason: &str,
        located: Option<&super::Located>,
    ) -> Result<(), OwnerError> {
        let reason = located
            .and_then(|located| self.parts.state.team(&located.team))
            .and_then(|team| team.investigation_failure(req.id))
            .unwrap_or(reason)
            .to_owned();
        if let Some(located) = located {
            self.parts
                .state
                .team_mut(&located.team)
                .ok_or_else(|| OwnerError::LostTeam {
                    team: located.team.clone(),
                })?
                .record_investigation_failure(req.id, reason.clone());
            self.save().await?;
        }
        if let Some(task) = &req.inv_task {
            self.close_investigation_task(task).await?;
        }
        if self.is_investigation_requester_live(req)
            && let Some(located) = located
        {
            let values = super::Variables::new()
                .with("investigation_id", req.id.to_string())?
                .with("reason", reason.as_str())?;
            let text = self
                .parts
                .team_texts
                .render(agent_core::prompt::TeamPrompt::InvestigationFailed, &values)?;
            let from = located.agent.parse()?;
            let to = super::Recipient::Task {
                agent: req.from_agent.clone(),
                task: req.from_task.clone(),
            };
            self.parts
                .bus
                .send(&from, &to, &super::Outgoing::Chat { text })
                .await
                .map_err(|source| OwnerError::Bus { source })?;
        }
        self.parts
            .bus
            .inv_set(req.id, &InvTransition::Failed { reason: reason.clone() })
            .await
            .map_err(|source| OwnerError::Bus { source })?;
        if let Some(located) = located {
            tracing::info!(event = LogEvent::InvestigationFailed.as_str(), team = %located.team, id = %req.id, reason = %reason);
        } else {
            tracing::info!(event = LogEvent::InvestigationFailed.as_str(), id = %req.id, reason = %reason);
        }
        Ok(())
    }

    /// Recovers open requests after restart and enforces their active answer deadline.
    pub(super) async fn reconcile_investigations(&mut self) -> Result<(), OwnerError> {
        let open = self
            .parts
            .bus
            .inv_open()
            .await
            .map_err(|source| OwnerError::Bus { source })?;
        let now = Timestamp::now(&self.parts.clock)?;
        for req in open {
            if !self.is_investigation_requester_live(&req) {
                self.fail_investigation(&req, "the requester's task ended", None)
                    .await?;
                continue;
            }
            let pending = req
                .inv_task
                .as_ref()
                .and_then(|task| self.parts.state.find_task(task))
                .and_then(|found| found.team.investigation_failure(req.id))
                .map(str::to_owned);
            if let Some(reason) = pending {
                let located = req.inv_task.as_ref().map(|task| self.locate(task)).transpose()?;
                self.fail_investigation(&req, &reason, located.as_ref()).await?;
                continue;
            }
            if !matches!(
                req.status,
                Some(InvestigationStatus::Starting | InvestigationStatus::Running)
            ) {
                continue;
            }
            let found = req.inv_task.as_ref().and_then(|task| self.parts.state.find_task(task));
            let Some(found) = found else {
                self.parts
                    .bus
                    .inv_set(req.id, &InvTransition::Requeued)
                    .await
                    .map_err(|source| OwnerError::Bus { source })?;
                continue;
            };
            if matches!(found.member.status(), TaskStatus::Done | TaskStatus::Failed) {
                self.parts
                    .bus
                    .inv_set(req.id, &InvTransition::Requeued)
                    .await
                    .map_err(|source| OwnerError::Bus { source })?;
                continue;
            }
            let task = found.member.task().clone();
            let located = self.locate(&task)?;
            if found.member.status() == TaskStatus::Stopped {
                self.fail_investigation(&req, "stopped by the human", Some(&located))
                    .await?;
                continue;
            }
            let record = self
                .parts
                .state
                .team(&located.team)
                .and_then(|team| team.investigations().iter().find(|record| record.id == req.id));
            if let Some(record) = record {
                let minutes = self.parts.settings.config.investigations().answer_minutes();
                if now.as_utc() - record.started.as_utc() >= time::Duration::minutes(i64::from(minutes)) {
                    self.fail_investigation(&req, &format!("no answer in {minutes} minutes"), Some(&located))
                        .await?;
                }
            }
        }
        Ok(())
    }
}

/// Stable descriptions for private attempt headings, failure notices and logs.
fn investigation_end_reason(end: &RunEnd) -> String {
    match end {
        RunEnd::Reported { outcome } => format!("reported {}", outcome.as_str()),
        RunEnd::NoStatus { .. } => "no status".to_owned(),
        RunEnd::QuotaExhausted { .. } => "quota exhausted".to_owned(),
        RunEnd::RateLimited => "rate limited".to_owned(),
        RunEnd::Hung => "hung".to_owned(),
        RunEnd::TurnLimit => "turn limit".to_owned(),
        RunEnd::Stopped => "stopped by the human".to_owned(),
    }
}
