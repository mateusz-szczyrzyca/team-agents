//! Bounded read-only notifications collector for team tasks.

#[cfg(test)]
mod tests;

use std::path::PathBuf;

use agent_core::state::Team;

use crate::store::Home;
use crate::text_file::read_text_file;

/// Maximum bytes read per notification file.
const LIMIT: usize = 4 * 1024 * 1024;
/// Maximum directory entries inspected in notifications directory.
const MAX_DIR_ENTRIES: usize = 4096;
/// Maximum total bytes across all collected notification files.
const MAX_TOTAL_BYTES: usize = 4 * 1024 * 1024;

/// Why notifications could not be read.
#[derive(Debug, thiserror::Error)]
pub(super) enum NotificationError {
    /// The notifications directory or file could not be read.
    #[error("cannot read notifications in {path:?}")]
    Read {
        /// File or directory path.
        path: PathBuf,
        /// Original operating system failure.
        source: std::io::Error,
    },
    /// Notification bytes are not UTF-8 or file cannot be read.
    #[error("cannot read notification file {path:?}")]
    Text {
        /// File path.
        path: PathBuf,
        /// Source error.
        source: crate::text_file::TextFileError,
    },
    /// Directory holds more entries than allowed.
    #[error("too many notification entries in {path:?} ({count} > {limit})")]
    TooManyEntries {
        /// Notifications directory.
        path: PathBuf,
        /// Number of entries observed before stopping.
        count: usize,
        /// Maximum allowed entries.
        limit: usize,
    },
    /// Total bytes scanned across notification files exceeds budget.
    #[error("total notification bytes exceeds limit ({bytes} > {limit})")]
    TotalBytesExceeded {
        /// Accumulated bytes scanned.
        bytes: usize,
        /// Maximum allowed bytes.
        limit: usize,
    },
}

/// Parses the timestamp and numeric sequence suffix of a notification filename.
fn parse_sort_key(name: &str) -> Option<(&str, u64)> {
    let path = std::path::Path::new(name);
    if !path.extension().is_some_and(|ext| ext.eq_ignore_ascii_case("md")) {
        return None;
    }
    let inner = path.file_stem()?.to_str()?.strip_prefix("notification-")?;
    let (stamp, suffix) = inner.rsplit_once('-')?;
    let num = suffix.parse::<u64>().ok()?;
    Some((stamp, num))
}

/// Checks whether notification header fields match the target team or any of its tasks.
fn matches_team_headers(text: &str, team_id: &str, tasks: &[String]) -> bool {
    for line in text.lines() {
        if line.is_empty() {
            break;
        }
        if line.strip_prefix("Team: ").is_some_and(|team| team.trim() == team_id) {
            return true;
        }
        if line
            .strip_prefix("Task: ")
            .is_some_and(|task| tasks.iter().any(|t| t == task.trim()))
        {
            return true;
        }
    }
    false
}

/// Reads notifications written for the team and its tasks, whole, the newest last.
///
/// # Errors
/// Returns [`NotificationError`] when reading the directory or any notification file fails.
pub(super) async fn collect(home: &Home, team: &Team) -> Result<Option<Vec<String>>, NotificationError> {
    let dir = home.notifications_dir();
    let mut entries = match tokio::fs::read_dir(&dir).await {
        Ok(entries) => entries,
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => return Ok(Some(Vec::new())),
        Err(source) => return Err(NotificationError::Read { path: dir, source }),
    };

    let mut files = Vec::new();
    let mut entries_count: usize = 0;
    while let Some(entry) = entries.next_entry().await.map_err(|source| NotificationError::Read {
        path: dir.clone(),
        source,
    })? {
        entries_count = entries_count.saturating_add(1);
        if entries_count > MAX_DIR_ENTRIES {
            break;
        }
        let file_name = entry.file_name();
        let Some(name_str) = file_name.to_str() else {
            continue;
        };
        if name_str.starts_with("notification-")
            && std::path::Path::new(name_str)
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("md"))
        {
            files.push(entry.path());
        }
    }

    files.sort_by(|a, b| {
        let key_a = a.file_name().and_then(|n| n.to_str()).and_then(parse_sort_key);
        let key_b = b.file_name().and_then(|n| n.to_str()).and_then(parse_sort_key);
        key_a.cmp(&key_b).then_with(|| a.cmp(b))
    });

    let tasks: Vec<String> = team
        .crew()
        .roster()
        .into_iter()
        .map(|(_, member)| member.task().to_string())
        .chain(team.reviewers().iter().map(|m| m.task().to_string()))
        .collect();
    let team_id = team.id().to_string();

    let mut kept = Vec::new();
    let mut total_bytes: usize = 0;
    for file in files {
        if let Some(text_file) = read_text_file(&file, LIMIT)
            .await
            .map_err(|source| NotificationError::Text {
                path: file.clone(),
                source,
            })?
        {
            let text = text_file.text;
            if matches_team_headers(&text, &team_id, &tasks) {
                let len = text.len();
                if total_bytes.saturating_add(len) > MAX_TOTAL_BYTES {
                    break;
                }
                total_bytes = total_bytes.saturating_add(len);
                kept.push(text);
            }
        }
    }

    Ok(Some(kept))
}
