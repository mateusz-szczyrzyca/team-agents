//! The reminder of a full context: a session whose context reached `[commit_reminder] context_percent` of its bound is told
//! once, on the bus, to commit what is finished and to write down where it is, before the next run may start a new session.
//!
//! The engine writes the context of the run's latest model call to `tasks/<task>/context.json` while the run goes on
//! (`agent_runtime::run::context`); the daemon reads it at each look at the team. What it keeps between two looks is in
//! memory only: a restart of the daemon re-arms the reminder.

use agent_core::cli::SessionId;
use agent_core::decide::context_reminder::context_due;
use agent_core::ids::{RunId, TaskId};
use agent_core::log::LogEvent;
use agent_core::prompt::{TeamPrompt, Variables, VariablesError};
use agent_runtime::agentbus::Outgoing;
use agent_runtime::process::error_chain;
use agent_runtime::run::context::{ContextError, ContextRecord, read_context};
use agent_runtime::team_prompts::RenderTeamTextError;

use super::bound::bound_of;
use super::commit_reminder::{Watched, watched_run};
use super::{Owner, OwnerError};

/// What a session is told by: its ID, or, while the CLI names none, the run that holds it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Told {
    /// A session the CLI named: a resumed session keeps its ID, so it is not told again, and a new session is told anew.
    Session(SessionId),
    /// A run whose CLI named no session.
    Run(RunId),
}

impl Told {
    /// What the session of `record` is told by.
    fn of(record: &ContextRecord) -> Self {
        match &record.session {
            Some(session) => Self::Session(session.clone()),
            None => Self::Run(record.run.clone()),
        }
    }
}

/// Why a look at the context of a session came to nothing.
#[derive(Debug, thiserror::Error)]
enum ContextLookError {
    /// The context file of the task cannot be read.
    #[error("the context file of the task cannot be read")]
    Read {
        /// What the file system or the JSON reader refused.
        source: ContextError,
    },
    /// The values of the text of the reminder are refused.
    #[error("the values of the reminder of a full context are refused")]
    Values {
        /// What refused them.
        source: VariablesError,
    },
    /// The loader of the texts cannot render the reminder.
    #[error("the reminder of a full context cannot be rendered")]
    Render {
        /// What the loader refused.
        source: RenderTeamTextError,
    },
    /// The task has no recipient on the bus.
    #[error("the reminder of a full context has no recipient")]
    Recipient {
        /// Why the task has none.
        source: OwnerError,
    },
    /// The bus refused the reminder.
    #[error("the reminder of a full context cannot be put on the bus")]
    Bus {
        /// What the bus said.
        source: OwnerError,
    },
}

impl ContextLookError {
    /// The log event the failure is logged under: a bus that is down, or a task that failed at its reminder.
    const fn event(&self) -> LogEvent {
        match self {
            Self::Bus { .. } => LogEvent::BusDown,
            Self::Read { .. } | Self::Values { .. } | Self::Render { .. } | Self::Recipient { .. } => {
                LogEvent::DaemonTaskFailed
            }
        }
    }
}

/// The context to put in the reminder of the run `run`, when `record` calls for one: it is the record of that run, the
/// context reached `percent` of `bound`, and the session was not `told` yet.
fn context_to_tell(
    record: &ContextRecord,
    run: &RunId,
    bound: Option<u64>,
    percent: u8,
    told: Option<&Told>,
) -> Option<u64> {
    if record.run != *run {
        return None;
    }
    let already_told = told == Some(&Told::of(record));
    context_due(record.context, bound, percent, already_told).then_some(record.context)
}

impl Owner {
    /// Reminds the run of `task` once per session when the context of its session reaches the share of its bound that
    /// `[commit_reminder] context_percent` names. A look that fails is logged and the run goes on.
    pub(super) async fn remind_of_context(&mut self, task: &TaskId) {
        let percent = self.parts.settings.config.commit_reminder().context_percent;
        if percent == 0 {
            return;
        }
        let target = match watched_run(&self.parts.state, task) {
            Ok(Some(target)) => target,
            Ok(None) => return,
            Err(error) => {
                tracing::warn!(
                    event = LogEvent::DaemonTaskFailed.as_str(),
                    task = %task,
                    operation = "context_reminder",
                    error = %error_chain(&error),
                    "the agent of a task has no name the bus knows"
                );
                return;
            }
        };
        if let Err(error) = self.remind_one_of_context(&target, percent).await {
            tracing::warn!(
                event = error.event().as_str(),
                task = %target.task,
                run = %target.run,
                operation = "context_reminder",
                error = %error_chain(&error),
                "the session was not looked at for the reminder of a full context"
            );
        }
    }

    /// Reads the context file of one task and reminds its run when the session is at `percent` of its bound.
    ///
    /// # Errors
    /// [`ContextLookError`] when the file cannot be read, or the reminder cannot be made or put on the bus.
    async fn remind_one_of_context(&mut self, target: &Watched, percent: u8) -> Result<(), ContextLookError> {
        let Some(bound) = self
            .parts
            .state
            .find_task(&target.task)
            .and_then(|found| bound_of(&self.parts.settings.config, found.member.agent()))
        else {
            return Ok(());
        };
        let dir = self.parts.store.home().task_dir(&target.task);
        let Some(record) = read_context(&dir)
            .await
            .map_err(|source| ContextLookError::Read { source })?
        else {
            return Ok(());
        };
        let Some(context) = context_to_tell(
            &record,
            &target.run,
            Some(bound),
            percent,
            self.context_told.get(&target.task),
        ) else {
            return Ok(());
        };
        self.put_context_reminder(target, context, bound, percent).await?;
        self.context_told.insert(target.task.clone(), Told::of(&record));
        tracing::info!(
            event = LogEvent::ContextReminded.as_str(),
            task = %target.task,
            run = %target.run,
            agent = %target.agent,
            context,
            bound,
            "a session near the bound of its context was reminded to commit and to write down where it is"
        );
        Ok(())
    }

    /// Puts the reminder for a session of `context` tokens at `percent` of its `bound` on the bus for the run.
    async fn put_context_reminder(
        &self,
        target: &Watched,
        context: u64,
        bound: u64,
        percent: u8,
    ) -> Result<(), ContextLookError> {
        let values = Variables::new()
            .with("context", context.to_string())
            .and_then(|values| values.with("bound", bound.to_string()))
            .and_then(|values| values.with("percent", percent.to_string()))
            .map_err(|source| ContextLookError::Values { source })?;
        let text = self
            .parts
            .team_texts
            .render(TeamPrompt::ContextRemind, &values)
            .map_err(|source| ContextLookError::Render { source })?;
        let recipient = self
            .recipient_of(&target.task)
            .map_err(|source| ContextLookError::Recipient { source })?;
        self.send_from_harness(&recipient, &Outgoing::Chat { text })
            .await
            .map_err(|source| ContextLookError::Bus { source })
    }
}

#[cfg(test)]
pub(super) mod tests;
