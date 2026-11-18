//! Read-only collection of newest untracked file metadata per worktree.

#[cfg(test)]
mod tests;

use std::path::{Path, PathBuf};

use agent_core::debug_summary::{SummarySection, UntrackedFileSummaryFacts};
use agent_core::diagnostic_assessment::ReadFailure;
use agent_core::state::Timestamp;

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
                    part: format!("untracked file {}", rel_path.display()),
                    reason: source.to_string(),
                });
                continue;
            }
        };

        if meta.is_dir() {
            continue;
        }

        let size_bytes = meta.len();
        let modified = match meta.modified() {
            Ok(system_time) => match Timestamp::from_utc(time::UtcDateTime::from(system_time)) {
                Ok(ts) => Some(ts),
                Err(source) => {
                    unreadable.push(ReadFailure {
                        section: SummarySection::GitWorktrees,
                        part: format!("untracked file {}", rel_path.display()),
                        reason: source.to_string(),
                    });
                    None
                }
            },
            Err(source) => {
                unreadable.push(ReadFailure {
                    section: SummarySection::GitWorktrees,
                    part: format!("untracked file {}", rel_path.display()),
                    reason: source.to_string(),
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
            f.path.display(),
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
