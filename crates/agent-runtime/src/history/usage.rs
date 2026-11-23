//! What a run's stream told of its context and cost: the `run_usage` table, kept outside the ledger like judgements.

use agent_core::cli::RunUsage;
use agent_core::ids::RunId;
use rusqlite::{OptionalExtension, Transaction, params};

use super::error::{ReadError, RecordKind, ValueProblem, WriteError};

/// One run's usage as the history keeps it. Each part of `usage` is `None` where the CLI did not tell it.
#[derive(Debug, Clone, PartialEq)]
pub struct RunUsageRecord {
    /// The run.
    pub run: RunId,
    /// What the run's stream showed.
    pub usage: RunUsage,
}

/// The record a usage write is reported as in an error: the run's usage has no kind of its own in the ledger, and it
/// is kept beside the run.
const RECORD: RecordKind = RecordKind::Run;

/// Writes the usage of each run in `records` in `tx`. A run that has its usage already keeps the first one.
///
/// # Errors
/// [`WriteError`]: a count that does not fit the table, or a refusal of the database (a run that the history does not
/// have is refused by its foreign key).
pub(super) fn insert(tx: &Transaction<'_>, records: &[RunUsageRecord]) -> Result<(), WriteError> {
    for record in records {
        let usage = record.usage;
        tx.execute(
            "INSERT INTO run_usage (run, first_context, last_context, tokens_written, cost_usd) \
             VALUES (?1, ?2, ?3, ?4, ?5) ON CONFLICT (run) DO NOTHING",
            params![
                record.run.as_str(),
                count(usage.first_context)?,
                count(usage.last_context)?,
                count(usage.tokens_written)?,
                usage.cost_usd,
            ],
        )
        .map_err(|source| WriteError::Sqlite { record: RECORD, source })?;
    }
    Ok(())
}

/// A count of tokens as the table's integer; `None` stays `None`.
///
/// # Errors
/// [`WriteError::Number`] when the count does not fit an SQLite integer.
fn count(value: Option<u64>) -> Result<Option<i64>, WriteError> {
    value
        .map(|value| i64::try_from(value).map_err(|source| WriteError::Number { record: RECORD, source }))
        .transpose()
}

/// The usage of `run` as the table keeps it; `None` when there is no row for it, or no table, which is "not told".
///
/// # Errors
/// [`ReadError::Query`] when the row cannot be read, [`ReadError::Value`] when a count in it is negative.
pub(super) fn read(snapshot: &Transaction<'_>, run: &RunId) -> Result<Option<RunUsage>, ReadError> {
    // A history written before migration 22 has no table: none of its runs has a usage, and so none is told.
    let tables: i64 = snapshot
        .query_row(
            "SELECT count(*) FROM sqlite_master WHERE type = 'table' AND name = 'run_usage'",
            [],
            |row| row.get(0),
        )
        .map_err(|source| ReadError::Query {
            table: "run_usage",
            source,
        })?;
    if tables == 0 {
        return Ok(None);
    }
    let row = snapshot
        .query_row(
            "SELECT first_context, last_context, tokens_written, cost_usd FROM run_usage WHERE run = ?1",
            params![run.as_str()],
            |row| {
                Ok((
                    row.get::<_, Option<i64>>(0)?,
                    row.get::<_, Option<i64>>(1)?,
                    row.get::<_, Option<i64>>(2)?,
                    row.get::<_, Option<f64>>(3)?,
                ))
            },
        )
        .optional()
        .map_err(|source| ReadError::Query {
            table: "run_usage",
            source,
        })?;
    let Some((first, last, written, cost)) = row else {
        return Ok(None);
    };
    Ok(Some(RunUsage {
        first_context: read_count(run, first)?,
        last_context: read_count(run, last)?,
        tokens_written: read_count(run, written)?,
        cost_usd: cost,
    }))
}

/// A count read from the table: `None` stays `None`, and a negative count is refused, since the table never holds one.
///
/// # Errors
/// [`ReadError::Value`] for a negative count.
fn read_count(run: &RunId, value: Option<i64>) -> Result<Option<u64>, ReadError> {
    value
        .map(|value| {
            u64::try_from(value).map_err(|source| ReadError::Value {
                table: "run_usage",
                column: "count",
                key: run.to_string(),
                source: ValueProblem::Count(source),
            })
        })
        .transpose()
}
