//! Read-only collection of newest untracked file metadata per worktree.

#[cfg(test)]
mod tests;

use std::path::{Path, PathBuf};

use agent_core::debug_summary::UntrackedFileSummaryFacts;
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

    for rel_path in untracked_paths {
        let full_path = worktree.join(rel_path);
        let Ok(meta) = tokio::fs::symlink_metadata(&full_path).await else {
            continue;
        };

        if meta.is_dir() {
            continue;
        }

        let size_bytes = meta.len();
        let modified = meta
            .modified()
            .ok()
            .and_then(|st| Timestamp::from_utc(time::UtcDateTime::from(st)).ok());

        files.push(UntrackedFileSummaryFacts {
            path: rel_path.clone(),
            size_bytes,
            modified,
        });
    }

    files.sort_by(|a, b| b.modified.cmp(&a.modified).then_with(|| a.path.cmp(&b.path)));
    files.truncate(50);
    UntrackedCollection {
        summary_facts: files,
        artifact_text: String::new(),
        unreadable: Vec::new(),
    }
}
