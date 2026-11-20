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
        let previous = last(tx, &record.task, None).map_err(sql)?;
        if previous.as_ref().is_some_and(|last| last.read == record.read) {
            continue;
        }
        let (kind, text, detail) = match &record.read {
            PlanRead::Version { text } => (VERSION, Some(text.as_str()), None),
            PlanRead::Missing => (MISSING, None, None),
            PlanRead::Unreadable { detail, .. } => (UNREADABLE, None, Some(detail.as_str())),
        };
        let added = tx
            .execute(
                "INSERT INTO plan_versions (task, team, run, at, kind, text, detail) \
                 SELECT ?1, ?2, ?3, ?4, ?5, ?6, ?7 FROM runs WHERE task = ?1 LIMIT 1",
                params![
                    record.task.as_str(),
                    record.team.as_str(),
                    record.run.as_ref().map(RunId::as_str),
                    time_text(record.at),
                    kind,
                    text,
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
        .prepare("SELECT task, team, run, at, kind, text, detail, id FROM plan_versions WHERE task = ?1 ORDER BY id")
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
    last(snapshot, task, exclude).map_err(|source| ReadError::Query {
        table: "plan_versions",
        source,
    })
}

/// The last row of `task` whose run is not `exclude`.
fn last(
    snapshot: &Transaction<'_>,
    task: &TaskId,
    exclude: Option<&RunId>,
) -> Result<Option<PlanRecord>, rusqlite::Error> {
    let row = snapshot
        .query_row(
            "SELECT task, team, run, at, kind, text, detail, id FROM plan_versions \
             WHERE task = ?1 AND (?2 IS NULL OR run IS NULL OR run <> ?2) ORDER BY id DESC LIMIT 1",
            params![task.as_str(), exclude.map(RunId::as_str)],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, Option<String>>(5)?,
                    row.get::<_, Option<String>>(6)?,
                    row.get::<_, i64>(7)?,
                ))
            },
        )
        .optional()?;
    row.map(decode_parts).transpose()
}

/// Whether the database has `plan_versions`: a file written before migration 12 has none.
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

/// The row `row` as a plan record.
fn decode(row: &rusqlite::Row<'_>) -> Result<PlanRecord, ReadError> {
    let id: i64 = row.get(7).map_err(query)?;
    let parts = (
        row.get::<_, String>(0).map_err(query)?,
        row.get::<_, String>(1).map_err(query)?,
        row.get::<_, Option<String>>(2).map_err(query)?,
        row.get::<_, String>(3).map_err(query)?,
        row.get::<_, String>(4).map_err(query)?,
        row.get::<_, Option<String>>(5).map_err(query)?,
        row.get::<_, Option<String>>(6).map_err(query)?,
        id,
    );
    decode_parts(parts).map_err(|source| ReadError::Query {
        table: "plan_versions",
        source,
    })
}

/// The columns of a row of `plan_versions` as a plan record, checked against the rules of its kind.
fn decode_parts(
    (task, team, run, at, kind, text, detail, id): (
        String,
        String,
        Option<String>,
        String,
        String,
        Option<String>,
        Option<String>,
        i64,
    ),
) -> Result<PlanRecord, rusqlite::Error> {
    let key = format!("plan_versions {id}");
    let refuse = |column: &'static str, rule: &str| {
        rusqlite::Error::InvalidParameterName(format!("the {column} of {key} {rule}"))
    };
    let task = task
        .parse::<TaskId>()
        .map_err(|_| rusqlite::Error::InvalidParameterName(format!("task of {key}")))?;
    let team = team
        .parse::<TeamId>()
        .map_err(|_| rusqlite::Error::InvalidParameterName(format!("team of {key}")))?;
    let run = run
        .map(|run| run.parse::<RunId>())
        .transpose()
        .map_err(|_| rusqlite::Error::InvalidParameterName(format!("run of {key}")))?;
    let at = parse_time(&at).map_err(|_| rusqlite::Error::InvalidParameterName(format!("at of {key}")))?;
    let read = match (kind.as_str(), text, detail) {
        (VERSION, Some(text), None) => PlanRead::Version { text },
        (MISSING, None, None) => PlanRead::Missing,
        (UNREADABLE, None, Some(detail)) => PlanRead::Unreadable { line: None, detail },
        (VERSION | MISSING | UNREADABLE, _, _) => {
            return Err(refuse("kind", "does not fit its text and detail"));
        }
        (other, _, _) => return Err(refuse("kind", other)),
    };
    Ok(PlanRecord {
        task,
        team,
        run,
        at,
        read,
    })
}
