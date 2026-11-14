//! The records of a team or a round in the order they were written in, read through the ledger.
//!
//! A history made before the ledger has none: [`prepare`] gives a read-only connection to such a file two temporary views
//! with the shape of the ledger, built from the same SQL as the migration that makes it, so a story reads an old file as
//! it would read the file after the migration, and the file is not changed.

use std::collections::HashMap;
use std::str::FromStr;

use agent_core::history::{Cut, EventKind, EventRecord, HistoryEvent, Record, RunSession, SequencedRecord};
use agent_core::ids::{RoundId, RunId, TeamId};
use agent_core::kinds::UnknownKeyword;
use agent_core::state::SessionId;
use rusqlite::{Connection, OptionalExtension, Transaction, params};

use super::{
    ReadError, Row_, Stored, count, query, read_changes, read_end, read_launch, read_round, read_run_start, read_steps,
    read_team,
};
use crate::history::codec::parse_time;
use crate::history::error::RecordKind;
use crate::history::schema::{LEDGER_VERSION, SESSIONS_VERSION, ledger_links, ledger_records};

/// The fields of an event's payload that can be cut.
const EVENT_CUTS: &[&str] = &["issues", "title", "body", "report", "findings", "text", "detail"];

/// What a row of the ledger is of.
enum LedgerKind {
    /// A record that has tables of its own.
    Record(RecordKind),
    /// An event, kept in the ledger.
    Event(EventKind),
}

impl FromStr for LedgerKind {
    type Err = UnknownKeyword;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        // The two sets of keywords share a column: a text that is no event is read as a record, and its error is the
        // one reported.
        EventKind::from_str(text)
            .map(Self::Event)
            .or_else(|_| RecordKind::from_str(text).map(Self::Record))
    }
}

/// Makes the ledger readable on `connection`, a database of `version`: a database of the ledger's version has it; an
/// older one gets it as two temporary views, which live as long as the connection and change nothing in the file.
///
/// # Errors
/// What SQLite says when the views cannot be made.
pub(super) fn prepare(connection: &Connection, version: u32) -> Result<(), rusqlite::Error> {
    if version >= LEDGER_VERSION {
        return Ok(());
    }
    let sql = if version >= SESSIONS_VERSION {
        concat!(
            "CREATE TEMP VIEW history_records AS SELECT row_number() OVER (ORDER BY at, rank, key) AS sequence, \
             kind, key, at, team, NULL AS payload, '' AS cut FROM (",
            ledger_records!(with_sessions),
            "); CREATE TEMP VIEW history_record_rounds AS ",
            ledger_links!(),
            ";"
        )
    } else {
        concat!(
            "CREATE TEMP VIEW history_records AS SELECT row_number() OVER (ORDER BY at, rank, key) AS sequence, \
             kind, key, at, team, NULL AS payload, '' AS cut FROM (",
            ledger_records!(without_sessions),
            "); CREATE TEMP VIEW history_record_rounds AS ",
            ledger_links!(),
            ";"
        )
    };
    connection.execute_batch(sql)
}

/// A row of the ledger as the database has it, with the rounds it is linked to.
struct Raw {
    /// The place in the order of writes.
    sequence: i64,
    /// The `kind` column.
    kind: String,
    /// The `key` column.
    key: Option<String>,
    /// The `at` column.
    at: String,
    /// The `team` column.
    team: String,
    /// The `payload` column.
    payload: Option<String>,
    /// The `cut` column.
    cut: String,
    /// The rounds of the links, in their order.
    rounds: Vec<String>,
}

/// The columns of a row of the ledger with one of its links; the rows of a record are together and its links are in
/// order.
const SELECT: &str = "SELECT h.sequence, h.kind, h.key, h.at, h.team, h.payload, h.cut, r.round \
     FROM history_records AS h LEFT JOIN history_record_rounds AS r ON r.sequence = h.sequence";

/// Every record of `team`, in the order they were written.
///
/// # Errors
/// [`ReadError`]: a query fails, or a row is not what the ledger says it is.
pub(super) fn team_records(snapshot: &Transaction<'_>, team: &TeamId) -> Result<Vec<SequencedRecord>, ReadError> {
    let sql = format!("{SELECT} WHERE h.team = ?1 ORDER BY h.sequence, r.position");
    decode_all(snapshot, &sql, params![team.as_str()])
}

/// Every record of `round` and the start of its team, in the order they were written; none when no record is linked to
/// the round.
///
/// # Errors
/// [`ReadError`]: a query fails, or a row is not what the ledger says it is.
pub(super) fn round_records(snapshot: &Transaction<'_>, round: &RoundId) -> Result<Vec<SequencedRecord>, ReadError> {
    let sql = format!(
        "{SELECT} WHERE h.sequence IN (SELECT sequence FROM history_record_rounds WHERE round = ?1) \
         OR (h.kind = ?2 AND h.team = (SELECT hr.team FROM history_records AS hr \
             JOIN history_record_rounds AS rr ON rr.sequence = hr.sequence WHERE rr.round = ?1 LIMIT 1)) \
         ORDER BY h.sequence, r.position"
    );
    let found = decode_all(snapshot, &sql, params![round.as_str(), RecordKind::Team.to_string()])?;
    // A round with no record linked to it is no round: the start of a team alone is not its story.
    if found.iter().all(|stored| stored.rounds.is_empty()) {
        return Ok(Vec::new());
    }
    Ok(found)
}

/// The rows `sql` gives, folded into one [`Raw`] for each record and decoded.
fn decode_all(
    snapshot: &Transaction<'_>,
    sql: &str,
    arguments: impl rusqlite::Params,
) -> Result<Vec<SequencedRecord>, ReadError> {
    let table = "history_records";
    let mut statement = snapshot.prepare(sql).map_err(query(table))?;
    let mut rows = statement.query(arguments).map_err(query(table))?;
    let mut found: Vec<Raw> = Vec::new();
    while let Some(row) = rows.next().map_err(query(table))? {
        let sequence: i64 = row.get(0).map_err(query(table))?;
        let round: Option<String> = row.get(7).map_err(query(table))?;
        if let Some(last) = found.last_mut().filter(|last| last.sequence == sequence) {
            last.rounds.extend(round);
            continue;
        }
        found.push(Raw {
            sequence,
            kind: row.get(1).map_err(query(table))?,
            key: row.get(2).map_err(query(table))?,
            at: row.get(3).map_err(query(table))?,
            team: row.get(4).map_err(query(table))?,
            payload: row.get(5).map_err(query(table))?,
            cut: row.get(6).map_err(query(table))?,
            rounds: round.into_iter().collect(),
        });
    }
    let mut steps = HashMap::new();
    found.iter().map(|raw| decode(snapshot, raw, &mut steps)).collect()
}

/// The steps of the runs read so far, indexed by step number so that a run's steps are read once in linear time.
type StepCache = HashMap<RunId, HashMap<u32, Stored<agent_core::history::Step>>>;

/// The record a row of the ledger stands for.
fn decode(snapshot: &Transaction<'_>, raw: &Raw, steps: &mut StepCache) -> Result<SequencedRecord, ReadError> {
    let here = Row_::of("history_records", raw.sequence.to_string());
    let at = here.value("at", parse_time(&raw.at))?;
    let team = here.value("team", TeamId::from_str(&raw.team))?;
    let rounds = raw
        .rounds
        .iter()
        .map(|round| here.value("round", RoundId::from_str(round)))
        .collect::<Result<Vec<_>, _>>()?;
    check_round_owners(snapshot, &here, &raw.team, &raw.rounds)?;
    let sequence = here.value("sequence", u64::try_from(raw.sequence))?;
    let kind = here.value("kind", LedgerKind::from_str(&raw.kind))?;
    let (record, cut) = match kind {
        LedgerKind::Event(kind) => {
            let event = event_of(&here, raw, kind)?;
            let cut = here.cut(&raw.cut, EVENT_CUTS)?;
            let record = Record::Event(Box::new(EventRecord {
                at,
                team: team.clone(),
                rounds: rounds.clone(),
                event,
            }));
            (record, cut)
        }
        LedgerKind::Record(kind) => {
            here.absent("payload", raw.payload.as_ref())?;
            record_of(snapshot, &here, raw, kind, steps)?
        }
    };
    Ok(SequencedRecord {
        sequence,
        at,
        team,
        rounds,
        record,
        cut,
    })
}

/// Checks that none of `rounds` belongs to a team other than `team` in `rounds` or in the ledger.
fn check_round_owners(snapshot: &Transaction<'_>, here: &Row_, team: &str, rounds: &[String]) -> Result<(), ReadError> {
    for round in rounds {
        let foreign: Option<i64> = snapshot
            .query_row(
                "SELECT 1 FROM rounds WHERE round = ?1 AND team <> ?2 \
                 UNION ALL \
                 SELECT 1 FROM history_record_rounds AS l \
                 JOIN history_records AS r ON r.sequence = l.sequence \
                 WHERE l.round = ?1 AND r.team <> ?2 \
                 LIMIT 1",
                params![round, team],
                |row| row.get(0),
            )
            .optional()
            .map_err(query("history_record_rounds"))?;
        if foreign.is_some() {
            return Err(here.rule("round", "a record is linked to a round of another team"));
        }
    }
    Ok(())
}

/// The event in the payload of a row of `kind`.
fn event_of(here: &Row_, raw: &Raw, kind: EventKind) -> Result<HistoryEvent, ReadError> {
    let payload = here.needed("payload", raw.payload.as_deref())?;
    let event: HistoryEvent = here.value("payload", serde_json::from_str(payload))?;
    if event.kind() != kind {
        return Err(here.rule("payload", "the payload is of another kind than the row"));
    }
    Ok(event)
}

/// The error of a row whose key names a record its table does not have.
fn missing(here: &Row_) -> ReadError {
    here.rule("key", "the ledger names a record that its table does not have")
}

/// The record a row of the ledger of `kind` refers to, read from its own tables, with the fields cut.
fn record_of(
    snapshot: &Transaction<'_>,
    here: &Row_,
    raw: &Raw,
    kind: RecordKind,
    steps: &mut StepCache,
) -> Result<(Record, Cut), ReadError> {
    let key = here.needed("key", raw.key.as_deref())?;
    match kind {
        RecordKind::Team => {
            let found = read_team(snapshot, &here.value("key", TeamId::from_str(key))?)?;
            let Stored { value, cut } = found.ok_or_else(|| missing(here))?;
            Ok((Record::Team(Box::new(value)), cut))
        }
        RecordKind::Round => {
            let found = read_round(snapshot, &here.value("key", RoundId::from_str(key))?)?;
            let Stored { value, cut } = found.ok_or_else(|| missing(here))?;
            Ok((Record::Round(Box::new(value)), cut))
        }
        RecordKind::Run => {
            let found = read_run_start(snapshot, &here.value("key", RunId::from_str(key))?)?;
            let Stored { value, cut } = found.ok_or_else(|| missing(here))?;
            Ok((Record::Run(Box::new(value)), cut))
        }
        RecordKind::Launch => {
            let found = read_launch(snapshot, &here.value("key", RunId::from_str(key))?)?;
            let Stored { value, cut } = found.ok_or_else(|| missing(here))?;
            Ok((Record::Launch(Box::new(value)), cut))
        }
        RecordKind::RunEnd => {
            let found = read_end(snapshot, &here.value("key", RunId::from_str(key))?)?;
            let Stored { value, cut } = found.ok_or_else(|| missing(here))?;
            Ok((Record::RunEnd(Box::new(value)), cut))
        }
        RecordKind::Changes => {
            let found = read_changes(snapshot, &here.value("key", RunId::from_str(key))?)?;
            let Stored { value, cut } = found.ok_or_else(|| missing(here))?;
            Ok((Record::Changes(Box::new(value)), cut))
        }
        RecordKind::Step => {
            let (run, number) = numbered(here, key)?;
            if !steps.contains_key(&run) {
                let by_number = read_steps(snapshot, &run)?
                    .into_iter()
                    .map(|step| (step.value.number.get(), step))
                    .collect();
                steps.insert(run.clone(), by_number);
            }
            let found = steps
                .get(&run)
                .and_then(|by_number| by_number.get(&number))
                .ok_or_else(|| missing(here))?;
            Ok((Record::Step(Box::new(found.value.clone())), found.cut.clone()))
        }
        RecordKind::Session => {
            let (run, number) = numbered(here, key)?;
            Ok((
                Record::Session(Box::new(read_session(snapshot, &run, number)?)),
                Cut::default(),
            ))
        }
        RecordKind::Event => Err(here.rule("kind", "an event is kept under the keyword of its kind")),
    }
}

/// The run and the number of a key `run/number`.
fn numbered(here: &Row_, key: &str) -> Result<(RunId, u32), ReadError> {
    let (run, number) = key
        .split_once('/')
        .ok_or_else(|| here.rule("key", "the key of a step or a session is a run and a number"))?;
    let number = here.value("key", number.parse::<u32>())?;
    Ok((here.value("key", RunId::from_str(run))?, number))
}

/// The session `number` of `run`, as it was observed.
fn read_session(snapshot: &Transaction<'_>, run: &RunId, number: u32) -> Result<RunSession, ReadError> {
    let table = "run_sessions";
    let (at, session): (String, String) = snapshot
        .query_row(
            "SELECT at, session FROM run_sessions WHERE run = ?1 AND n = ?2",
            params![run.as_str(), number],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .map_err(query(table))?;
    let here = Row_::of(table, format!("{run}/{number}"));
    let number = std::num::NonZeroU32::new(count(&here, "n", i64::from(number))?)
        .ok_or_else(|| here.rule("n", "a session number starts at one"))?;
    Ok(RunSession {
        run: run.clone(),
        number,
        at: here.value("at", parse_time(&at))?,
        session: here.value("session", SessionId::from_str(&session))?,
    })
}
