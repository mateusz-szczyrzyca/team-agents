//! A run that reported `STATUS: FAILED` without calling a tool: its task is started once in a new session instead of
//! failing, because a run whose CLI did not offer — or whose model did not use — the tools of the bus deserves one clean
//! chance. A second such end fails the task as before, and a run that called a tool is never retried by this decision.

use crate::state::RunEnd;

/// What follows a run of a task that reported `STATUS: FAILED`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AfterFailedWithoutTool {
    /// The task is started once in a new session, briefed on the session before; nobody is told of the failure.
    Retry,
    /// The task fails and the human is told, as before.
    Fail,
}

/// Whether the end `end` of a run — the first run of its session, `used_a_tool` calls to a tool, never retried for this
/// before — sends its task to a new session or fails it.
///
/// A run is retried exactly when it reported `STATUS: FAILED`, called no tool step, opened its session, and the task
/// was not already retried for a failed run without a tool call: the durable "already retried" flag is what bounds the
/// retry to one even across a restart of the daemon.
#[must_use]
pub fn after_failed_without_tool(
    end: &RunEnd,
    used_a_tool: bool,
    first_of_session: bool,
    already_retried: bool,
) -> AfterFailedWithoutTool {
    // RED: the decision is implemented in the GREEN commit; a stub returns Fail so that only the new tests fail.
    let _ = (end, used_a_tool, first_of_session, already_retried);
    AfterFailedWithoutTool::Fail
}

#[cfg(test)]
mod tests;