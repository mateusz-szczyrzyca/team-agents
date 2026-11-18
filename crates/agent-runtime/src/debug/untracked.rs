//! Read-only collection of newest untracked file metadata per worktree.

#[cfg(test)]
mod tests;

use std::path::{Path, PathBuf};

use agent_core::debug_summary::UntrackedFileSummaryFacts;
use agent_core::diagnostic_assessment::ReadFailure;

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
pub(super) async fn collect(_worktree: &Path, _untracked_paths: &[PathBuf]) -> UntrackedCollection {
    // Stub for RED commit
    UntrackedCollection {
        summary_facts: Vec::new(),
        artifact_text: String::new(),
        unreadable: Vec::new(),
    }
}
