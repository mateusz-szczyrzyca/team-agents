//! The ledger of the history: one row for every record, in the order the records were written in, and the rounds each
//! belongs to. A record of a run or a team keeps its own tables and the ledger refers to it by its kind and key; an event
//! of a round is kept in the ledger itself, as the JSON of its payload.
//!
//! The row of a record is inserted in the transaction of the record, after its own rows, so the ledger never names a
//! record that is not there, and a record that was written before is not written again: the ledger's key is unique for
//! each kind and team.

use agent_core::history::{Cut, EventRecord, HistoryEvent, MaxTextBytes, Record};
use rusqlite::{OptionalExtension, Transaction, params};

use super::codec::time_text;
use super::error::{RecordKind, WriteError};

/// What a record is in the ledger.
struct Entry {
    /// The `kind` column: the keyword of the record or of its event.
    kind: String,
    /// The `key` column: what makes the record the same record when it is written twice; `None` for an event that can
    /// happen again.
    key: Option<String>,
    /// The `at` column, as the database keeps times.
    at: String,
    /// The team.
    team: String,
    /// The rounds, oldest first.
    rounds: Vec<String>,
    /// The JSON of an event's payload; `None` for the records that have tables of their own.
    payload: Option<String>,
    /// The fields of the payload that were cut, as the `cut` column holds them.
    cut: String,
}

/// The team and rounds of a run that is recorded already, for the records that follow it.
struct RunScope {
    /// The team of the run.
    team: String,
    /// The rounds the run works for, oldest first.
    rounds: Vec<String>,
}

/// The kind a ledger error is told as.
const fn kind_of(record: &Record) -> RecordKind {
    match record {
        Record::Team(_) => RecordKind::Team,
        Record::Round(_) => RecordKind::Round,
        Record::Run(_) => RecordKind::Run,
        Record::Launch(_) => RecordKind::Launch,
        Record::Step(_) => RecordKind::Step,
        Record::RunEnd(_) => RecordKind::RunEnd,
        Record::Changes(_) => RecordKind::Changes,
        Record::Session(_) => RecordKind::Session,
        Record::Event(_) => RecordKind::Event,
    }
}

/// The error of an SQL statement of the ledger, for `record`.
fn sql(record: RecordKind) -> impl Fn(rusqlite::Error) -> WriteError {
    move |source| WriteError::Sqlite { record, source }
}

/// Enters `record` in the ledger in `tx`, unless it is there already.
///
/// # Errors
/// [`WriteError`]: the record of a run whose start is not recorded, an event that cannot be turned into JSON, or a
/// refusal of the database.
pub(super) fn append(tx: &Transaction<'_>, record: &Record, limit: MaxTextBytes) -> Result<(), WriteError> {
    let kind = kind_of(record);
    let entry = entry_of(tx, record, limit)?;
    let added = tx
        .execute(
            "INSERT INTO history_records (kind, key, at, team, payload, cut) VALUES (?1, ?2, ?3, ?4, ?5, ?6) \
             ON CONFLICT (kind, team, key) WHERE key IS NOT NULL DO NOTHING",
            params![entry.kind, entry.key, entry.at, entry.team, entry.payload, entry.cut],
        )
        .map_err(sql(kind))?;
    if added == 0 {
        return Ok(());
    }
    let sequence = tx.last_insert_rowid();
    for (index, round) in entry.rounds.iter().enumerate() {
        let position = u32::try_from(index + 1).map_err(|source| WriteError::Number { record: kind, source })?;
        tx.execute(
            "INSERT INTO history_record_rounds (sequence, position, round) VALUES (?1, ?2, ?3)",
            params![sequence, position, round],
        )
        .map_err(sql(kind))?;
    }
    Ok(())
}

/// What `record` is in the ledger; for a record of a run, the team and rounds of the run as they were recorded.
fn entry_of(tx: &Transaction<'_>, record: &Record, limit: MaxTextBytes) -> Result<Entry, WriteError> {
    let kind = kind_of(record);
    let plain = |kind: RecordKind, key: String, at: String, team: String, rounds: Vec<String>| Entry {
        kind: kind.to_string(),
        key: Some(key),
        at,
        team,
        rounds,
        payload: None,
        cut: String::new(),
    };
    let of_run = |run: &agent_core::ids::RunId, key: String, at: String| -> Result<Entry, WriteError> {
        let scope = run_scope(tx, run.as_str(), kind)?;
        Ok(plain(kind, key, at, scope.team, scope.rounds))
    };
    match record {
        Record::Team(team) => Ok(plain(
            kind,
            team.team.to_string(),
            time_text(team.at),
            team.team.to_string(),
            Vec::new(),
        )),
        Record::Round(round) => Ok(plain(
            kind,
            round.round.to_string(),
            time_text(round.at),
            round.team.to_string(),
            vec![round.round.to_string()],
        )),
        Record::Run(run) => Ok(plain(
            kind,
            run.run.to_string(),
            time_text(run.at),
            run.team.to_string(),
            run.rounds.iter().map(ToString::to_string).collect(),
        )),
        Record::Launch(launch) => of_run(&launch.run, launch.run.to_string(), time_text(launch.at)),
        Record::Step(step) => of_run(&step.run, format!("{}/{}", step.run, step.number), time_text(step.at)),
        Record::RunEnd(end) => of_run(&end.run, end.run.to_string(), time_text(end.at)),
        Record::Session(session) => of_run(
            &session.run,
            format!("{}/{}", session.run, session.number),
            time_text(session.at),
        ),
        Record::Changes(changes) => {
            let at = changed_at(tx, changes.run.as_str(), kind)?;
            of_run(&changes.run, changes.run.to_string(), at)
        }
        Record::Event(event) => event_entry(tx, event, limit),
    }
}

/// The rounds of the latest run of `task`; none when the task has no run recorded.
fn rounds_of_task(tx: &Transaction<'_>, task: &str, kind: RecordKind) -> Result<Vec<String>, WriteError> {
    let mut statement = tx
        .prepare(
            "SELECT round FROM run_rounds WHERE run = \
             (SELECT run FROM runs WHERE task = ?1 ORDER BY at DESC, rowid DESC LIMIT 1) ORDER BY position",
        )
        .map_err(sql(kind))?;
    statement
        .query_map(params![task], |row| row.get(0))
        .map_err(sql(kind))?
        .collect::<Result<Vec<String>, _>>()
        .map_err(sql(kind))
}

/// The rounds of the latest run of `team`; none when the team has no run recorded.
fn rounds_of_team(tx: &Transaction<'_>, team: &str, kind: RecordKind) -> Result<Vec<String>, WriteError> {
    let mut statement = tx
        .prepare(
            "SELECT round FROM run_rounds WHERE run = \
             (SELECT run FROM runs WHERE team = ?1 ORDER BY at DESC, rowid DESC LIMIT 1) ORDER BY position",
        )
        .map_err(sql(kind))?;
    statement
        .query_map(params![team], |row| row.get(0))
        .map_err(sql(kind))?
        .collect::<Result<Vec<String>, _>>()
        .map_err(sql(kind))
}

/// When the changes of `run` count as having happened: when it ended, or when it started if no end is recorded.
fn changed_at(tx: &Transaction<'_>, run: &str, kind: RecordKind) -> Result<String, WriteError> {
    tx.query_row(
        "SELECT COALESCE((SELECT at FROM run_ends WHERE run = ?1), (SELECT at FROM runs WHERE run = ?1))",
        params![run],
        |row| row.get::<_, Option<String>>(0),
    )
    .map_err(sql(kind))?
    .ok_or(WriteError::Rule {
        record: kind,
        rule: "the changes of a run need the start of the run recorded before them",
    })
}

/// The team and the rounds of the run `run`, which is recorded in `tx` already.
fn run_scope(tx: &Transaction<'_>, run: &str, kind: RecordKind) -> Result<RunScope, WriteError> {
    let team: Option<String> = tx
        .query_row("SELECT team FROM runs WHERE run = ?1", params![run], |row| row.get(0))
        .optional()
        .map_err(sql(kind))?;
    let team = team.ok_or(WriteError::Rule {
        record: kind,
        rule: "a record of a run needs the start of the run recorded before it",
    })?;
    let mut statement = tx
        .prepare("SELECT round FROM run_rounds WHERE run = ?1 ORDER BY position")
        .map_err(sql(kind))?;
    let rounds = statement
        .query_map(params![run], |row| row.get(0))
        .map_err(sql(kind))?
        .collect::<Result<Vec<String>, _>>()
        .map_err(sql(kind))?;
    Ok(RunScope { team, rounds })
}

/// The ledger entry of an event: its payload with the texts cut at `limit`.
fn event_entry(tx: &Transaction<'_>, record: &EventRecord, limit: MaxTextBytes) -> Result<Entry, WriteError> {
    let kind = RecordKind::Event;
    let mut event = record.event.clone();
    let mut cut = Cut::default();
    for (field, text) in event.texts_mut() {
        *text = cut.keep(field, text, limit);
    }
    let payload = serde_json::to_string(&event).map_err(|source| WriteError::Json { record: kind, source })?;
    // A gate runs where no state of rounds is at hand: it belongs to the rounds of the run of its task that waits for it.
    let rounds = match &event {
        HistoryEvent::Gate { task, .. } if record.rounds.is_empty() => rounds_of_task(tx, task.as_str(), kind)?,
        HistoryEvent::Lost { what, .. } if what == "gate" && record.rounds.is_empty() => {
            rounds_of_team(tx, record.team.as_str(), kind)?
        }
        _ => record.rounds.iter().map(ToString::to_string).collect(),
    };
    Ok(Entry {
        kind: event.kind().to_string(),
        key: event.key(),
        at: time_text(record.at),
        team: record.team.to_string(),
        rounds,
        payload: Some(payload),
        cut: cut.to_string(),
    })
}
