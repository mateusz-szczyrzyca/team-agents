//! What a run's stream told of its context and cost: the `run_usage` table, kept outside the ledger like judgements.

use agent_core::cli::RunUsage;
use agent_core::ids::RunId;
use rusqlite::{OptionalExtension, Transaction, params, params_from_iter};

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
    if !has_table(snapshot)? {
        return Ok(None);
    }
    read_row(snapshot, run)
}

/// Whether the history has the table of usage: one written before migration 22 has none, and none of its runs has a
/// usage, so none is told.
///
/// # Errors
/// [`ReadError::Query`] when the schema cannot be read.
fn has_table(snapshot: &Transaction<'_>) -> Result<bool, ReadError> {
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
    Ok(tables != 0)
}

/// The row of `run` in the table of usage; `None` when it has none.
///
/// # Errors
/// [`ReadError::Query`] when the row cannot be read, [`ReadError::Value`] when a count in it is negative.
fn read_row(snapshot: &Transaction<'_>, run: &RunId) -> Result<Option<RunUsage>, ReadError> {
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

/// How many runs one query of [`read_many`] asks for: far below the number of variables SQLite accepts in a statement,
/// whatever its build.
const RUNS_PER_QUERY: usize = 500;

/// The usage of each of `runs` that has a row, read in the one snapshot of `snapshot` with one query for each
/// [`RUNS_PER_QUERY`] runs; a run without a row is not in the result, and a history without the table has none.
///
/// # Errors
/// [`ReadError::Query`] when a row cannot be read, [`ReadError::Value`] when a count in it is negative.
pub(super) fn read_many(
    snapshot: &Transaction<'_>,
    runs: &[RunId],
) -> Result<std::collections::BTreeMap<RunId, RunUsage>, ReadError> {
    let mut found = std::collections::BTreeMap::new();
    if runs.is_empty() || !has_table(snapshot)? {
        return Ok(found);
    }
    let query_error = |source| ReadError::Query {
        table: "run_usage",
        source,
    };
    for chunk in runs.chunks(RUNS_PER_QUERY) {
        let asked: std::collections::HashMap<&str, &RunId> = chunk.iter().map(|run| (run.as_str(), run)).collect();
        let marks = vec!["?"; chunk.len()].join(", ");
        let mut statement = snapshot
            .prepare(&format!(
                "SELECT run, first_context, last_context, tokens_written, cost_usd FROM run_usage WHERE run IN ({marks})"
            ))
            .map_err(query_error)?;
        let rows = statement
            .query_map(params_from_iter(chunk.iter().map(RunId::as_str)), |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Option<i64>>(1)?,
                    row.get::<_, Option<i64>>(2)?,
                    row.get::<_, Option<i64>>(3)?,
                    row.get::<_, Option<f64>>(4)?,
                ))
            })
            .map_err(query_error)?;
        for row in rows {
            let (id, first, last, written, cost) = row.map_err(query_error)?;
            // The query asked for these runs only: an id that is none of them cannot come back.
            let Some(run) = asked.get(id.as_str()) else {
                continue;
            };
            found.insert(
                (*run).clone(),
                RunUsage {
                    first_context: read_count(run, first)?,
                    last_context: read_count(run, last)?,
                    tokens_written: read_count(run, written)?,
                    cost_usd: cost,
                },
            );
        }
    }
    Ok(found)
}
