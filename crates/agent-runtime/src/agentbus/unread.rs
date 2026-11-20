//! The messages of the bus that the session of one task has not read yet: what the briefing of a task that is started
//! anew names, so that the new session knows what its team said to the old one.
//!
//! Like [`super::latest`], this reads the files of the bus itself, bounded, without the lock and changing nothing: the
//! log is append-only, and the bus replaces its state file whole (written aside, then renamed), so the cursor read is
//! one the bus wrote. Which message a session reads follows `isForSession` of `agentbus/team.go`, and the cursor
//! `cursor` of the same file: the one of the agent in the team, or the agent's single one before the team had its own.

use std::collections::BTreeMap;
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
    /// The state of the bus is longer than the reader reads.
    #[error("the state of the bus in {path:?} is longer than {limit} bytes")]
    StateTooLarge {
        /// The state file.
        path: PathBuf,
        /// The most bytes read.
        limit: u64,
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

/// How many bytes of the bus's state file are read at most: the bus keeps its teams, investigations and requests in it,
/// far below this; a longer file is refused rather than read whole.
const MAX_STATE_BYTES: u64 = 64 << 20;

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

/// The file of the bus that holds the messages, one JSON object a line.
const LOG_FILE: &str = "messages.jsonl";

/// The file of the bus that holds its state, with the read cursors.
const STATE_FILE: &str = "state.json";

/// The part of the state of the bus that tells where each session stopped reading.
#[derive(serde::Deserialize)]
struct State {
    /// The number of the last message read, by session: `<agent>/<team>` in a team, `<agent>` outside teams and in
    /// states written before teams kept their own.
    #[serde(default)]
    cursors: BTreeMap<String, u64>,
}

/// A line of the log, as far as [`unread_messages`] reads it.
#[derive(serde::Deserialize)]
struct Line {
    /// The number of the message.
    id: NonZeroU64,
    /// The sender.
    from: String,
    /// The recipient: an agent, the id of a task, `all`, or `team:` and a team.
    to: String,
    /// What it says.
    #[serde(default)]
    text: String,
    /// The team a direct message is bound to; empty when it is bound to none.
    #[serde(default)]
    team: String,
    /// The task a direct message is for; empty when it is for any session of the agent.
    #[serde(default)]
    task: String,
}

impl Line {
    /// Whether the session of `reader` reads this message (`isForSession` of `agentbus/team.go`).
    fn is_for(&self, reader: Reader<'_>) -> bool {
        let agent = reader.agent.as_str();
        if self.from == agent {
            return false;
        }
        let team = reader.team.as_str();
        let addressed = if self.to == agent || self.to == reader.task.as_str() {
            self.team.is_empty() || self.team == team
        } else if let Some(to_team) = self.to.strip_prefix("team:") {
            to_team == team
        } else {
            self.to == "all"
        };
        addressed && (self.task.is_empty() || self.task == reader.task.as_str())
    }
}

/// See [`AgentBus::unread_messages`]; `bus_dir` is the directory of the bus.
async fn unread_messages(bus_dir: &Path, reader: Reader<'_>) -> Result<UnreadMessages, UnreadMessagesError> {
    unread_messages_within(bus_dir, reader, MAX_STATE_BYTES).await
}

/// [`unread_messages`], with a state file of at most `state_limit` bytes.
async fn unread_messages_within(
    bus_dir: &Path,
    reader: Reader<'_>,
    state_limit: u64,
) -> Result<UnreadMessages, UnreadMessagesError> {
    let cursor = read_cursor(&bus_dir.join(STATE_FILE), reader, state_limit).await?;
    let path = bus_dir.join(LOG_FILE);
    let Some(window) = read_window(&path).await? else {
        return Ok(UnreadMessages {
            shown: Vec::new(),
            completeness: Completeness::All,
        });
    };
    let text = std::str::from_utf8(window.bytes()).map_err(|source| UnreadMessagesError::NotUtf8 {
        path: path.clone(),
        source,
    })?;
    let mut unread = Vec::new();
    let mut first_id = None;
    for (index, text) in text.lines().enumerate().filter(|(_, text)| !text.trim().is_empty()) {
        let line: Line = serde_json::from_str(text).map_err(|source| UnreadMessagesError::Line {
            path: path.clone(),
            line: index.saturating_add(1),
            source,
        })?;
        first_id.get_or_insert(line.id.get());
        if line.id.get() > cursor && line.is_for(reader) {
            unread.push(UnreadMessage {
                id: line.id,
                from: line.from,
                text: line.text.chars().take(MAX_TEXT_CHARS).collect(),
            });
        }
    }
    // A window that starts inside the log may have left out unread messages: those after the cursor and before the
    // first message read.
    let cut_before = window.cut && first_id.is_some_and(|first| first > cursor.saturating_add(1));
    let skip = unread.len().saturating_sub(MAX_MESSAGES);
    let completeness = if cut_before || skip > 0 {
        Completeness::Newest
    } else {
        Completeness::All
    };
    unread.drain(..skip);
    Ok(UnreadMessages {
        shown: unread,
        completeness,
    })
}

/// The number of the last message the session of `reader` read, from the state file at `path`; 0 when the bus has no
/// state or the session never read; a file longer than `limit` bytes is refused.
async fn read_cursor(path: &Path, reader: Reader<'_>, limit: u64) -> Result<u64, UnreadMessagesError> {
    use tokio::io::AsyncReadExt;

    let read = |source| UnreadMessagesError::Read {
        path: path.to_owned(),
        source,
    };
    let file = match tokio::fs::File::open(path).await {
        Ok(file) => file,
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(source) => return Err(read(source)),
    };
    let mut bytes = Vec::new();
    // One byte more than the limit tells a file of exactly the limit from a longer one.
    file.take(limit.saturating_add(1))
        .read_to_end(&mut bytes)
        .await
        .map_err(read)?;
    if u64::try_from(bytes.len()).map_or(true, |len| len > limit) {
        return Err(UnreadMessagesError::StateTooLarge {
            path: path.to_owned(),
            limit,
        });
    }
    let state: State = serde_json::from_slice(&bytes).map_err(|source| UnreadMessagesError::State {
        path: path.to_owned(),
        source,
    })?;
    let in_team = format!("{}/{}", reader.agent, reader.team);
    let cursor = state
        .cursors
        .get(&in_team)
        .or_else(|| state.cursors.get(reader.agent.as_str()))
        .copied();
    // A session that never read has read nothing: every message is after its cursor.
    Ok(cursor.unwrap_or(0))
}

/// The end of the log that is read: whole lines, at most [`WINDOW_BYTES`].
struct Window {
    /// The bytes read, from the start of the file or from just after the line break where the window starts.
    bytes: Vec<u8>,
    /// Where the lines start in [`Window::bytes`].
    from: usize,
    /// Whether the window starts inside the log, so that older lines were left out.
    cut: bool,
}

impl Window {
    /// The whole lines of the window.
    fn bytes(&self) -> &[u8] {
        self.bytes.get(self.from..).unwrap_or_default()
    }
}

/// Reads the end of the log at `path`; `None` when there is no log.
async fn read_window(path: &Path) -> Result<Option<Window>, UnreadMessagesError> {
    use tokio::io::{AsyncReadExt, AsyncSeekExt};

    let read = |source| UnreadMessagesError::Read {
        path: path.to_owned(),
        source,
    };
    let mut file = match tokio::fs::File::open(path).await {
        Ok(file) => file,
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(source) => return Err(read(source)),
    };
    let len = file.metadata().await.map_err(read)?.len();
    let start = len.saturating_sub(WINDOW_BYTES);
    file.seek(std::io::SeekFrom::Start(start)).await.map_err(read)?;
    let mut bytes = Vec::new();
    file.take(WINDOW_BYTES).read_to_end(&mut bytes).await.map_err(read)?;
    // A window that starts inside the log starts inside a line, and maybe inside a character: it is cut at the first
    // line break, before the text is decoded.
    let cut = start > 0;
    let from = if cut {
        bytes
            .iter()
            .position(|byte| *byte == b'\n')
            .map_or(bytes.len(), |at| at.saturating_add(1))
    } else {
        0
    };
    Ok(Some(Window { bytes, from, cut }))
}

#[cfg(test)]
mod tests;
