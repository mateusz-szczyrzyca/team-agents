//! The errors of the history: opening the database, writing a record, reading a row, closing the writer.

use std::path::PathBuf;

use agent_core::config::InvalidValue;
use agent_core::ids::{InvalidId, InvalidItemId, InvalidName};
use agent_core::kinds::UnknownKeyword;
use agent_core::repo_path::InvalidRepoPath;
use agent_core::state::{InvalidCommitId, InvalidTimestamp};

/// Why the database of the history cannot be opened for writing.
#[derive(Debug, thiserror::Error)]
pub enum OpenError {
    /// SQLite cannot open the file, or what it opened is not a database.
    #[error("cannot open the history database {path:?}")]
    Database {
        /// The file.
        path: PathBuf,
        /// What SQLite said.
        source: rusqlite::Error,
    },
    /// The database was written by a newer build: it is not written to.
    #[error(
        "the history database {path:?} is of version {found}, newer than the {supported} this build writes: it is left as it is and nothing is recorded"
    )]
    Newer {
        /// The file.
        path: PathBuf,
        /// The version it has.
        found: u32,
        /// The newest version this build knows.
        supported: u32,
    },
    /// A migration failed; the database stays at the version before it.
    #[error("cannot migrate the history database {path:?} to version {version}")]
    Migrate {
        /// The file.
        path: PathBuf,
        /// The version the migration leads to.
        version: u32,
        /// What SQLite said.
        source: rusqlite::Error,
    },
    /// The list of migrations does not count 1, 2, 3, ...
    #[error(
        "the migrations of the history are not numbered 1, 2, 3, ...: version {found} stands where {expected} belongs"
    )]
    MigrationOrder {
        /// The version found.
        found: u32,
        /// The version expected there.
        expected: u32,
    },
    /// SQLite would not switch the database to write-ahead logging.
    #[error("the history database {path:?} cannot be written ahead: its journal mode is {mode:?}")]
    NotWal {
        /// The file.
        path: PathBuf,
        /// The journal mode it stays in.
        mode: String,
    },
}

/// Which record a write was about, for the message of an error.
#[derive(Debug, Clone, Copy, PartialEq, Eq, strum::Display)]
#[strum(serialize_all = "snake_case")]
pub enum RecordKind {
    /// A team starting.
    Team,
    /// A round opening.
    Round,
    /// A run starting.
    Run,
    /// The process of a run starting.
    Launch,
    /// A step of a run.
    Step,
    /// A run ending.
    RunEnd,
    /// What a run changed.
    Changes,
}

/// How a write failed, as far as anyone can act on it: the writer reports each kind once.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, strum::Display)]
#[strum(serialize_all = "snake_case")]
pub enum WriteKind {
    /// The disk is full.
    DiskFull,
    /// The database is locked by another process for longer than the writer waits.
    Busy,
    /// The database or its directory is read-only.
    ReadOnly,
    /// The system failed to read or write the file.
    Io,
    /// The database is damaged or is not a database.
    Corrupt,
    /// The database refuses the record: a rule of its schema is broken (a step of a run that has no row, for one).
    Refused,
    /// The record cannot be turned into rows: a number out of range, a value that is not JSON.
    Encoding,
    /// Anything else SQLite reports.
    Other,
}

/// Why a record cannot be written.
#[derive(Debug, thiserror::Error)]
pub enum WriteError {
    /// SQLite failed.
    #[error("cannot write a {record} record to the history")]
    Sqlite {
        /// The record.
        record: RecordKind,
        /// What SQLite said.
        source: rusqlite::Error,
    },
    /// A list of the record cannot be written as JSON.
    #[error("cannot write a list of a {record} record as JSON")]
    Json {
        /// The record.
        record: RecordKind,
        /// What the serializer said.
        source: serde_json::Error,
    },
    /// A number of the record is out of the range the database holds.
    #[error("a number of a {record} record is out of range")]
    Number {
        /// The record.
        record: RecordKind,
        /// What the conversion said.
        source: std::num::TryFromIntError,
    },
}

impl WriteError {
    /// How the write failed.
    #[must_use]
    pub fn kind(&self) -> WriteKind {
        match self {
            Self::Sqlite { source, .. } => sqlite_kind(source),
            Self::Json { .. } | Self::Number { .. } => WriteKind::Encoding,
        }
    }
}

/// The kind of failure an SQLite error is.
fn sqlite_kind(error: &rusqlite::Error) -> WriteKind {
    use rusqlite::ErrorCode;
    let Some(code) = error.sqlite_error_code() else {
        return WriteKind::Other;
    };
    match code {
        ErrorCode::DiskFull => WriteKind::DiskFull,
        ErrorCode::DatabaseBusy | ErrorCode::DatabaseLocked => WriteKind::Busy,
        ErrorCode::ReadOnly | ErrorCode::CannotOpen => WriteKind::ReadOnly,
        ErrorCode::SystemIoFailure => WriteKind::Io,
        ErrorCode::DatabaseCorrupt | ErrorCode::NotADatabase => WriteKind::Corrupt,
        ErrorCode::ConstraintViolation => WriteKind::Refused,
        _ => WriteKind::Other,
    }
}

/// A value in a row that is not what its column holds.
#[derive(Debug, thiserror::Error)]
pub enum ValueProblem {
    /// An agent, investigator or other name.
    #[error("not a name")]
    Name(#[from] InvalidName),
    /// A team, round, task or run ID.
    #[error("not an ID")]
    Id(#[from] InvalidId),
    /// An item number.
    #[error("not an item number")]
    Item(#[from] InvalidItemId),
    /// A time.
    #[error("not a time")]
    Time(#[from] InvalidTimestamp),
    /// A keyword of a fixed list.
    #[error("not a known keyword")]
    Keyword(#[from] UnknownKeyword),
    /// A path, a session, a model or another checked text.
    #[error("not a valid text")]
    Text(#[from] InvalidValue),
    /// A commit.
    #[error("not a commit")]
    Commit(#[from] InvalidCommitId),
    /// A path inside a repository.
    #[error("not a path of a repository")]
    RepoPath(#[from] InvalidRepoPath),
    /// A branch.
    #[error("not a branch")]
    Branch(#[from] agent_core::ids::InvalidBranchName),
    /// A list kept as JSON.
    #[error("not a JSON list of texts")]
    Json(#[from] serde_json::Error),
    /// A number outside what its field holds.
    #[error("a number out of range")]
    Number(#[from] std::num::TryFromIntError),
    /// A row that breaks a rule of its record that the columns alone do not carry.
    #[error("{rule}")]
    Rule {
        /// The rule, in words.
        rule: &'static str,
    },
}

/// Why the history cannot be read.
#[derive(Debug, thiserror::Error)]
pub enum ReadError {
    /// SQLite cannot open the file for reading.
    #[error("cannot open the history database {path:?} for reading")]
    Open {
        /// The file.
        path: PathBuf,
        /// What SQLite said.
        source: rusqlite::Error,
    },
    /// The file has no history in it: its version is 0.
    #[error("{path:?} holds no history: a daemon has not written to it yet")]
    Empty {
        /// The file.
        path: PathBuf,
    },
    /// The database was written by a newer build.
    #[error("the history database {path:?} is of version {found}; this build reads up to version {supported}")]
    Newer {
        /// The file.
        path: PathBuf,
        /// The version it has.
        found: u32,
        /// The newest version this build reads.
        supported: u32,
    },
    /// A query failed.
    #[error("cannot read the {table} of the history")]
    Query {
        /// The table.
        table: &'static str,
        /// What SQLite said.
        source: rusqlite::Error,
    },
    /// A row holds a value that its column should not.
    #[error("the {column} of a row of {table} ({key}) is not valid")]
    Value {
        /// The table.
        table: &'static str,
        /// The column.
        column: &'static str,
        /// The key of the row, as text.
        key: String,
        /// What is wrong with the value.
        source: ValueProblem,
    },
}

/// Why the writer could not be closed cleanly.
#[derive(Debug, thiserror::Error)]
pub enum CloseError {
    /// The thread of the writer panicked: records may be missing.
    #[error("the thread of the history writer panicked")]
    Panicked,
    /// The queue was already closed: the writer was gone before it was asked to stop.
    #[error("the history writer was gone before it was closed")]
    Gone,
}

/// Why the writer thread could not be started.
#[derive(Debug, thiserror::Error)]
#[error("cannot start the thread of the history writer")]
pub struct StartError(#[source] pub std::io::Error);
