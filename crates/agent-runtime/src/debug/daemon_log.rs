//! Bounded read-only daemon log tail, restricted to this team and recent warnings.
use crate::store::Home;
use agent_core::state::Team;
use std::path::{Path, PathBuf};
use tokio::io::{AsyncReadExt, AsyncSeekExt};

/// Maximum bytes read from the end of a daemon log.
const LIMIT: u64 = 4 * 1024 * 1024;
/// Number of recent daemon lines inspected.
const LINES: usize = 500;

/// Why a daemon log could not be read.
#[derive(Debug, thiserror::Error)]
pub(super) enum LogError {
    /// The selected log could not be accessed.
    #[error("cannot read daemon log {}", path.display())]
    Read {
        /// File selected by the caller or the default.
        path: PathBuf,
        /// Original operating system failure.
        source: std::io::Error,
    },
    /// Log bytes are not UTF-8.
    #[error("daemon log is not UTF-8")]
    Utf8(#[source] std::string::FromUtf8Error),
    /// A source grew beyond its bounded snapshot.
    #[error("daemon log changed beyond the read limit")]
    TooLarge,
}

/// Reads the last 500 lines without retaining unrelated informational records.
///
/// # Errors
/// Returns [`LogError`] for unreadable explicit paths or invalid log text.
pub(super) async fn collect(
    home: &Home,
    explicit: Option<&Path>,
    team: &Team,
) -> Result<Option<Vec<String>>, LogError> {
    let path = explicit.map_or_else(|| home.root().join("daemon.log"), Path::to_path_buf);
    let mut file = match tokio::fs::File::open(&path).await {
        Ok(file) => file,
        Err(source) if explicit.is_none() && source.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(source) => return Err(LogError::Read { path, source }),
    };
    let len = file
        .metadata()
        .await
        .map_err(|source| LogError::Read {
            path: path.clone(),
            source,
        })?
        .len();
    let start = len.saturating_sub(LIMIT);
    file.seek(std::io::SeekFrom::Start(start))
        .await
        .map_err(|source| LogError::Read {
            path: path.clone(),
            source,
        })?;
    let mut bytes = Vec::new();
    file.take(LIMIT.saturating_add(1))
        .read_to_end(&mut bytes)
        .await
        .map_err(|source| LogError::Read { path, source })?;
    if bytes.len() > 4 * 1024 * 1024 {
        return Err(LogError::TooLarge);
    }
    // Seeking may land inside a UTF-8 character; discard only that incomplete first line.
    let body = if start == 0 {
        bytes
    } else {
        bytes
            .splitn(2, |byte| *byte == b'\n')
            .nth(1)
            .map_or_else(Vec::new, <[u8]>::to_vec)
    };
    let text = String::from_utf8(body).map_err(LogError::Utf8)?;
    let tasks: Vec<_> = team
        .crew()
        .roster()
        .into_iter()
        .map(|(_, member)| member.task())
        .chain(team.reviewers().iter().map(agent_core::state::Member::task))
        .collect();
    let mut kept: Vec<_> = text
        .lines()
        .rev()
        .take(LINES)
        .filter(|line| {
            line.contains(team.id().as_str()) || tasks.iter().any(|task| line.contains(task.as_str())) || warning(line)
        })
        .map(str::to_owned)
        .collect();
    kept.reverse();
    Ok(Some(kept))
}

/// The level of a JSON daemon record; unrelated fields are never retained.
#[derive(Debug, serde::Deserialize)]
struct LogLevel<'a> {
    /// Structured severity emitted by the daemon.
    level: &'a str,
}

fn warning(line: &str) -> bool {
    match serde_json::from_str::<LogLevel<'_>>(line) {
        Ok(record) => matches!(record.level, "WARN" | "ERROR" | "warn" | "error" | "warning"),
        // Text daemon logs are a supported alternative to JSON, parsed by complete level words.
        Err(_not_json) => line.split(|ch: char| !ch.is_ascii_alphabetic()).any(|word| {
            word.eq_ignore_ascii_case("warn")
                || word.eq_ignore_ascii_case("warning")
                || word.eq_ignore_ascii_case("error")
        }),
    }
}

#[cfg(test)]
mod tests;
