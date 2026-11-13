//! `agent unpause`: unpause a task or a team.

use std::io::Write;

use agent_core::protocol::{Command, ControlRequest, Target};

use super::control::{ControlError, applied};
use crate::client::Connection;

/// Asks the daemon to unpause `target` and prints the tasks it reached.
///
/// # Errors
/// [`ControlError`].
pub(crate) async fn unpause(
    connection: &mut Connection,
    target: Target,
    out: &mut impl Write,
) -> Result<(), ControlError> {
    applied(connection, Command::Unpause(ControlRequest { target }), "Unpaused", out).await
}
