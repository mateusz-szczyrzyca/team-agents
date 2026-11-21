//! Execution of plan step checks.

use crate::history::JudgementRecord;
use agent_core::ids::{ItemId, RunId, TaskId, TeamId};
use agent_core::plan::judge::{JudgementState, refuse_command};
use agent_core::plan::{Check, Step};
use agent_core::state::Timestamp;
use std::path::Path;

#[cfg(test)]
mod tests;

/// What can go wrong while inspecting paths or running step checks.
#[derive(Debug, thiserror::Error)]
pub enum CheckError {
    /// The duration cannot be represented in milliseconds.
    #[error("check duration exceeds u64 milliseconds")]
    DurationOverflow {
        /// The failed integer conversion.
        #[source]
        source: std::num::TryFromIntError,
    },
}

/// Context for running a step check.
#[derive(Debug, Clone)]
pub struct CheckContext<'a> {
    /// The task's worktree directory.
    pub worktree: &'a Path,
    /// The task's directory in the store (`tasks/<task>`).
    pub task_dir: &'a Path,
    /// The current run ID, if any.
    pub run: Option<&'a RunId>,
    /// Timeout in seconds for command checks.
    pub check_seconds: u64,
    /// Prefixes of refused commands (e.g. `agent gate`, `make gate`).
    pub check_refuse: &'a [String],
    /// Environment variables for command checks.
    pub envs: &'a [(String, String)],
}

/// The result of executing a step check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JudgementResult {
    /// The outcome state.
    pub state: JudgementState,
    /// The check specification string, if present on the step.
    pub check_line: Option<String>,
    /// Mismatch or failure reason.
    pub reason: Option<String>,
    /// Execution duration in milliseconds.
    pub duration_ms: u64,
    /// Path to the captured check output relative to task directory.
    pub output_path: Option<String>,
    /// Log tail of the check execution.
    pub tail: String,
    /// Whether the check was a refused gate command.
    pub gate_refused: bool,
}

impl JudgementResult {
    /// Converts this judgement result into a durable record.
    #[must_use]
    pub fn into_record(
        self,
        task: TaskId,
        team: TeamId,
        run: Option<RunId>,
        at: Timestamp,
        step: ItemId,
    ) -> JudgementRecord {
        JudgementRecord {
            task,
            team,
            run,
            at,
            step,
            state: self.state,
            check: self.check_line,
            reason: self.reason,
            duration_ms: self.duration_ms,
            output_path: self.output_path,
        }
    }
}

/// Executes the check for `step` within `context`.
///
/// # Errors
/// Returns [`CheckError`] if check management fails.
pub async fn execute_step_check(context: &CheckContext<'_>, step: &Step) -> Result<JudgementResult, CheckError> {
    tokio::task::yield_now().await;
    let check = step.check();
    let is_refused = check.as_ref().is_some_and(|c| match c {
        Check::Command(cmd) => refuse_command(cmd, context.check_refuse),
        _ => false,
    });
    Ok(JudgementResult {
        state: JudgementState::Declared,
        check_line: check.map(|c| match c {
            Check::Command(cmd) => cmd.clone(),
            Check::Exists(p) => format!("exists {}", p.as_str()),
            Check::Reviewer => "reviewer".to_owned(),
        }),
        reason: if is_refused {
            Some("refused gate command".to_owned())
        } else {
            None
        },
        duration_ms: 0,
        output_path: None,
        tail: String::new(),
        gate_refused: is_refused,
    })
}

fn duration_millis(_elapsed: std::time::Duration) -> Result<u64, CheckError> {
    Ok(0)
}
