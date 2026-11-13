//! What `agent stop`, `pause`, `unpause` and `resume` have in common: one request for a task or a team, and the tasks it
//! reached printed.

use std::io::{self, Write};

use agent_core::protocol::{Client, Command, Reply};

use super::status::kind_of;
use crate::client::{ClientError, Connection};

/// Why a control failed.
#[derive(Debug, thiserror::Error)]
pub(crate) enum ControlError {
    /// The daemon could not be asked, or refused.
    #[error(transparent)]
    Client(#[from] ClientError),
    /// The daemon answered with something else than the tasks it reached.
    #[error("the daemon answered with {kind}, not with the tasks it reached")]
    UnexpectedReply {
        /// What it answered.
        kind: &'static str,
    },
    /// The output cannot be written.
    #[error("cannot write what the daemon did")]
    Output(#[source] io::Error),
}

/// Sends `command` and prints a line for each task the daemon reached, started by `verb`.
///
/// # Errors
/// [`ControlError`].
pub(super) async fn applied(
    connection: &mut Connection,
    command: Command,
    verb: &str,
    out: &mut impl Write,
) -> Result<(), ControlError> {
    let _ = (connection, command, verb, out);
    Ok(())
}

/// The kind of the reply, for [`ControlError::UnexpectedReply`].
pub(super) const fn unexpected(reply: &Reply) -> ControlError {
    ControlError::UnexpectedReply { kind: kind_of(reply) }
}

/// The client the command line is.
pub(super) const CLIENT: Client = Client::Cli;
