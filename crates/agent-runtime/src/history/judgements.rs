//! The record of a plan step's judgement: whether its check confirmed, declared or was a mismatch.

use std::collections::HashMap;

use rusqlite::{Transaction, params};

use super::codec::{parse_time, time_text};
use super::error::{ReadError, RecordKind, WriteError};
use super::reader::Row_;
use agent_core::ids::{ItemId, RunId, TaskId, TeamId};
use agent_core::plan::judge::JudgementState;
use agent_core::state::Timestamp;

/// One judgement of a step of a task's plan, as the history keeps it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JudgementRecord {
    /// The task whose plan step this is.
    pub task: TaskId,
    /// The task's team.
    pub team: TeamId,
    /// The run at whose end it was judged; `None` when outside a run.
    pub run: Option<RunId>,
    /// When the judgement occurred.
    pub at: Timestamp,
    /// The step ID.
    pub step: ItemId,
    /// The settled state.
    pub state: JudgementState,
    /// The check as written, if any.
    pub check: Option<String>,
    /// Why the check failed or was settled this way, if any.
    pub reason: Option<String>,
    /// How long the check took, in milliseconds.
    pub duration_ms: u64,
    /// Where the output of the check command is stored in the task's files, if any.
    pub output_path: Option<String>,
}

/// The error of an SQL statement of the judgements table.
fn sql(source: rusqlite::Error) -> WriteError {
    WriteError::Sqlite {
        record: RecordKind::Judgement,
        source,
    }
}

/// The error of a query of the judgements rows.
fn query(source: rusqlite::Error) -> ReadError {
    ReadError::Query {
        table: "step_judgements",
        source,
    }
}

/// Writes `records` in `tx`.
///
/// # Errors
/// [`WriteError`]: a row of a task that is not in the history, or a refusal of the database.
pub(super) fn insert(tx: &Transaction<'_>, records: &[JudgementRecord]) -> Result<(), WriteError> {
    for record in records {
        let added = tx
            .execute(
                "INSERT INTO step_judgements (task, team, run, at, step, state, check_line, reason, duration_ms, output_path) \
                 SELECT ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10 FROM runs WHERE task = ?1 LIMIT 1",
                params![
                    record.task.as_str(),
                    record.team.as_str(),
                    record.run.as_ref().map(RunId::as_str),
                    time_text(record.at),
                    record.step.to_string(),
                    record.state.as_str(),
                    record.check.as_deref(),
                    record.reason.as_deref(),
                    i64::try_from(record.duration_ms).unwrap_or(i64::MAX),
                    record.output_path.as_deref(),
                ],
            )
            .map_err(sql)?;
        if added == 0 {
            return Err(WriteError::Rule {
                record: RecordKind::Judgement,
                rule: "a judgement is kept for a task whose run the history has",
            });
        }
    }
    Ok(())
}

/// The judgements of a task, oldest first.
///
/// # Errors
/// [`ReadError`] if a row or the database cannot be read.
pub(super) fn read(snapshot: &Transaction<'_>, task: &TaskId) -> Result<Vec<JudgementRecord>, ReadError> {
    if !has_table(snapshot)? {
        return Ok(Vec::new());
    }
    let mut statement = snapshot
        .prepare(
            "SELECT task, team, run, at, step, state, check_line, reason, duration_ms, output_path, id \
             FROM step_judgements WHERE task = ?1 ORDER BY id",
        )
        .map_err(query)?;
    let mut rows = statement.query(params![task.as_str()]).map_err(query)?;
    let mut records = Vec::new();
    while let Some(row) = rows.next().map_err(query)? {
        records.push(decode(row)?);
    }
    Ok(records)
}

/// The latest judgement for each step of `task`.
///
/// # Errors
/// [`ReadError`] if a row or the database cannot be read.
pub(super) fn latest(snapshot: &Transaction<'_>, task: &TaskId) -> Result<HashMap<ItemId, JudgementRecord>, ReadError> {
    let all = read(snapshot, task)?;
    let mut map = HashMap::new();
    for record in all {
        map.insert(record.step, record);
    }
    Ok(map)
}

/// Whether the database has `step_judgements`: a file written before migration 16 has none.
pub(super) fn has_table(snapshot: &Transaction<'_>) -> Result<bool, ReadError> {
    snapshot
        .query_row(
            "SELECT count(*) FROM sqlite_master WHERE type = 'table' AND name = 'step_judgements'",
            [],
            |row| row.get::<_, i64>(0),
        )
        .map(|count| count > 0)
        .map_err(query)
}

/// Decodes one row of `step_judgements`.
fn decode(row: &rusqlite::Row<'_>) -> Result<JudgementRecord, ReadError> {
    let id: i64 = row.get(10).map_err(query)?;
    let here = Row_::of("step_judgements", id.to_string());
    let task = here.value("task", row.get::<_, String>(0).map_err(query)?.parse::<TaskId>())?;
    let team = here.value("team", row.get::<_, String>(1).map_err(query)?.parse::<TeamId>())?;
    let run = row
        .get::<_, Option<String>>(2)
        .map_err(query)?
        .map(|run| here.value("run", run.parse::<RunId>()))
        .transpose()?;
    let at = here.value("at", parse_time(&row.get::<_, String>(3).map_err(query)?))?;
    let step = here.value("step", row.get::<_, String>(4).map_err(query)?.parse::<ItemId>())?;
    let state_str = row.get::<_, String>(5).map_err(query)?;
    let state = match state_str.as_str() {
        "confirmed" => JudgementState::Confirmed,
        "declared" => JudgementState::Declared,
        "mismatch" => JudgementState::Mismatch,
        _ => return Err(here.rule("state", "must be confirmed, declared or mismatch")),
    };
    let check = row.get::<_, Option<String>>(6).map_err(query)?;
    let reason = row.get::<_, Option<String>>(7).map_err(query)?;
    let duration_raw = row.get::<_, i64>(8).map_err(query)?;
    let duration_ms = u64::try_from(duration_raw).unwrap_or(0);
    let output_path = row.get::<_, Option<String>>(9).map_err(query)?;
    Ok(JudgementRecord {
        task,
        team,
        run,
        at,
        step,
        state,
        check,
        reason,
        duration_ms,
        output_path,
    })
}
