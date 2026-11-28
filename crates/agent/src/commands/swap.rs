//! `agent swap`: swap the agent holding a task.

use std::io::Write;

use agent_core::ids::{AgentName, TaskId};

use super::control::ControlError;
use crate::client::Connection;

#[cfg(test)]
mod tests;

/// Asks the daemon to swap `task` to `agent`.
///
/// # Errors
/// [`ControlError`].
pub(crate) async fn swap(
    connection: &mut Connection,
    task: TaskId,
    agent: AgentName,
    out: &mut impl Write,
) -> Result<(), ControlError> {
    let _request = (connection, task, agent);
    writeln!(out, "swap stub").map_err(ControlError::Output)?;
    out.flush().map_err(ControlError::Output)
}
