//! Durable finding IDs already delivered to a task.
use crate::text_file::{ReplaceError, TextFileError};
use agent_core::decide::findings::FindingId;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// Maximum size in bytes of a task's delivered-ID record.
pub const MAX_SHOWN_BYTES: usize = 1_048_576;

/// IDs retained across runs; absence of the file means none were delivered.
#[derive(Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShownFindings {
    /// Unique IDs, stored in numeric order for stable JSON.
    shown: BTreeSet<FindingId>,
}

/// Why delivery bookkeeping could not be loaded or saved.
#[derive(Debug, thiserror::Error)]
pub enum ShownFindingsError {
    /// The existing record could not be read.
    #[error("cannot read shown findings {path:?}")]
    Read {
        /// Record being read.
        path: PathBuf,
        /// Filesystem failure or size limit.
        source: TextFileError,
    },
    /// The record is not valid delivery JSON.
    #[error("invalid shown findings {path:?}")]
    Json {
        /// Record being decoded or encoded.
        path: PathBuf,
        /// JSON cause.
        source: serde_json::Error,
    },
    /// The complete record could not be installed atomically.
    #[error("cannot save shown findings {path:?}")]
    Write {
        /// Target record.
        path: PathBuf,
        /// Replacement failure.
        source: ReplaceError,
    },
}
impl ShownFindings {
    /// Loads a task's shown IDs, with an absent record treated as empty.
    /// # Errors
    /// Returns [`ShownFindingsError`] for unreadable or invalid records.
    pub async fn read(path: &Path) -> Result<Self, ShownFindingsError> {
        let Some(record) = crate::text_file::read_text_file(path, MAX_SHOWN_BYTES)
            .await
            .map_err(|source| ShownFindingsError::Read {
                path: path.to_path_buf(),
                source,
            })?
        else {
            return Ok(Self::default());
        };
        serde_json::from_str(&record.text).map_err(|source| ShownFindingsError::Json {
            path: path.to_path_buf(),
            source,
        })
    }
    /// The IDs excluded from later selections.
    #[must_use]
    pub const fn ids(&self) -> &BTreeSet<FindingId> {
        &self.shown
    }
    /// Records selected IDs before launching their run.
    /// # Errors
    /// Returns [`ShownFindingsError`] if the record cannot be encoded or replaced.
    pub async fn save(
        &mut self,
        path: &Path,
        ids: impl IntoIterator<Item = FindingId>,
    ) -> Result<(), ShownFindingsError> {
        self.shown.extend(ids);
        let text = serde_json::to_string(self).map_err(|source| ShownFindingsError::Json {
            path: path.to_path_buf(),
            source,
        })?;
        crate::text_file::replace_text_file(path, &text, 0o600)
            .await
            .map_err(|source| ShownFindingsError::Write {
                path: path.to_path_buf(),
                source,
            })
    }
}
