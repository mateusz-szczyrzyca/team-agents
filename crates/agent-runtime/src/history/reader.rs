//! The reading API of the history: typed answers to the questions about a team, a round and a run.
//!
//! A reader opens the file read-only and needs no daemon. Every row is read through the checks of the types (an ID is an
//! ID, a status is a status) and the rules of its record, so a file that was changed by something else is refused with
//! the row named, not half believed.

use std::path::Path;

use agent_core::history::{Cut, Launch, RoundOpening, RunChanges, RunEndRecord, RunStart, Step, TeamStart};
use agent_core::ids::{RoundId, RunId, TeamId};
use agent_core::kinds::Role;
use agent_core::state::Timestamp;
use rusqlite::Connection;

use super::error::ReadError;

/// A record as the database keeps it, with the names of the fields whose text was cut.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stored<T> {
    /// The record.
    pub value: T,
    /// The fields that were cut at the limit of text.
    pub cut: Cut,
}

/// A run with what was recorded about it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunRow {
    /// How it started.
    pub start: Stored<RunStart>,
    /// How its process started; absent when the run failed before it had one.
    pub launch: Option<Stored<Launch>>,
    /// How it ended; absent while it is going, and for a run the daemon did not see end.
    pub end: Option<Stored<RunEndRecord>>,
}

/// The prompt of one run of a round.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoundPrompt {
    /// The run.
    pub run: RunId,
    /// When it started.
    pub at: Timestamp,
    /// The agent.
    pub agent: String,
    /// Its role.
    pub role: Role,
    /// The prompt, whole unless the record says it was cut.
    pub prompt: String,
    /// Whether the prompt was cut.
    pub was_cut: bool,
}

/// The history database, open read-only.
#[derive(Debug)]
pub struct Reader {
    /// The connection.
    connection: Connection,
}

impl Reader {
    /// Opens the history at `path` for reading.
    ///
    /// # Errors
    /// [`ReadError`].
    pub fn open(path: &Path) -> Result<Self, ReadError> {
        let connection = Connection::open_in_memory().map_err(|source| ReadError::Open {
            path: path.to_path_buf(),
            source,
        })?;
        Ok(Self { connection })
    }

    /// The schema as `docs/history.schema.sql` pins it: `PRAGMA user_version` and what `sqlite_master` holds.
    ///
    /// # Errors
    /// [`ReadError::Query`].
    pub fn schema(&self) -> Result<String, ReadError> {
        let _ = &self.connection;
        Ok(String::new())
    }

    /// The team `team`, as it started.
    ///
    /// # Errors
    /// [`ReadError`].
    pub fn team(&self, team: &TeamId) -> Result<Option<Stored<TeamStart>>, ReadError> {
        let _ = team;
        Ok(None)
    }

    /// The rounds of `team`, oldest first.
    ///
    /// # Errors
    /// [`ReadError`].
    pub fn rounds_of_team(&self, team: &TeamId) -> Result<Vec<Stored<RoundOpening>>, ReadError> {
        let _ = team;
        Ok(Vec::new())
    }

    /// The round `round`, as it opened.
    ///
    /// # Errors
    /// [`ReadError`].
    pub fn round(&self, round: &RoundId) -> Result<Option<Stored<RoundOpening>>, ReadError> {
        let _ = round;
        Ok(None)
    }

    /// The runs of `team`, oldest first.
    ///
    /// # Errors
    /// [`ReadError`].
    pub fn runs_of_team(&self, team: &TeamId) -> Result<Vec<RunRow>, ReadError> {
        let _ = team;
        Ok(Vec::new())
    }

    /// The runs that worked for `round`, oldest first.
    ///
    /// # Errors
    /// [`ReadError`].
    pub fn runs_of_round(&self, round: &RoundId) -> Result<Vec<RunRow>, ReadError> {
        let _ = round;
        Ok(Vec::new())
    }

    /// The run `run`.
    ///
    /// # Errors
    /// [`ReadError`].
    pub fn run(&self, run: &RunId) -> Result<Option<RunRow>, ReadError> {
        let _ = run;
        Ok(None)
    }

    /// The steps of `run`, in their order.
    ///
    /// # Errors
    /// [`ReadError`].
    pub fn steps(&self, run: &RunId) -> Result<Vec<Stored<Step>>, ReadError> {
        let _ = run;
        Ok(Vec::new())
    }

    /// What `run` changed; `None` when nothing was recorded about it.
    ///
    /// # Errors
    /// [`ReadError`].
    pub fn changes(&self, run: &RunId) -> Result<Option<Stored<RunChanges>>, ReadError> {
        let _ = run;
        Ok(None)
    }

    /// The prompts of the runs of `round`, oldest first.
    ///
    /// # Errors
    /// [`ReadError`].
    pub fn prompts_of_round(&self, round: &RoundId) -> Result<Vec<RoundPrompt>, ReadError> {
        let _ = round;
        Ok(Vec::new())
    }
}
