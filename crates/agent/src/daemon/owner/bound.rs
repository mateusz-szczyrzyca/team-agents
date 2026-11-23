//! The bound of a session: a session whose last run reached more tokens than its agent's `max_context` is not resumed.
//! The task is started anew before its next run, which is briefed from the checkpoint. [`starts_anew`] is the decision;
//! this module gathers what it takes, from the configuration and from the history.

use agent_core::config::{Config, context_bound};
use agent_core::decide::context::starts_anew;
use agent_core::ids::{RunId, TaskId};
use agent_core::log::LogEvent;
use agent_core::state::Participant;
use agent_runtime::history::{ReadError, Reader, log_decision};
use agent_runtime::process::error_chain;

use super::{Owner, OwnerError};

/// Why the size of a run's context cannot be read from the history.
#[derive(Debug, thiserror::Error)]
pub(super) enum SizeProblem {
    /// The history cannot be read.
    #[error("cannot read the usage of run {run} from the history")]
    Read {
        /// The run whose usage was asked for.
        run: RunId,
        /// What the history refused.
        #[source]
        source: Box<ReadError>,
    },
    /// The reader of the history cannot be joined.
    #[error("cannot join the reader of the usage of a run")]
    Join(#[source] tokio::task::JoinError),
}

/// The bound in tokens of the session of `participant`: its agent's own `max_context`, else the `[session]` one; `None`
/// when the bound is off.
pub(super) fn bound_of(config: &Config, participant: &Participant) -> Option<u64> {
    let section = config.session().max_context;
    let own = match participant {
        Participant::Normal(name) | Participant::Worker(name) => config.agent(name).and_then(|agent| agent.max_context),
        Participant::Investigator(name) => config.investigator(name).and_then(|agent| agent.max_context),
    };
    context_bound(own, section)
}

impl Owner {
    /// Starts `task` anew in a new session when the context its last run reached is above its agent's bound. Returns the
    /// size that ended the session when it did, so that the briefing of the new session can name it, and `None` when the
    /// session is resumed as before: a task with no session to resume, a bound that is off, a size the CLI did not tell,
    /// and a history that cannot be read, which is logged with its cause.
    ///
    /// # Errors
    /// [`OwnerError`] when the state refuses the new session.
    pub(super) async fn renew_for_context(&mut self, task: &TaskId) -> Result<Option<u64>, OwnerError> {
        let (run, bound) = {
            let found = self
                .parts
                .state
                .find_task(task)
                .ok_or_else(|| OwnerError::LostTask { task: task.clone() })?;
            let member = found.member;
            let (Some(last), Some(_)) = (member.runs().last(), member.last_session()) else {
                return Ok(None);
            };
            let Some(bound) = bound_of(&self.parts.settings.config, member.agent()) else {
                return Ok(None);
            };
            (last.id().clone(), bound)
        };
        let size = match self.context_of_run(&run).await {
            Ok(size) => size,
            Err(problem) => {
                tracing::warn!(
                    event = LogEvent::HistoryRecordIncomplete.as_str(),
                    task = %task,
                    run = %run,
                    error = %error_chain(&problem),
                    "the context of the last run cannot be read: its session is resumed"
                );
                return Ok(None);
            }
        };
        let Some(tokens) = size.filter(|tokens| starts_anew(Some(*tokens), Some(bound))) else {
            return Ok(None);
        };
        let at = self.after_last_run(task)?;
        let begun = self.parts.state.begin_context_session(task, at)?;
        log_decision(
            "context",
            &format!("task={task}, context={tokens}, bound={bound}"),
            &"NewSession",
        );
        Ok(begun.then_some(tokens))
    }

    /// The size of the context of the last model call of `run`, as the history recorded it; `None` when the run has no
    /// usage row, which is a size not told.
    ///
    /// # Errors
    /// [`SizeProblem`] when the history cannot be read.
    pub(super) async fn context_of_run(&self, run: &RunId) -> Result<Option<u64>, SizeProblem> {
        let path = self.parts.store.home().history_file();
        if !path.exists() {
            return Ok(None);
        }
        let wanted = run.clone();
        let read =
            tokio::task::spawn_blocking(move || Reader::open(&path).and_then(|reader| reader.run_usage(&wanted)))
                .await
                .map_err(SizeProblem::Join)?;
        match read {
            Ok(usage) => Ok(usage.and_then(|usage| usage.last_context)),
            Err(ReadError::Empty { .. }) => Ok(None),
            Err(source) => Err(SizeProblem::Read {
                run: run.clone(),
                source: Box::new(source),
            }),
        }
    }
}
