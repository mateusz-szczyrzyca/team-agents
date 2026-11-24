//! The second try of a request: a wrong result is given once more, to a free worker of another provider when there is one,
//! with the reason the first result did not serve; a second wrong result returns the step to the requester with both reasons.

use std::path::Path;

use agent_core::decide::worker_result::{Elsewhere, worker_for_second_try};
use agent_core::ids::{AgentName, TaskId, TeamId};
use agent_core::kinds::CliKind;
use agent_core::log::LogEvent;
use agent_runtime::agentbus::{DelegationRequest, DelegationStatus, DelegationUpdate};
use agent_runtime::process::error_chain;
use agent_runtime::worker_request::{Pending, Settlement, Try, read_try, write_try};

use super::super::{Owner, OwnerError};
use crate::daemon::team::{BusJob, BusNotice, NoticeRenderError, delegation_again};

/// A request that goes once more to a worker: what the worker of the second try is told and the requester is told.
#[derive(Debug, Clone)]
pub(in crate::daemon::owner) struct Again {
    /// The team.
    pub(in crate::daemon::owner) team: TeamId,
    /// The worker whose result was wrong.
    pub(in crate::daemon::owner) worker: AgentName,
    /// The provider of that worker: the second try goes to another provider when one is free.
    pub(in crate::daemon::owner) failed_cli: CliKind,
    /// The provider of that worker computed via `provider_of`.
    pub(in crate::daemon::owner) failed_provider: String,
    /// The try the request is on once it is given again, with why each earlier one was wrong.
    pub(in crate::daemon::owner) tried: Try,
    /// Why the result that was wrong did not serve, in the words of the requester's notice.
    pub(in crate::daemon::owner) reason: String,
}

/// The labels of the tries, in the words the requester reads.
const ORDINALS: [&str; 3] = ["first", "second", "third"];

/// The reasons of the earlier tries and of this one, each told with its try.
pub(super) fn reasons_by_try(tried: &Try, this: &[String]) -> Vec<String> {
    if tried.earlier.is_empty() {
        return this.to_vec();
    }
    let label = |index: usize| ORDINALS.get(index).copied().unwrap_or("later");
    tried
        .earlier
        .iter()
        .enumerate()
        .map(|(index, reason)| format!("the {} result: {reason}", label(index)))
        .chain(std::iter::once(format!(
            "the {} result: {}",
            label(tried.earlier.len()),
            this.join("; ")
        )))
        .collect()
}

impl Owner {
    /// Which worker, if any, the second try of a request of `pending` can go to: another provider's, else one of the same
    /// provider, else none.
    pub(super) async fn elsewhere_of(&self, pending: &Pending) -> Elsewhere {
        let workers = match self.workers_of(&pending.team) {
            Ok(Some(workers)) if !workers.in_order.is_empty() => Some(workers),
            Ok(_) => None,
            Err(error) => {
                tracing::warn!(
                    event = LogEvent::BusDown.as_str(),
                    team = %pending.team,
                    error = %error_chain(&error),
                    "cannot read workers of team for second try"
                );
                return Elsewhere::None;
            }
        };
        if let Some(workers) = workers {
            let listed = self.workers_with_cli(&workers.in_order);
            let failed = self.cli_of(&pending.worker);
            let as_refs: Vec<(&AgentName, CliKind)> = listed.iter().map(|(name, cli)| (name, *cli)).collect();
            match (
                failed,
                worker_for_second_try(&as_refs, failed.unwrap_or(CliKind::Claude), &workers.busy),
            ) {
                (_, None) => Elsewhere::None,
                (Some(failed), Some(name)) if listed.iter().any(|(listed, cli)| listed == name && *cli == failed) => {
                    Elsewhere::SameProvider
                }
                _ => Elsewhere::Free,
            }
        } else {
            let pool = self.parts.settings.config.worker_agents();
            if pool.is_empty() {
                return Elsewhere::None;
            }
            let failed_provider = self.provider_of_agent(&pending.worker);
            let run_limit = agent_core::decide::slot::limit_for(self.parts.settings.config.limits(), agent_core::kinds::Tier::Worker);
            let mut has_same = false;

            for name in pool {
                let Some(agent_conf) = self.parts.settings.config.agent(name) else {
                    continue;
                };
                if self.is_in_active_crew(name) {
                    continue;
                }
                let avail = self.worker_availability(name).await;
                let below_limit = match run_limit {
                    agent_core::decide::slot::RunLimit::Unlimited => true,
                    agent_core::decide::slot::RunLimit::AtMost(limit) => avail.runs < limit.get(),
                };
                if avail.busy || avail.unsettled || !below_limit {
                    continue;
                }
                let worker_provider = agent_core::decide::pool::provider_of(agent_conf.cli, agent_conf.model.as_ref());
                if worker_provider != failed_provider {
                    return Elsewhere::Free;
                }
                has_same = true;
            }
            if has_same {
                Elsewhere::SameProvider
            } else {
                Elsewhere::None
            }
        }
    }

    /// The provider of the agent `name` computed via `provider_of`.
    pub(in crate::daemon::owner) fn provider_of_agent(&self, name: &AgentName) -> String {
        self.parts
            .settings
            .config
            .agent(name)
            .map_or_else(
                || name.as_str().to_owned(),
                |agent| agent_core::decide::pool::provider_of(agent.cli, agent.model.as_ref()).to_owned(),
            )
    }

    /// The provider of the agent `name`, when the configuration has it.
    pub(in crate::daemon::owner) fn cli_of(&self, name: &AgentName) -> Option<CliKind> {
        self.parts.settings.config.agents().get(name).map(|agent| agent.cli)
    }

    /// The workers `in_order` with their providers; one the configuration no longer has is left out.
    fn workers_with_cli(&self, in_order: &[(AgentName, TaskId)]) -> Vec<(AgentName, CliKind)> {
        in_order
            .iter()
            .filter_map(|(name, _)| self.cli_of(name).map(|cli| (name.clone(), cli)))
            .collect()
    }

    /// The settlement of a result that is wrong for `reasons`: it goes once more to another worker when `give_again` says
    /// so, else it is returned to the requester with the reasons of every try.
    pub(super) fn wrong_settlement(&self, pending: &Pending, reasons: Vec<String>, give_again: bool) -> Settlement {
        if give_again {
            return Settlement::Again { reasons };
        }
        let task_dir = self.parts.store.home().task_dir(&pending.worker_task);
        match read_try(&task_dir, pending.request) {
            Ok(tried) => Settlement::Returned {
                reasons: reasons_by_try(&tried, &reasons),
            },
            Err(error) => {
                tracing::warn!(
                    event = LogEvent::WorkerRequestDamaged.as_str(),
                    request = %pending.request,
                    error = %error_chain(&error),
                    "the try of a request cannot be read: the step is returned with this result's reasons alone"
                );
                Settlement::Returned { reasons }
            }
        }
    }

    /// The try a result is judged as: what the file of its try says, the first when it has none or it cannot be read.
    pub(super) fn try_of(&self, pending: &Pending) -> Try {
        let task_dir = self.parts.store.home().task_dir(&pending.worker_task);
        read_try(&task_dir, pending.request).unwrap_or_else(|error| {
            tracing::warn!(
                event = LogEvent::WorkerRequestDamaged.as_str(),
                request = %pending.request,
                error = %error_chain(&error),
                "the try of a request cannot be read: it is judged as the first"
            );
            Try::first()
        })
    }

    /// Gives the request of `pending` back to the bus's queue for a second try, and remembers what the second worker and the
    /// requester are told when it is given: `Ok(true)` when the file of this try can go, because the request is no longer this
    /// worker's; `Ok(false)` while it waits in the queue.
    ///
    /// # Errors
    /// [`OwnerError`] never today; the stage's loop takes this error type.
    pub(super) async fn give_again(&mut self, pending: &Pending, reasons: &[String]) -> Result<bool, OwnerError> {
        let known = match self.parts.bus.helper_get(pending.request).await {
            Ok(known) => known,
            Err(error) => {
                tracing::warn!(
                    event = LogEvent::BusDown.as_str(),
                    request = %pending.request,
                    error = %error_chain(&error),
                    "the state of a request to be given again cannot be read now: it is tried again at the next look"
                );
                return Ok(false);
            }
        };
        match known.status {
            DelegationStatus::Queued => {}
            DelegationStatus::Running | DelegationStatus::Starting
                if super::is_running_for(&known, &pending.worker_task, &pending.worker) =>
            {
                let update = DelegationUpdate {
                    status: Some(DelegationStatus::Queued),
                    task: None,
                    summary: None,
                };
                if let Err(error) = self.parts.bus.helper_set(pending.request, &update).await {
                    tracing::warn!(
                        event = LogEvent::BusDown.as_str(),
                        request = %pending.request,
                        error = %error_chain(&error),
                        "a request cannot be queued for its second try now: it is tried again at the next look"
                    );
                    return Ok(false);
                }
            }
            // Another worker has it, or it was answered: the second try is given, or there is nothing to give, and what was
            // remembered for it is not wanted.
            _ => {
                self.again.remove(&pending.request);
                return Ok(true);
            }
        }
        let tried = self.try_of(pending);
        let mut earlier = tried.earlier.clone();
        let reason = reasons.join("; ");
        earlier.push(reason.clone());
        let failed_cli = self.cli_of(&pending.worker).unwrap_or(CliKind::Claude);
        let failed_provider = self.provider_of_agent(&pending.worker);
        self.again.insert(
            pending.request,
            Again {
                team: pending.team.clone(),
                worker: pending.worker.clone(),
                failed_cli,
                failed_provider,
                tried: Try {
                    number: tried.number + 1,
                    earlier,
                },
                reason,
            },
        );
        self.look_at(&pending.team);
        Ok(false)
    }

    /// The worker of another provider that a request given again goes to, when one is free; else one of the same provider; else
    /// none, and the request waits.
    pub(in crate::daemon::owner) fn second_try_pick(
        &self,
        team: &TeamId,
        in_order: &[(AgentName, TaskId)],
        busy: &[AgentName],
    ) -> Option<AgentName> {
        let wanted = self.again.values().find(|again| &again.team == team)?;
        let listed = self.workers_with_cli(in_order);
        let as_refs: Vec<(&AgentName, CliKind)> = listed.iter().map(|(name, cli)| (name, *cli)).collect();
        worker_for_second_try(&as_refs, wanted.failed_cli, busy).cloned()
    }

    /// The request as the worker of the second try is given it: with what the first result lacked told after its context.
    ///
    /// # Errors
    /// [`NoticeRenderError`] when the text cannot be rendered.
    pub(super) fn request_again(
        &self,
        request: &DelegationRequest,
        again: &Again,
    ) -> Result<DelegationRequest, NoticeRenderError> {
        let told = delegation_again(&self.parts.team_texts, &again.worker, &again.reason)?;
        let mut changed = request.clone();
        changed.context = format!("{}\n\n{}", request.context.trim_end(), told.trim_end());
        Ok(changed)
    }

    /// Removes the file of the try of `request` from `task_dir`; one that is not there needs nothing.
    pub(in crate::daemon::owner) fn drop_try(task_dir: &Path, request: agent_runtime::agentbus::RequestId) {
        let path = agent_runtime::worker_request::try_path(task_dir, request);
        if let Err(error) = std::fs::remove_file(&path)
            && error.kind() != std::io::ErrorKind::NotFound
        {
            tracing::warn!(
                event = LogEvent::WorkerRequestDamaged.as_str(),
                path = ?path,
                error = %error,
                "the file of a try cannot be removed"
            );
        }
    }

    /// The request as the worker `worker` of `task` is given it: a request given again carries what the first worker's
    /// result lacked, the try is kept in the worker's task directory, and the requester is told it went once more.
    pub(in crate::daemon::owner) fn given_again(
        &mut self,
        team: &TeamId,
        request: &DelegationRequest,
        worker: &AgentName,
        task: &TaskId,
    ) -> DelegationRequest {
        let Some(again) = self.again.remove(&request.id) else {
            return request.clone();
        };
        let task_dir = self.parts.store.home().task_dir(task);
        if let Err(error) = write_try(&task_dir, request.id, &again.tried) {
            tracing::warn!(
                event = LogEvent::WorkerRequestDamaged.as_str(),
                request = %request.id,
                error = %error_chain(&error),
                "the try of a request cannot be kept: the result is judged as a first try"
            );
        }
        self.tell_bus(
            team,
            BusJob::Tell {
                to: request.from.clone(),
                notice: BusNotice::HelperGivenAgain {
                    request: request.id,
                    worker: again.worker.clone(),
                    next: worker.clone(),
                    reason: again.reason.clone(),
                },
            },
        );
        match self.request_again(request, &again) {
            Ok(given) => given,
            Err(error) => {
                tracing::warn!(
                    event = LogEvent::WorkerRequestDamaged.as_str(),
                    request = %request.id,
                    error = %error_chain(&error),
                    "what the first result lacked cannot be worded for the second worker: it is given the request as it is"
                );
                request.clone()
            }
        }
    }
}
