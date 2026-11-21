//! The line a follow prints after a run ended: what the round of the run waits for next, as `agent status` tells it.

#[cfg(test)]
mod tests;

use std::path::Path;

use agent_core::ids::RoundId;
use agent_core::local_time::Zone;
use agent_core::protocol::{Client, Command, Reply, ShowRequest, Target};
use agent_runtime::process::error_chain;

use crate::client::Connection;
use crate::commands::status::kind_of;

/// What the daemon said of a round, or why it could not be asked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Asked {
    /// The round waits for this; the text of the daemon's wait line.
    Waits(String),
    /// The round does not wait for anything: another run of it is going, or it is closed.
    Nothing,
    /// The daemon could not say, for this reason.
    Unknown(String),
}

/// The line that tells `asked` of `round`.
#[cfg(test)]
pub(crate) fn wait_text(round: &RoundId, asked: &Asked) -> String {
    wait_text_in(round, asked, Zone::Utc)
}

/// The line that tells `asked` of `round`, with any retry timestamp formatted in `zone`.
pub(crate) fn wait_text_in(round: &RoundId, asked: &Asked, _zone: Zone) -> String {
    match asked {
        Asked::Waits(what) => format!("round {round} waits for {what}"),
        Asked::Nothing => format!("round {round} waits for nothing: a run of it goes on, or it is closed"),
        Asked::Unknown(why) => {
            format!("round {round}: what it waits for is not known, the daemon was not asked: {why}")
        }
    }
}

/// Asks the daemon at `socket` what `round` waits for, and tells it in a line in `zone`.
///
/// A daemon that cannot be asked is told in the line, not left out: the follow goes on without it.
pub(crate) async fn wait_line_in(socket: &Path, round: &RoundId, zone: Zone) -> String {
    wait_text_in(round, &ask(socket, round).await, zone)
}

/// What the daemon at `socket` says of `round`.
async fn ask(socket: &Path, round: &RoundId) -> Asked {
    let mut connection = match Connection::connect(socket).await {
        Ok(connection) => connection,
        Err(error) => return Asked::Unknown(error_chain(&error)),
    };
    let request = Command::Show(ShowRequest {
        target: Target::Round(round.clone()),
    });
    match connection.request(Client::Cli, request).await {
        Ok(Reply::Round { round }) => round.waiting.map_or(Asked::Nothing, Asked::Waits),
        Ok(other) => Asked::Unknown(format!("the daemon answered with a {} reply", kind_of(&other))),
        Err(error) => Asked::Unknown(error_chain(&error)),
    }
}
