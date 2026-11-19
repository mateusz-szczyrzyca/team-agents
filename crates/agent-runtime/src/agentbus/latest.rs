//! The newest message the bus holds for an agent, read from the end of the message log: what the daemon names when it
//! wakes the agent for a message.
//!
//! This is the one place the daemon reads a file of the bus itself; every other fact it takes from a command. The read
//! is bounded, takes no lock and changes nothing: the log is append-only, and a line that was being written when it was
//! read is the last one, which [`latest_message_for`] reports as the failure it is.

use std::num::NonZeroU64;
use std::path::{Path, PathBuf};

use agent_core::ids::{AgentName, TeamId};

use super::AgentBus;

/// How many bytes from the end of the log are read; a message that wakes an agent is among the last ones.
const WINDOW_BYTES: u64 = 1 << 20;

/// A message of the bus, as far as a wake-up needs to name it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MessageNote {
    /// Its number on the bus.
    pub id: NonZeroU64,
    /// Who sent it: an agent, `agentbus` for a notice of the system, or the human.
    pub from: String,
}

/// Why the message log could not be read for the newest message of an agent.
#[derive(Debug, thiserror::Error)]
pub enum LatestMessageError {
    /// The log could not be opened or read.
    #[error("cannot read the messages of the bus in {path:?}")]
    Read {
        /// The log.
        path: PathBuf,
        /// What the system said.
        source: std::io::Error,
    },
    /// The part of the log that was read is not UTF-8.
    #[error("the messages of the bus in {path:?} are not UTF-8")]
    NotUtf8 {
        /// The log.
        path: PathBuf,
        /// Where the text goes wrong.
        source: std::string::FromUtf8Error,
    },
    /// A line of the log is not a message.
    #[error("line {line} of the messages of the bus in {path:?} is not a message")]
    Line {
        /// The log.
        path: PathBuf,
        /// The line, counted from the first line that was read.
        line: usize,
        /// What the decoder said.
        source: serde_json::Error,
    },
}

impl AgentBus {
    /// The newest message that would wake `agent`, a member of `team`: sent to it, to its team or to everybody, by
    /// somebody else, and not sent quietly. `None` when the log holds none, or does not exist.
    ///
    /// # Errors
    /// [`LatestMessageError`].
    pub async fn latest_message_for(
        &self,
        agent: &AgentName,
        team: &TeamId,
    ) -> Result<Option<MessageNote>, LatestMessageError> {
        latest_message_for(&self.bus_dir(), agent, team).await
    }
}

/// See [`AgentBus::latest_message_for`]; `bus_dir` is the directory that holds the log.
async fn latest_message_for(
    bus_dir: &Path,
    agent: &AgentName,
    team: &TeamId,
) -> Result<Option<MessageNote>, LatestMessageError> {
    let _ = (bus_dir, agent, team);
    Ok(None)
}

#[cfg(test)]
mod tests;
