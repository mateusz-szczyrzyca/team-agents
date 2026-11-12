//! `agent status`: the teams the daemon knows, and what each of their tasks is doing.

use std::io::{self, Write};

use agent_core::protocol::{Client, Command, Reply, StatusRequest, Target, TeamSummary};

use crate::client::{ClientError, Connection};

/// Why `agent status` failed.
#[derive(Debug, thiserror::Error)]
pub(crate) enum StatusError {
    /// The daemon could not be asked.
    #[error("cannot ask the daemon for the status")]
    Client(#[from] ClientError),
    /// The daemon answered with something else than a list of teams.
    #[error("the daemon answered a status request with {kind}")]
    UnexpectedReply {
        /// What it answered.
        kind: &'static str,
    },
    /// The output cannot be written.
    #[error("cannot write the status")]
    Output(#[source] io::Error),
}

/// Asks the daemon for the status of `target` (every team without one) and writes it to `out`.
///
/// # Errors
/// [`StatusError`].
pub(crate) async fn status(
    connection: &mut Connection,
    target: Option<Target>,
    out: &mut impl Write,
) -> Result<(), StatusError> {
    let _ = (connection, target, out);
    Ok(())
}

/// The teams as text: a line for each team, and under it a line for each of its tasks.
pub(crate) fn render(teams: &[TeamSummary]) -> String {
    let _ = teams;
    String::new()
}

#[cfg(test)]
mod tests;
