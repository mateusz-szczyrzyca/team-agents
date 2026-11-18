//! Read-only machine and resource observations: load, memory, filesystems, processes, directory sizes.

#[cfg(test)]
mod tests;

use std::path::Path;

use agent_core::debug_summary::MachineSummary;
use agent_core::diagnostic_assessment::{Filesystem, ReadFailure};

use agent_core::state::Timestamp;

use crate::debug_redaction::SecretsCut;
use crate::store::Home;

/// Machine observations ready for assessment, summary, and bundle persistence.
#[derive(Debug)]
pub(super) struct MachineObservation {
    /// Summary facts for the Machine section.
    pub(super) summary: MachineSummary,
    /// Unique filesystems for diagnostic assessment.
    pub(super) filesystems: Vec<Filesystem>,
    /// Read failures during collection.
    pub(super) unreadable: Vec<ReadFailure>,
    /// Redacted text artifact for `machine.txt`.
    pub(super) artifact_bytes: Vec<u8>,
}

/// Collects machine metrics and descendant processes.
pub(super) async fn observe(
    _home: &Home,
    _procfs: &Path,
    _temp_dir: &Path,
    _daemon_pid: Option<u32>,
    _cut: &SecretsCut,
    _now: Timestamp,
) -> MachineObservation {
    // Stub for RED commit
    MachineObservation {
        summary: MachineSummary {
            load_avg: None,
            memory_total_bytes: None,
            memory_available_bytes: None,
            home_fs: None,
            temp_fs: None,
            daemon_descendant_pids: vec![],
            tasks_dir_bytes: None,
            worktrees_dir_bytes: None,
        },
        filesystems: vec![],
        unreadable: vec![],
        artifact_bytes: vec![],
    }
}
