//! The snapshots of a round's tasks as `agent status <round>` shows them: the last one of each task, when it was taken and
//! how big it is.

#[cfg(test)]
mod tests;

use agent_core::history::SnapshotRecord;
use time::UtcOffset;

/// One line per task that has a snapshot, showing its last snapshot. `snapshots` is in the order of the history: by task,
/// then by number, so the last of a task is the one that follows the others of it.
pub(crate) fn snapshot_lines(_snapshots: &[SnapshotRecord], _offset: UtcOffset) -> String {
    String::new()
}

/// The snapshots of `round` in the history of `home`, by task and then by number; a history that cannot be read is said so
/// in the line that replaces them.
pub(crate) fn snapshots_of(
    _home: &agent_runtime::store::Home,
    _round: &agent_core::ids::RoundId,
    _offset: UtcOffset,
) -> String {
    String::new()
}
