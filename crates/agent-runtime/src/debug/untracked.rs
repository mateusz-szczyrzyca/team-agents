//! Read-only collection of newest untracked file metadata per worktree.

#[cfg(test)]
mod tests;

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use agent_core::debug_summary::{SummarySection, UntrackedFileSummaryFacts};
use agent_core::diagnostic_assessment::ReadFailure;
use agent_core::state::{InvalidTimestamp, Timestamp};
use time::{SignedDuration, UtcDateTime};

use crate::process::error_chain;

/// Collected untracked files metadata for a single worktree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct UntrackedCollection {
    /// Up to fifty newest untracked files for summary rendering.
    pub summary_facts: Vec<UntrackedFileSummaryFacts>,
    /// Complete listing of all untracked files with metadata for the bundle.
    pub artifact_text: String,
    /// Unreadable untracked paths with error reasons.
    pub unreadable: Vec<ReadFailure>,
}

/// Why a file's modification time cannot be recorded as a [`Timestamp`].
#[derive(Debug, thiserror::Error)]
enum ModifiedError {
    /// The time is further from the epoch than the time crate can represent. The conversion
    /// reports no detail beyond that, so there is no cause to keep.
    #[error("modification time is outside the range the time crate represents")]
    OutOfRange,
    /// The time lies outside the years 0000 to 9999 that state timestamps hold.
    #[error("cannot record modification time")]
    Timestamp(#[source] InvalidTimestamp),
}

/// Converts a file's modification time into a [`Timestamp`] without panicking.
///
/// The time crate's `From<SystemTime>` panics on times past its own range, so the offset from the
/// epoch is added with checked arithmetic instead.
///
/// # Errors
/// [`ModifiedError::OutOfRange`] when the time crate cannot represent the time,
/// [`ModifiedError::Timestamp`] when its year is outside 0000 to 9999.
fn modified_timestamp(system_time: SystemTime) -> Result<Timestamp, ModifiedError> {
    let utc = match system_time.duration_since(SystemTime::UNIX_EPOCH) {
        Ok(after) => {
            let offset = match SignedDuration::try_from(after) {
                Ok(offset) => offset,
                Err(time::error::ConversionRange) => return Err(ModifiedError::OutOfRange),
            };
            UtcDateTime::UNIX_EPOCH.checked_add(offset)
        }
        Err(before) => {
            let offset = match SignedDuration::try_from(before.duration()) {
                Ok(offset) => offset,
                Err(time::error::ConversionRange) => return Err(ModifiedError::OutOfRange),
            };
            UtcDateTime::UNIX_EPOCH.checked_sub(offset)
        }
    }
    .ok_or(ModifiedError::OutOfRange)?;
    Timestamp::from_utc(utc).map_err(ModifiedError::Timestamp)
}

/// Spells a worktree-relative path so that distinct paths stay distinct, including non-UTF-8 names.
///
/// `Path::display` replaces invalid bytes with U+FFFD, so two names differing only in such bytes would
/// print the same. The `Debug` spelling of the OS string escapes them (`"f\xFE"`) and is injective.
#[expect(
    clippy::unnecessary_debug_formatting,
    reason = "the Debug spelling is the lossless one; Display drops invalid bytes"
)]
fn lossless_path(path: &Path) -> String {
    format!("{:?}", path.as_os_str())
}

/// Collects metadata for up to fifty newest untracked files in `worktree`.
pub(super) async fn collect(worktree: &Path, untracked_paths: &[PathBuf]) -> UntrackedCollection {
    let mut files = Vec::new();
    let mut unreadable = Vec::new();

    for rel_path in untracked_paths {
        let full_path = worktree.join(rel_path);
        let meta = match tokio::fs::symlink_metadata(&full_path).await {
            Ok(m) => m,
            Err(source) => {
                unreadable.push(ReadFailure {
                    section: SummarySection::GitWorktrees,
                    part: format!("untracked file {}", lossless_path(rel_path)),
                    reason: error_chain(&source),
                });
                continue;
            }
        };

        if meta.is_dir() {
            continue;
        }

        let size_bytes = meta.len();
        let modified = match meta.modified() {
            Ok(system_time) => match modified_timestamp(system_time) {
                Ok(ts) => Some(ts),
                Err(source) => {
                    unreadable.push(ReadFailure {
                        section: SummarySection::GitWorktrees,
                        part: format!("untracked file {}", lossless_path(rel_path)),
                        reason: error_chain(&source),
                    });
                    None
                }
            },
            Err(source) => {
                unreadable.push(ReadFailure {
                    section: SummarySection::GitWorktrees,
                    part: format!("untracked file {}", lossless_path(rel_path)),
                    reason: error_chain(&source),
                });
                None
            }
        };

        files.push(UntrackedFileSummaryFacts {
            path: rel_path.clone(),
            size_bytes,
            modified,
        });
    }

    files.sort_by(|a, b| b.modified.cmp(&a.modified).then_with(|| a.path.cmp(&b.path)));

    let mut artifact_text = String::new();
    let mut append = |line: &str| {
        artifact_text.push_str(line);
        artifact_text.push('\n');
    };
    append(&format!("### Untracked Files ({})", files.len()));
    for f in &files {
        let time_str = match f.modified {
            Some(ts) => ts.to_string(),
            None => "unknown".to_owned(),
        };
        append(&format!(
            "- {} ({} bytes, modified {})",
            lossless_path(&f.path),
            f.size_bytes,
            time_str
        ));
    }

    let summary_facts = files.iter().take(50).cloned().collect();

    UntrackedCollection {
        summary_facts,
        artifact_text,
        unreadable,
    }
}
