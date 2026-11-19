//! The newest message the bus holds for an agent, read from the end of the message log: what the daemon names when it
//! wakes the agent for a message.
//!
//! This is the one place the daemon reads a file of the bus itself; every other fact it takes from a command. The read
//! is bounded, takes no lock and changes nothing: the log is append-only, and a line that was being written when it was
//! read is the last one, and is reported as the failure it is.

use std::num::NonZeroU64;
use std::path::{Path, PathBuf};

use agent_core::ids::{AgentName, TeamId};
use tokio::io::{AsyncReadExt, AsyncSeekExt};

use super::AgentBus;

/// The file of the bus that holds the messages, one JSON object a line.
const LOG_FILE: &str = "messages.jsonl";

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
        source: std::str::Utf8Error,
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

/// A line of the log, as far as [`latest_message_for`] reads it.
#[derive(serde::Deserialize)]
struct Line {
    /// The number of the message.
    id: NonZeroU64,
    /// The sender.
    from: String,
    /// The recipient: an agent, `all`, or `team:` and a team.
    to: String,
    /// Whether it was sent so that it wakes nobody.
    #[serde(default)]
    quiet: bool,
}

/// See [`AgentBus::latest_message_for`]; `bus_dir` is the directory that holds the log.
async fn latest_message_for(
    bus_dir: &Path,
    agent: &AgentName,
    team: &TeamId,
) -> Result<Option<MessageNote>, LatestMessageError> {
    let path = bus_dir.join(LOG_FILE);
    let read = |source| LatestMessageError::Read {
        path: path.clone(),
        source,
    };
    let mut file = match tokio::fs::File::open(&path).await {
        Ok(file) => file,
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(source) => return Err(read(source)),
    };
    let len = file.metadata().await.map_err(read)?.len();
    let start = len.saturating_sub(WINDOW_BYTES);
    file.seek(std::io::SeekFrom::Start(start)).await.map_err(read)?;
    let mut bytes = Vec::new();
    file.take(WINDOW_BYTES).read_to_end(&mut bytes).await.map_err(read)?;
    // A window that starts inside the log starts inside a line, and maybe inside a character: it is cut at the first line
    // break, before the text is decoded.
    let whole = if start == 0 {
        bytes.as_slice()
    } else {
        bytes
            .iter()
            .position(|byte| *byte == b'\n')
            .and_then(|at| bytes.get(at + 1..))
            .unwrap_or_default()
    };
    let text = std::str::from_utf8(whole).map_err(|source| LatestMessageError::NotUtf8 {
        path: path.clone(),
        source,
    })?;
    let to_team = format!("team:{team}");
    let mut newest = None;
    for (index, text) in text.lines().enumerate().filter(|(_, text)| !text.trim().is_empty()) {
        let line: Line = serde_json::from_str(text).map_err(|source| LatestMessageError::Line {
            path: path.clone(),
            line: index + 1,
            source,
        })?;
        let for_agent = line.to == agent.as_str() || line.to == to_team || line.to == "all";
        if for_agent && !line.quiet && line.from != agent.as_str() {
            newest = Some(MessageNote {
                id: line.id,
                from: line.from,
            });
        }
    }
    Ok(newest)
}

#[cfg(test)]
mod tests;
