//! The bound of a session: a session whose last run reached more tokens than its agent's `max_context` is not resumed.
//! The task is started anew before its next run, which is briefed from the checkpoint. [`starts_anew`] is the decision;
//! this module gathers what it takes, from the configuration and from the history.

use std::path::Path;

use agent_core::config::{Config, context_bound};
use agent_core::decide::context::starts_anew;
use agent_core::ids::{RunId, TaskId};
use agent_core::log::LogEvent;
use agent_core::state::Participant;
use agent_runtime::history::{ReadError, Reader, log_decision};
use agent_runtime::process::error_chain;
use agent_runtime::run::outcome::read_outcome;

use super::{Owner, OwnerError};

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

/// What a run's outcome says of the size of its session's context.
#[derive(Debug)]
pub(super) enum SizeRead {
    /// The run's last model call reached this many tokens.
    Told(u64),
    /// The run told no size: the CLI reported none.
    NotTold,
    /// The size cannot be read, and the cause is kept.
    Unreadable(String),
}

/// The size of the context of run `run`: its outcome file's when that tells one, else the history's. A file that cannot be
/// read does not decide alone: the history's size of the run does, and only when neither source tells is the run unreadable.
pub(super) async fn size_of_run(run_dir: &Path, history: &Path, run: &RunId) -> SizeRead {
    let file = read_outcome(run_dir).await;
    if let Ok(Some(outcome)) = &file {
        return outcome.last_context.map_or(SizeRead::NotTold, SizeRead::Told);
    }
    // A run with no outcome file ended before the file was written, and an unreadable file is judged by the history too.
    let from_history = size_from_history(history, run).await;
    match (file, from_history) {
        (Err(error), Ok(None)) => SizeRead::Unreadable(error_chain(&error)),
        (Err(error), Err(cause)) => SizeRead::Unreadable(format!("{}; {cause}", error_chain(&error))),
        (_, Ok(Some(tokens))) => SizeRead::Told(tokens),
        (_, Ok(None)) => SizeRead::NotTold,
        (Ok(_), Err(cause)) => SizeRead::Unreadable(cause),
    }
}

/// The size of the context of `run` as the history records it: `None` when it has no usage row or there is no history.
///
/// # Errors
/// The cause, as text, when the history cannot be read.
async fn size_from_history(history: &Path, run: &RunId) -> Result<Option<u64>, String> {
    if !history.exists() {
        return Ok(None);
    }
    let (path, wanted) = (history.to_path_buf(), run.clone());
    let read =
        tokio::task::spawn_blocking(move || Reader::open(&path).and_then(|reader| reader.run_usage(&wanted))).await;
    match read {
        Err(join) => Err(error_chain(&join)),
        Ok(Err(ReadError::Empty { .. })) => Ok(None),
        Ok(Err(source)) => Err(error_chain(&source)),
        Ok(Ok(usage)) => Ok(usage.and_then(|usage| usage.last_context)),
    }
}

#[cfg(test)]
mod tests;

impl Owner {
    /// Starts `task` anew in a new session when the context its last run reached is above its agent's bound. Returns the
    /// size that ended the session when it did, so that the briefing of the new session can name it, and `None` when the
    /// session is resumed as before: a task with no session to resume, a bound that is off, a size the CLI did not tell,
    /// and a history that cannot be read, which is logged with its cause.
    ///
    /// # Errors
    /// [`OwnerError`] when the state refuses the new session.
    pub(super) async fn renew_for_context(&mut self, task: &TaskId) -> Result<bool, OwnerError> {
        let (run, bound) = {
            let found = self
                .parts
                .state
                .find_task(task)
                .ok_or_else(|| OwnerError::LostTask { task: task.clone() })?;
            let member = found.member;
            let (Some(last), Some(_)) = (member.runs().last(), member.last_session()) else {
                return Ok(false);
            };
            let Some(bound) = bound_of(&self.parts.settings.config, member.agent()) else {
                return Ok(false);
            };
            (last.id().clone(), bound)
        };
        let run_dir = self.parts.store.home().run_dir(task, &run);
        let history = self.parts.store.home().history_file();
        let read = size_of_run(run_dir.as_path(), history.as_path(), &run).await;
        let at = self.after_last_run(task)?;
        match read {
            SizeRead::NotTold => Ok(false),
            // A size that cannot be read may be above the bound: the session is replaced on a fail-safe basis, and its briefing
            // says the size is unknown.
            SizeRead::Unreadable(cause) => {
                tracing::warn!(
                    event = LogEvent::HistoryRecordIncomplete.as_str(),
                    task = %task,
                    run = %run,
                    error = %cause,
                    "the context of the last run cannot be read: its session is replaced, its size unknown"
                );
                let begun = self.parts.state.begin_context_session(task, at)?;
                log_decision(
                    "context",
                    &format!("task={task}, context=unknown, bound={bound}"),
                    &"NewSession",
                );
                Ok(begun)
            }
            SizeRead::Told(tokens) if !starts_anew(Some(tokens), Some(bound)) => Ok(false),
            SizeRead::Told(tokens) => {
                let begun = self.parts.state.begin_context_session_with_size(task, at, tokens)?;
                log_decision(
                    "context",
                    &format!("task={task}, context={tokens}, bound={bound}"),
                    &"NewSession",
                );
                Ok(begun)
            }
        }
    }
}
