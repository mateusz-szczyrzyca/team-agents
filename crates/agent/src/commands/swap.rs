//! `agent swap`: swap the agent holding a task.

use std::io::Write;

use agent_core::ids::{AgentName, TaskId};

use super::control::ControlError;
use crate::client::Connection;

/// Asks the daemon to swap `task` to `agent`.
///
/// # Errors
/// [`ControlError`].
pub(crate) async fn swap(
    _connection: &mut Connection,
    _task: TaskId,
    _agent: AgentName,
    out: &mut impl Write,
) -> Result<(), ControlError> {
    writeln!(out, "stub").map_err(ControlError::Output)?;
    Ok(())
}
