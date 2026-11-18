//! Bounded read-only notifications collector for team tasks.

#[cfg(test)]
mod tests;

use std::path::PathBuf;

use agent_core::state::Team;

use crate::store::Home;
use crate::text_file::read_text_file;

/// Maximum bytes read per notification file.
const LIMIT: usize = 4 * 1024 * 1024;

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
    while let Some(entry) = entries.next_entry().await.map_err(|source| NotificationError::Read {
        path: dir.clone(),
        source,
    })? {
        let file_name = entry.file_name();
        let name_str = file_name.to_string_lossy();
        if name_str.starts_with("notification-") && name_str.ends_with(".md") {
            files.push(entry.path());
        }
    }

    files.sort();

    let tasks: Vec<String> = team
        .crew()
        .roster()
        .into_iter()
        .map(|(_, member)| member.task().to_string())
        .chain(team.reviewers().iter().map(|m| m.task().to_string()))
        .collect();
    let team_id = team.id().to_string();

    let mut kept = Vec::new();
    for file in files {
        if let Some(text_file) = read_text_file(&file, LIMIT)
            .await
            .map_err(|source| NotificationError::Text {
                path: file.clone(),
                source,
            })?
        {
            let text = text_file.text;
            let is_team = text.contains(&team_id) || tasks.iter().any(|t| text.contains(t));
            if is_team {
                kept.push(text.trim_end().to_owned());
            }
        }
    }

    Ok(Some(kept))
}
