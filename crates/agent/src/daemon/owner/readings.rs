//! The rate-limit readings that runs show: kept in the state with the account of the agent that ran, or with the agent
//! when it has no account, so that `agent status` shows how much of each window is used.

use agent_core::ids::TaskId;
use agent_core::log::LogEvent;
use agent_core::state::{ReadingOwner, Timestamp};

use super::{Owner, OwnerError};

impl Owner {
    /// Keeps the readings that the runs ended since the last call showed, and saves the state when there were any.
    ///
    /// # Errors
    /// [`OwnerError`] when the time cannot be read or the state cannot be saved.
    pub(super) async fn keep_run_readings(&mut self) -> Result<(), OwnerError> {
        let shown = self.parts.engine.take_readings();
        if shown.is_empty() {
            return Ok(());
        }
        let now = Timestamp::now(&self.parts.clock)?;
        for run in shown {
            let Some(owner) = self.reading_owner_of(&run.task) else {
                tracing::debug!(
                    event = LogEvent::DecisionTaken.as_str(),
                    decision = "reading",
                    task = %run.task,
                    result = "no_owner",
                    "the readings of a run are not kept: its task has no agent any more"
                );
                continue;
            };
            self.parts
                .state
                .keep_limit_readings(&owner, &run.readings, run.read_at, now);
        }
        self.update_holds(now).await;
        self.save().await
    }

    /// Whose limits the readings of the run of `task` tell of: the account of its agent, or the agent itself when it has
    /// none; `None` when no agent runs the task any more.
    fn reading_owner_of(&self, task: &TaskId) -> Option<ReadingOwner> {
        let agent = match self.parts.state.find_task(task) {
            Some(found) => found.member.agent().agent_name(),
            None => self.serving.get(task)?.worker.clone(),
        };
        Some(match self.account_of(&agent) {
            Some(account) => ReadingOwner::Account(account),
            None => ReadingOwner::Agent(agent),
        })
    }
}
