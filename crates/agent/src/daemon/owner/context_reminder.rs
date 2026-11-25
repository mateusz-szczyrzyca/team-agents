//! The reminder of a full context: a session whose context reached `[commit_reminder] context_percent` of its bound is told
//! once, on the bus, to commit what is finished and to write down where it is, before the next run may start a new session.
//!
//! The engine writes the context of the run's latest model call to `tasks/<task>/context.json` while the run goes on
//! (`agent_runtime::run::context`); the daemon reads it at each look at the team. What it keeps between two looks is in
//! memory only: a restart of the daemon re-arms the reminder.

use agent_core::cli::SessionId;
use agent_core::ids::{RunId, TaskId};
use agent_runtime::run::context::ContextRecord;

use super::Owner;

/// What a session is told by: its ID, or, while the CLI names none, the run that holds it.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Told {
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

/// The context to put in the reminder of the run `run`, when `record` calls for one: it is the record of that run, the
/// context reached `percent` of `bound`, and the session was not `told` yet.
fn context_to_tell(
    record: &ContextRecord,
    run: &RunId,
    bound: Option<u64>,
    percent: u8,
    told: Option<&Told>,
) -> Option<u64> {
    let _ = (record, run, bound, percent, told);
    None
}

impl Owner {
    /// Reminds the run of `task` once per session when the context of its session reaches the share of its bound that
    /// `[commit_reminder] context_percent` names. A look that fails is logged and the run goes on.
    pub(super) async fn remind_of_context(&mut self, task: &TaskId) {
        let _ = task;
    }
}

#[cfg(test)]
pub(super) mod tests;
