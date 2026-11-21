//! The record of a plan step's judgement: whether its check confirmed, declared or was a mismatch.

use std::collections::HashMap;

use rusqlite::Transaction;

use super::error::{ReadError, WriteError};
use agent_core::ids::{ItemId, RunId, TaskId, TeamId};
use agent_core::plan::judge::JudgementState;
use agent_core::state::Timestamp;

/// One judgement of a step of a task's plan, as the history keeps it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JudgementRecord {
    /// The task whose step was judged.
    pub task: TaskId,
    /// The team of the task.
    pub team: TeamId,
    /// The run at whose end the judgement was made, if made in a run.
    pub run: Option<RunId>,
    /// When the judgement was made.
    pub at: Timestamp,
    /// The step that was judged.
    pub step: ItemId,
    /// The state of the judgement.
    pub state: JudgementState,
    /// The check command or path, if the step had one.
    pub check: Option<String>,
    /// The reason for a mismatch or refusal.
    pub reason: Option<String>,
    /// How long the check command took, in milliseconds.
    pub duration_ms: u64,
    /// The path of the check's output log file, if any.
    pub output_path: Option<String>,
}

pub(super) fn insert(_tx: &Transaction<'_>, _records: &[JudgementRecord]) -> Result<(), WriteError> {
    Ok(())
}

pub(super) fn read(_snapshot: &Transaction<'_>, _task: &TaskId) -> Result<Vec<JudgementRecord>, ReadError> {
    Ok(Vec::new())
}

pub(super) fn latest(
    _snapshot: &Transaction<'_>,
    _task: &TaskId,
) -> Result<HashMap<ItemId, JudgementRecord>, ReadError> {
    Ok(HashMap::new())
}
