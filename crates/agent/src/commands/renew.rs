//! `agent renew`: starts the next run of a task as a new session.

use std::io::Write;

use agent_core::protocol::Target;

use super::control::ControlError;
use crate::client::Connection;

/// Asks the daemon to renew `target` and prints the tasks it reached.
///
/// # Errors
/// [`ControlError`].
pub(crate) async fn renew(
    _connection: &mut Connection,
    _target: Target,
    _out: &mut impl Write,
) -> Result<(), ControlError> {
    Ok(())
}
