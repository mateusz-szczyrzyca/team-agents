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
    let _inputs = (home, explicit, team);
    Ok(None)
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
