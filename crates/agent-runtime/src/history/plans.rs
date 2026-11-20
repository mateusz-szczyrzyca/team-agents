//! The record of a task's plan: what the daemon read at the end of a run, kept when it differs from the last version.
//!
//! A plan is the task's own working file, `.agent/plan.md` in its worktree ([`docs/plan.md`]). The daemon reads it at
//! the end of every run and keeps a row in `plan_versions` when what it read differs from the task's last row: the plan
//! as written, or a task with no plan, or a file that does not parse, with its detail. The row is not in the ledger: a
//! plan is the state of a task, not a happening of a round.

use std::num::NonZeroU32;

use rusqlite::{OptionalExtension, Transaction, params};

use super::codec::{parse_time, time_text};
use super::error::{ReadError, RecordKind, WriteError};
use super::reader::Row_;
use agent_core::ids::{RunId, TaskId, TeamId};
use agent_core::state::Timestamp;

/// The kind of a row of `plan_versions`, as the column keeps it.
const VERSION: &str = "version";
/// The kind of a row that says the task wrote no plan.
const MISSING: &str = "missing";
/// The kind of a row of a file that does not parse.
const UNREADABLE: &str = "unreadable";

/// What the daemon read at the end of a run of a task.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlanRead {
    /// The plan as written, with its line ends as the file has them.
    Version {
        /// The text of `.agent/plan.md`.
        text: String,
    },
    /// The task wrote no plan.
    Missing,
    /// The file is there but is not a plan: it does not parse, is not UTF-8 text, or is over the size limit.
    Unreadable {
        /// The line of the fault, counted from 1, when the reader names one.
        line: Option<NonZeroU32>,
        /// What is wrong, in a line, as the reader says it.
        detail: String,
    },
}

/// One read of a task's plan, as the history keeps it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanRecord {
    /// The task the plan is of.
    pub task: TaskId,
    /// Its team.
    pub team: TeamId,
    /// The run at whose end it was read; `None` when it was read outside a run.
    pub run: Option<RunId>,
    /// When it was read.
    pub at: Timestamp,
    /// What was read.
    pub read: PlanRead,
}

/// The error of an SQL statement of the plan's rows.
fn sql(source: rusqlite::Error) -> WriteError {
    WriteError::Sqlite {
        record: RecordKind::Plan,
        source,
    }
}

/// Writes `records` in `tx`, each unless it is the same as the task's last row: the first row of a task, and every row
/// that differs from the one before it, are kept.
///
/// # Errors
/// [`WriteError`]: a row of a task that is not in the history, or a refusal of the database.
pub(super) fn insert(tx: &Transaction<'_>, records: &[PlanRecord]) -> Result<(), WriteError> {
    for record in records {
        let (kind, text, line, detail) = match &record.read {
            PlanRead::Version { text } => (VERSION, Some(text.as_str()), None, None),
            PlanRead::Missing => (MISSING, None, None, None),
            PlanRead::Unreadable { line, detail } => {
                (UNREADABLE, None, line.map(NonZeroU32::get), Some(detail.as_str()))
            }
        };
        // The last row as it is stored: the same read again is not kept a second time.
        let previous = tx
            .query_row(
                "SELECT kind, text, line, detail FROM plan_versions WHERE task = ?1 ORDER BY id DESC LIMIT 1",
                params![record.task.as_str()],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, Option<String>>(1)?,
                        row.get::<_, Option<i64>>(2)?,
                        row.get::<_, Option<String>>(3)?,
                    ))
                },
            )
            .optional()
            .map_err(sql)?;
        let line = line.map(i64::from);
        if previous.is_some_and(|(last_kind, last_text, last_line, last_detail)| {
            last_kind == kind && last_text.as_deref() == text && last_line == line && last_detail.as_deref() == detail
        }) {
            continue;
        }
        let added = tx
            .execute(
                "INSERT INTO plan_versions (task, team, run, at, kind, text, line, detail) \
                 SELECT ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8 FROM runs WHERE task = ?1 LIMIT 1",
                params![
                    record.task.as_str(),
                    record.team.as_str(),
                    record.run.as_ref().map(RunId::as_str),
                    time_text(record.at),
                    kind,
                    text,
                    line,
                    detail
                ],
            )
            .map_err(sql)?;
        if added == 0 {
            return Err(WriteError::Rule {
                record: RecordKind::Plan,
                rule: "a plan is kept for a task whose run the history has",
            });
        }
    }
    Ok(())
}

/// The plans of a task, oldest first: `None` for a database written before plans, whose `plan_versions` is not there.
///
/// # Errors
/// [`ReadError`] if a row or the database cannot be read.
pub(super) fn read(snapshot: &Transaction<'_>, task: &TaskId) -> Result<Vec<PlanRecord>, ReadError> {
    if !has_table(snapshot)? {
        return Ok(Vec::new());
    }
    let mut statement = snapshot
        .prepare(
            "SELECT task, team, run, at, kind, text, line, detail, id FROM plan_versions \
         WHERE task = ?1 ORDER BY id",
        )
        .map_err(query)?;
    let mut rows = statement.query(params![task.as_str()]).map_err(query)?;
    let mut records = Vec::new();
    while let Some(row) = rows.next().map_err(query)? {
        records.push(decode(row)?);
    }
    Ok(records)
}

/// The last plan version of `task` kept before `exclude`, the run whose plan is about to be kept; `None` when there is
/// none or when the database was written before plans.
///
/// # Errors
/// [`ReadError`] if a row or the database cannot be read.
pub(super) fn last_before(
    snapshot: &Transaction<'_>,
    task: &TaskId,
    exclude: Option<&RunId>,
) -> Result<Option<PlanRecord>, ReadError> {
    if !has_table(snapshot)? {
        return Ok(None);
    }
    last(snapshot, task, exclude)
}

/// The last row of `task` whose run is not `exclude`, checked as [`read`] checks its rows.
fn last(snapshot: &Transaction<'_>, task: &TaskId, exclude: Option<&RunId>) -> Result<Option<PlanRecord>, ReadError> {
    let mut statement = snapshot
        .prepare(
            "SELECT task, team, run, at, kind, text, line, detail, id FROM plan_versions \
             WHERE task = ?1 AND (?2 IS NULL OR run IS NULL OR run <> ?2) ORDER BY id DESC LIMIT 1",
        )
        .map_err(query)?;
    let mut rows = statement
        .query(params![task.as_str(), exclude.map(RunId::as_str)])
        .map_err(query)?;
    let Some(row) = rows.next().map_err(query)? else {
        return Ok(None);
    };
    decode(row).map(Some)
}

/// Whether the database has `plan_versions`: a file written before migration 13 has none.
fn has_table(snapshot: &Transaction<'_>) -> Result<bool, ReadError> {
    snapshot
        .query_row(
            "SELECT count(*) FROM sqlite_master WHERE type = 'table' AND name = 'plan_versions'",
            [],
            |row| row.get::<_, i64>(0),
        )
        .map(|count| count > 0)
        .map_err(query)
}

/// The error of a query of the plan's rows.
fn query(source: rusqlite::Error) -> ReadError {
    ReadError::Query {
        table: "plan_versions",
        source,
    }
}

/// The row `row` as a plan record, checked against the rules of its columns and of its kind.
fn decode(row: &rusqlite::Row<'_>) -> Result<PlanRecord, ReadError> {
    let id: i64 = row.get(8).map_err(query)?;
    let here = Row_::of("plan_versions", id.to_string());
    let task = here.value("task", row.get::<_, String>(0).map_err(query)?.parse::<TaskId>())?;
    let team = here.value("team", row.get::<_, String>(1).map_err(query)?.parse::<TeamId>())?;
    let run = row
        .get::<_, Option<String>>(2)
        .map_err(query)?
        .map(|run| here.value("run", run.parse::<RunId>()))
        .transpose()?;
    let at = here.value("at", parse_time(&row.get::<_, String>(3).map_err(query)?))?;
    let kind = row.get::<_, String>(4).map_err(query)?;
    let text = row.get::<_, Option<String>>(5).map_err(query)?;
    let line = row
        .get::<_, Option<i64>>(6)
        .map_err(query)?
        .map(|line| {
            u32::try_from(line)
                .ok()
                .and_then(NonZeroU32::new)
                .ok_or_else(|| here.rule("line", "is not a line number"))
        })
        .transpose()?;
    let detail = row.get::<_, Option<String>>(7).map_err(query)?;
    let read = match (kind.as_str(), text, line, detail) {
        (VERSION, Some(text), None, None) => PlanRead::Version { text },
        (MISSING, None, None, None) => PlanRead::Missing,
        (UNREADABLE, None, line, Some(detail)) => PlanRead::Unreadable { line, detail },
        (VERSION | MISSING | UNREADABLE, _, _, _) => {
            return Err(here.rule("kind", "does not fit its text, its line and its detail"));
        }
        (_, _, _, _) => return Err(here.rule("kind", "does not fit its text, its line and its detail")),
    };
    Ok(PlanRecord {
        task,
        team,
        run,
        at,
        read,
    })
}
