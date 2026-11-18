//! Bounded read-only bus message tail, restricted to this team and the end of the bus.

#[cfg(test)]
mod tests;

use std::path::{Path, PathBuf};

use agent_core::state::Team;
use tokio::io::{AsyncReadExt, AsyncSeekExt};

use crate::debug_redaction::SecretsCut;
use crate::store::Home;

/// Maximum bytes read from the end of the bus log.
const LIMIT: u64 = 4 * 1024 * 1024;
/// Number of recent bus messages inspected from the end of the whole bus.
const RECENT_MESSAGES: usize = 50;

/// Artifact and summary representations of collected bus messages.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct BusCollection {
    /// Full selected messages for the `bus.txt` artifact.
    pub(super) artifact_messages: Vec<String>,
    /// Redacted and truncated message lines for the markdown summary.
    pub(super) summary_messages: Vec<String>,
}

/// Why the bus log could not be read.
#[derive(Debug, thiserror::Error)]
pub(super) enum BusError {
    /// The bus log could not be accessed.
    #[error("cannot read bus messages {path:?}")]
    Read {
        /// File path.
        path: PathBuf,
        /// Original operating system failure.
        source: std::io::Error,
    },
    /// A bus message line could not be parsed as JSON.
    #[error("cannot parse bus message line {line}")]
    Parse {
        /// 1-based line number.
        line: usize,
        /// Original JSON deserialization error.
        #[source]
        source: serde_json::Error,
    },
    /// Log bytes are not UTF-8.
    #[error("bus messages log is not UTF-8")]
    Utf8(#[source] std::string::FromUtf8Error),
}

/// Raw message payload parsed from JSON Lines on the bus.
#[derive(serde::Deserialize)]
struct RawBusMessage {
    /// Timestamp when posted.
    ts: String,
    /// Sender identifier.
    from: String,
    /// Recipient identifier.
    to: String,
    /// Message kind or category, if present.
    #[serde(default)]
    kind: Option<String>,
    /// Message text payload.
    text: String,
}

/// Reads the last messages to, from or about the team, its tasks and its agents,
/// and the last messages of the whole bus.
///
/// # Errors
/// Returns [`BusError`] when reading the bus file fails.
pub(super) async fn collect(home: &Home, team: &Team, cut: &SecretsCut) -> Result<Option<BusCollection>, BusError> {
    let path = home.root().join("bus").join("messages.jsonl");
    let file = match tokio::fs::File::open(&path).await {
        Ok(file) => file,
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => {
            return Ok(Some(BusCollection {
                artifact_messages: Vec::new(),
                summary_messages: Vec::new(),
            }));
        }
        Err(source) => return Err(BusError::Read { path, source }),
    };
    let len = file
        .metadata()
        .await
        .map_err(|source| BusError::Read {
            path: path.clone(),
            source,
        })?
        .len();
    let start = len.saturating_sub(LIMIT);
    let bytes = read_tail(file, len, &path).await?;
    let body = if start == 0 {
        bytes
    } else {
        bytes
            .splitn(2, |byte| *byte == b'\n')
            .nth(1)
            .map_or_else(Vec::new, <[u8]>::to_vec)
    };
    let text = String::from_utf8(body).map_err(BusError::Utf8)?;
    let tasks: Vec<String> = team
        .crew()
        .roster()
        .into_iter()
        .map(|(_, member)| member.task().to_string())
        .chain(team.reviewers().iter().map(|m| m.task().to_string()))
        .collect();
    let agents: Vec<String> = team
        .crew()
        .roster()
        .into_iter()
        .map(|(_, member)| member.agent().name().to_string())
        .chain(team.reviewers().iter().map(|m| m.agent().name().to_string()))
        .collect();
    let team_id = team.id().to_string();
    let team_to = format!("team:{team_id}");

    let mut parsed_lines = Vec::new();
    for (line_idx, line) in text.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let msg = serde_json::from_str::<RawBusMessage>(line).map_err(|source| BusError::Parse {
            line: line_idx + 1,
            source,
        })?;
        parsed_lines.push(msg);
    }

    let mut artifact_messages = Vec::new();
    let mut summary_messages = Vec::new();
    let total = parsed_lines.len();

    for (index, msg) in parsed_lines.into_iter().enumerate() {
        let age = total.saturating_sub(index).saturating_sub(1);
        let is_team = msg.from == team_id
            || agents.iter().any(|a| &msg.from == a)
            || tasks.iter().any(|t| &msg.from == t)
            || msg.to == team_to
            || msg.to == team_id
            || agents.iter().any(|a| &msg.to == a)
            || tasks.iter().any(|t| &msg.to == t)
            || msg.text.contains(&team_id)
            || tasks.iter().any(|t| msg.text.contains(t))
            || agents.iter().any(|a| msg.text.contains(a));

        if is_team || age < RECENT_MESSAGES {
            let ts = &msg.ts;
            let from = &msg.from;
            let to = &msg.to;
            let kind = msg.kind.as_deref().unwrap_or("chat");
            artifact_messages.push(format!("[{ts}] {from} → {to} ({kind}): {}", msg.text));

            let redacted = cut.redact_text(&msg.text);
            let first_line = redacted.lines().next().unwrap_or("");
            let text_start = if first_line.chars().count() > 100 {
                format!("{}…", first_line.chars().take(100).collect::<String>())
            } else {
                first_line.to_owned()
            };
            summary_messages.push(format!("[{ts}] {from} → {to} ({kind}): {text_start}"));
        }
    }
    Ok(Some(BusCollection {
        artifact_messages,
        summary_messages,
    }))
}

async fn read_tail(mut file: tokio::fs::File, len: u64, path: &Path) -> Result<Vec<u8>, BusError> {
    file.seek(std::io::SeekFrom::Start(len.saturating_sub(LIMIT)))
        .await
        .map_err(|source| BusError::Read {
            path: path.to_owned(),
            source,
        })?;
    let mut bytes = Vec::new();
    file.take(len.min(LIMIT))
        .read_to_end(&mut bytes)
        .await
        .map_err(|source| BusError::Read {
            path: path.to_owned(),
            source,
        })?;
    Ok(bytes)
}
