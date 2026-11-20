//! The snapshots of a round's tasks as `agent status <round>` shows them: the last one of each task, when it was taken and
//! how big it is.

#[cfg(test)]
mod tests;

use agent_core::history::SnapshotRecord;
use time::UtcOffset;

/// One line per task that has a snapshot, showing its last snapshot. `snapshots` is in the order of the history: by task,
/// then by number, so the last of a task is the one that follows the others of it.
pub(crate) fn snapshot_lines(snapshots: &[SnapshotRecord], offset: UtcOffset) -> String {
    let mut lines = Vec::new();
    for (index, snapshot) in snapshots.iter().enumerate() {
        let is_last = snapshots
            .get(index.saturating_add(1))
            .is_none_or(|next| next.task != snapshot.task);
        if !is_last {
            continue;
        }
        lines.push(format!(
            "  snapshot {} number {} at {}, {} bytes\n",
            snapshot.task,
            snapshot.number,
            super::clock_of(snapshot.at, offset),
            snapshot.bytes
        ));
    }
    lines.concat()
}

/// The snapshots of `round` in the history of `home`, by task and then by number; a history that cannot be read is said so
/// in the line that replaces them.
pub(crate) fn snapshots_of(
    home: &agent_runtime::store::Home,
    round: &agent_core::ids::RoundId,
    offset: UtcOffset,
) -> String {
    let read =
        agent_runtime::history::Reader::open(&home.history_file()).and_then(|reader| reader.snapshots_of_round(round));
    match read {
        Ok(snapshots) => snapshot_lines(&snapshots, offset),
        Err(error) => format!(
            "  snapshots: cannot read the history ({})\n",
            agent_runtime::process::error_chain(&error)
        ),
    }
}
