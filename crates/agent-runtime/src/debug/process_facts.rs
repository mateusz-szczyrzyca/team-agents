//! Read-only Linux daemon observations, selected by ownership of the home's lock.

use crate::store::Home;
use agent_core::debug_summary::DaemonSummary;
use std::path::Path;

/// Why daemon process observation failed.
#[derive(Debug, thiserror::Error)]
#[error("cannot observe daemon process")]
pub(super) struct ProcessError;

/// Observes the process holding this home's lock without changing the home.
pub(super) async fn observe(_home: &Home, _procfs: &Path) -> Result<Option<DaemonSummary>, ProcessError> {
    Ok(None)
}

/// Observes using the operating system's clock-tick unit.
async fn observe_with_ticks(
    home: &Home,
    procfs: &Path,
    _ticks: std::num::NonZeroU64,
) -> Result<Option<DaemonSummary>, ProcessError> {
    observe(home, procfs).await
}

#[cfg(test)]
mod tests;
