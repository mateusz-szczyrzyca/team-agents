//! `agent renew`: starts the next run of a task as a new session, briefed from its checkpoint.

use std::io::Write;

use agent_core::protocol::{Command, ControlRequest, Reply, Target};

use super::control::{CLIENT, ControlError, unexpected};
use crate::client::Connection;

/// Asks the daemon to renew `target` and prints the tasks it reached.
///
/// # Errors
/// [`ControlError`].
pub(crate) async fn renew(
    connection: &mut Connection,
    target: Target,
    out: &mut impl Write,
) -> Result<(), ControlError> {
    let reply = connection
        .request(CLIENT, Command::Renew(ControlRequest { target }))
        .await?;
    let Reply::Applied { tasks } = reply else {
        return Err(unexpected(&reply));
    };
    for task in tasks {
        writeln!(out, "Renewed {task}: its next run starts a new session").map_err(ControlError::Output)?;
    }
    out.flush().map_err(ControlError::Output)
}
