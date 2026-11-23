//! What a run's stream told of its context and cost: the `run_usage` table, kept outside the ledger like judgements.

use agent_core::cli::RunUsage;
use agent_core::ids::RunId;
use rusqlite::{Transaction, params};

use super::error::{RecordKind, WriteError};

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
