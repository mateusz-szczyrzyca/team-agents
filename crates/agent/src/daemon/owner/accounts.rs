//! The pause of an account: the first run of one of its agents that the provider refuses pauses every task of every agent
//! of the account until the same moment, with one notification; the end of the pause queues them again.
//!
//! [`agent_core::decide::account_pause::decide`] is the rule; this module carries out its answer. An agent without an
//! account is judged by its own run alone, as before accounts.

use std::collections::BTreeSet;

use agent_core::config::AccountName;
use agent_core::decide::account_pause::{AccountPauseOutcome, decide};
use agent_core::decide::limit::Limit;
use agent_core::decide::outcome::PauseReason;
use agent_core::ids::TaskId;
use agent_core::kinds::Role;
use agent_core::log::LogEvent;
use agent_core::state::{AccountPause, RunEnd, Timestamp};
use agent_core::status::TaskStatus;

use super::{AccountPausedNotice, Located, Notice, Owner, OwnerError, limit_of};

/// What the pause of an account makes of the end of a refused run.
pub(super) enum AccountHandled {
    /// The agent has no account: the pause of the task is the one the run's own refusal says.
    NotAnAccount,
    /// The task is paused until `until`, as the account is.
    Paused {
        /// When the task goes on: the end of the pause of its account.
        until: Timestamp,
        /// What the human is told: once for the account, by the run that starts its pause.
        notice: Option<Notice>,
    },
}

impl Owner {
    /// The account of the agent or investigator `agent`; `None` for one without an account or one the configuration
    /// does not have any more.
    pub(super) fn account_of(&self, agent: &str) -> Option<AccountName> {
        let config = &self.parts.settings.config;
        let of_agent = config
            .agents()
            .iter()
            .find(|(name, _)| name.as_str() == agent)
            .map(|(_, found)| found.account.clone());
        of_agent.unwrap_or_else(|| {
            config
                .investigators()
                .iter()
                .find(|(name, _)| name.as_str() == agent)
                .and_then(|(_, found)| found.account.clone())
        })
    }

    /// Whether the account of `agent` is paused at `now`: none of its runs starts.
    pub(super) fn account_is_paused(&self, agent: &str, now: Timestamp) -> bool {
        self.account_of(agent)
            .and_then(|account| self.parts.state.account_pause(&account).map(|pause| now < pause.until))
            .unwrap_or(false)
    }

    /// The agents and investigators that have `account`, in name order.
    fn agents_of(&self, account: &AccountName) -> Vec<String> {
        let config = &self.parts.settings.config;
        let agents = config
            .agents()
            .iter()
            .filter(|(_, agent)| agent.account.as_ref() == Some(account))
            .map(|(name, _)| name.as_str().to_owned());
        let investigators = config
            .investigators()
            .iter()
            .filter(|(_, agent)| agent.account.as_ref() == Some(account))
            .map(|(name, _)| name.as_str().to_owned());
        agents
            .chain(investigators)
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect()
    }

    /// Carries out what the pause of the account of `located`'s agent makes of the refused run `end` of `task`.
    ///
    /// # Errors
    /// [`OwnerError`] when the end of the pause cannot be computed or a task cannot be paused or saved.
    pub(super) async fn pause_account(
        &mut self,
        task: &TaskId,
        located: &Located,
        end: &RunEnd,
        now: Timestamp,
    ) -> Result<AccountHandled, OwnerError> {
        let Some(limit) = limit_of(end, now) else {
            return Ok(AccountHandled::NotAnAccount);
        };
        let Some(account) = self.account_of(&located.agent) else {
            return Ok(AccountHandled::NotAnAccount);
        };
        let mut answer = self.account_answer(&limit, &account, now)?;
        if answer == AccountPauseOutcome::Expired {
            self.resume_account(&account);
            answer = self.account_answer(&limit, &account, now)?;
        }
        match answer {
            AccountPauseOutcome::NotAnAccount | AccountPauseOutcome::Expired => Ok(AccountHandled::NotAnAccount),
            AccountPauseOutcome::AlreadyPaused => {
                let until = self
                    .parts
                    .state
                    .account_pause(&account)
                    .map_or(now, |pause| pause.until);
                self.hold(task, until);
                Ok(AccountHandled::Paused { until, notice: None })
            }
            AccountPauseOutcome::Pause { until, notify } => {
                self.parts
                    .state
                    .record_account_pause(account.clone(), Some(AccountPause { until, reason: limit }));
                self.hold(task, until);
                let paused = 1 + self.pause_queued_tasks(task, &account, until, now).await?;
                let agents = self.agents_of(&account);
                let reason = reason_of(&limit);
                tracing::info!(
                    event = LogEvent::AccountPaused.as_str(),
                    account = %account,
                    until = %until,
                    reason = %reason,
                    agents = %agents.join(","),
                    tasks = paused,
                    "an account is paused: no run of its agents starts before its pause ends"
                );
                let notice = notify.then(|| {
                    Notice::AccountPaused(AccountPausedNotice {
                        account,
                        agents,
                        until,
                        reason: words_of(&limit),
                        tasks: paused,
                    })
                });
                Ok(AccountHandled::Paused { until, notice })
            }
        }
    }

    /// What the rule says of a refused run with `limit` of an agent of `account`.
    fn account_answer(
        &self,
        limit: &Limit,
        account: &AccountName,
        now: Timestamp,
    ) -> Result<AccountPauseOutcome, OwnerError> {
        Ok(decide(
            limit,
            self.parts.settings.config.limits(),
            Some(account),
            self.parts.state.account_pause(account),
            now,
        )?)
    }

    /// Records that the pause of the account holds `task`, which wakes at `until`.
    fn hold(&mut self, task: &TaskId, until: Timestamp) {
        self.parts.state.hold_task_for_account(task.clone());
        self.wakes.insert(task.clone(), until);
    }

    /// Pauses every task of the account that waits in the queue, except `refused`, which the caller pauses; returns
    /// how many it paused.
    async fn pause_queued_tasks(
        &mut self,
        refused: &TaskId,
        account: &AccountName,
        until: Timestamp,
        now: Timestamp,
    ) -> Result<usize, OwnerError> {
        let mut paused = 0;
        for item in self.parts.state.queued_tasks() {
            if item.task == *refused
                || self.in_flight.contains(&item.task)
                || self.account_of(item.agent.as_str()).as_ref() != Some(account)
            {
                continue;
            }
            let located = self.locate(&item.task)?;
            if matches!(located.role, Role::Worker | Role::Reviewer | Role::Investigator) {
                continue;
            }
            let mut events = Vec::new();
            self.move_to(&item.task, &located.team, TaskStatus::Paused, &mut events)?;
            self.hold(&item.task, until);
            self.commit(&item.task, &located, now, events, None).await?;
            paused += 1;
        }
        Ok(paused)
    }

    /// Forgets the pauses of the accounts that ended at `now` and says so once for each.
    pub(super) fn resume_accounts(&mut self, now: Timestamp) {
        let ended: Vec<AccountName> = self
            .parts
            .state
            .account_pauses()
            .filter(|(_, pause)| pause.until <= now)
            .map(|(account, _)| account.clone())
            .collect();
        for account in ended {
            self.resume_account(&account);
        }
    }

    /// Forgets the pause of `account`.
    fn resume_account(&mut self, account: &AccountName) {
        let tasks = self
            .parts
            .state
            .tasks_held_by_account()
            .filter(|task| {
                self.parts
                    .state
                    .find_task(task)
                    .is_some_and(|found| self.account_of(found.member.agent().name()).as_ref() == Some(account))
            })
            .count();
        self.parts.state.record_account_pause(account.clone(), None);
        tracing::info!(
            event = LogEvent::AccountResumed.as_str(),
            account = %account,
            tasks = tasks,
            "the pause of an account ended: its tasks are queued again"
        );
    }

    /// Wakes, after a restart, the tasks the pause of an account holds: at the end of that pause, at once when it is not
    /// recorded any more. A task that is not paused any more is not held.
    pub(super) fn wake_held_tasks(&mut self, now: Timestamp) {
        let held: Vec<TaskId> = self.parts.state.tasks_held_by_account().cloned().collect();
        for task in held {
            let Some(found) = self.parts.state.find_task(&task) else {
                self.parts.state.release_task_from_account(&task);
                continue;
            };
            if found.member.status() != TaskStatus::Paused {
                self.parts.state.release_task_from_account(&task);
                continue;
            }
            let until = self
                .account_of(found.member.agent().name())
                .and_then(|account| self.parts.state.account_pause(&account).map(|pause| pause.until))
                .unwrap_or(now);
            self.wakes.insert(task, until);
        }
    }
}

/// The limit as the log writes it.
fn reason_of(limit: &Limit) -> PauseReason {
    match limit {
        Limit::Quota { .. } => PauseReason::Quota,
        Limit::RateLimit => PauseReason::RateLimit,
    }
}

/// The limit as the notice says it.
const fn words_of(limit: &Limit) -> &'static str {
    match limit {
        Limit::Quota { .. } => "the quota is exhausted",
        Limit::RateLimit => "a rate limit",
    }
}
