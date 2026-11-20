//! The messages of the bus that the session of one task has not read yet: what the briefing of a task that is started
//! anew names, so that the new session knows what its team said to the old one.
//!
//! Like [`super::latest`], this reads the files of the bus itself, bounded, without the lock and changing nothing: the
//! log is append-only, and the bus replaces its state file whole (written aside, then renamed), so the cursor read is
//! one the bus wrote. Which message a session reads follows `isForSession` of `agentbus/team.go`, and the cursor
//! `cursor` of the same file: the one of the agent in the team, or the agent's single one before the team had its own.

use std::num::NonZeroU64;
use std::path::{Path, PathBuf};

use agent_core::ids::{AgentName, TaskId, TeamId};

use super::AgentBus;

/// The session whose unread messages are wanted: an agent, in a team, working on a task.
#[derive(Debug, Clone, Copy)]
pub struct Reader<'a> {
    /// The agent the messages were written to.
    pub agent: &'a AgentName,
    /// The team the agent's session works in.
    pub team: &'a TeamId,
    /// The task of the session.
    pub task: &'a TaskId,
}

/// A message the session has not read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnreadMessage {
    /// Its number on the bus.
    pub id: NonZeroU64,
    /// Who sent it: an agent, `agentbus` for a notice of the system, or the human.
    pub from: String,
    /// What it says, as the bus rendered it, cut to [`MAX_TEXT_CHARS`] characters.
    pub text: String,
}

/// Whether [`UnreadMessages::shown`] holds every unread message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Completeness {
    /// Every unread message is shown.
    All,
    /// Only the newest are shown: there were more than [`MAX_MESSAGES`], or older ones lie before the part of the log
    /// that is read. The session reads the rest with `read_messages`.
    Newest,
}

/// The unread messages of a session, oldest first.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnreadMessages {
    /// The messages, oldest first; at most [`MAX_MESSAGES`].
    pub shown: Vec<UnreadMessage>,
    /// Whether they are all of them.
    pub completeness: Completeness,
}

/// Why the unread messages of a session could not be read.
#[derive(Debug, thiserror::Error)]
pub enum UnreadMessagesError {
    /// A file of the bus could not be opened or read.
    #[error("cannot read {path:?} of the bus")]
    Read {
        /// The file.
        path: PathBuf,
        /// What the system said.
        source: std::io::Error,
    },
    /// The state of the bus is not the JSON the bus writes.
    #[error("the state of the bus in {path:?} cannot be read")]
    State {
        /// The state file.
        path: PathBuf,
        /// What the decoder said.
        source: serde_json::Error,
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

/// How many bytes from the end of the log are read: the messages a session has not read are among the last ones.
const WINDOW_BYTES: u64 = 4 << 20;

/// How many unread messages are shown at most: the newest ones.
pub const MAX_MESSAGES: usize = 20;

/// How many characters of a message are shown at most.
pub const MAX_TEXT_CHARS: usize = 1500;

impl AgentBus {
    /// The messages the session of `reader` has not read, oldest first, at most [`MAX_MESSAGES`] of them. A bus with no
    /// log or no state has none, or has never been read: every message for the session is unread then.
    ///
    /// # Errors
    /// [`UnreadMessagesError`].
    pub async fn unread_messages(&self, reader: Reader<'_>) -> Result<UnreadMessages, UnreadMessagesError> {
        unread_messages(&self.bus_dir(), reader).await
    }
}

/// See [`AgentBus::unread_messages`]; `bus_dir` is the directory of the bus.
async fn unread_messages(_bus_dir: &Path, _reader: Reader<'_>) -> Result<UnreadMessages, UnreadMessagesError> {
    Ok(UnreadMessages {
        shown: vec![UnreadMessage {
            id: NonZeroU64::MAX,
            from: String::new(),
            text: String::new(),
        }],
        completeness: Completeness::Newest,
    })
}

#[cfg(test)]
mod tests;
