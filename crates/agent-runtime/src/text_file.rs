//! Reading a text file from the home with a bound: the files of the home (settings, prompts, questions) are put there
//! by a human or a model, so how much of them is read is the reader's decision.

use std::io::{self, Write};
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::string::FromUtf8Error;

use tokio::io::AsyncReadExt;

/// Why a text file could not be read.
#[derive(Debug, thiserror::Error)]
pub enum TextFileError {
    /// The file exists and cannot be read.
    #[error("the file cannot be read")]
    Read(#[source] io::Error),
    /// The file is longer than the limit.
    #[error("the file is longer than {limit} bytes")]
    TooLarge {
        /// The limit, in bytes.
        limit: usize,
    },
    /// The file is not UTF-8 text.
    #[error("the file is not UTF-8 text")]
    NotUtf8(#[source] FromUtf8Error),
}

/// The text of a file and its permission bits, as one open file showed them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextFile {
    /// The content.
    pub text: String,
    /// The `st_mode` of the file.
    pub mode: u32,
}

/// Reads the file at `path`, at most `limit` bytes of it; `None` when there is no such file.
///
/// The mode comes from the open file, not from its name: no other file can be put in its place between the check and
/// the read (SEC-5).
///
/// # Errors
/// [`TextFileError`].
pub async fn read_text_file(path: &Path, limit: usize) -> Result<Option<TextFile>, TextFileError> {
    let file = match tokio::fs::File::open(path).await {
        Ok(file) => file,
        Err(source) if source.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(source) => return Err(TextFileError::Read(source)),
    };
    let mode = file.metadata().await.map_err(TextFileError::Read)?.mode();
    // One byte more than the limit tells a file that is too long from one that has exactly the limit.
    let bound = u64::try_from(limit).map_or(u64::MAX, |limit| limit.saturating_add(1));
    let mut bytes = Vec::new();
    file.take(bound)
        .read_to_end(&mut bytes)
        .await
        .map_err(TextFileError::Read)?;
    if bytes.len() > limit {
        return Err(TextFileError::TooLarge { limit });
    }
    let text = String::from_utf8(bytes).map_err(TextFileError::NotUtf8)?;
    Ok(Some(TextFile { text, mode }))
}

/// Why a text file could not be replaced.
#[derive(Debug, thiserror::Error)]
pub enum ReplaceError {
    /// The file has no directory the new text can be written in beside it.
    #[error("the file {path:?} has no directory")]
    NoDirectory {
        /// The file.
        path: PathBuf,
    },
    /// The new text cannot be written or put in place of the file.
    #[error("cannot replace the file {path:?}")]
    Write {
        /// The file.
        path: PathBuf,
        /// What the operating system said.
        source: io::Error,
    },
    /// The task that writes the file failed.
    #[error("the task that replaces the file {path:?} failed")]
    Worker {
        /// The file.
        path: PathBuf,
        /// How the task ended.
        source: tokio::task::JoinError,
    },
}

/// Replaces the file at `path` with `text`, with the permission bits `mode`.
///
/// The text is written to a temporary file in the same directory and renamed over `path`, so a reader — a person with
/// the file open in an editor, a watcher — sees the old file or the whole new one, never half of it (SEC-5).
///
/// # Errors
/// [`ReplaceError`].
pub async fn replace_text_file(path: &Path, text: &str, mode: u32) -> Result<(), ReplaceError> {
    let target = path.to_path_buf();
    let text = text.to_owned();
    tokio::task::spawn_blocking(move || {
        let directory = target
            .parent()
            .ok_or_else(|| ReplaceError::NoDirectory { path: target.clone() })?;
        let write_error = |source| ReplaceError::Write {
            path: target.clone(),
            source,
        };
        let mut file = tempfile::NamedTempFile::new_in(directory).map_err(write_error)?;
        file.write_all(text.as_bytes()).map_err(write_error)?;
        file.as_file()
            .set_permissions(std::fs::Permissions::from_mode(mode))
            .map_err(write_error)?;
        file.as_file().sync_all().map_err(write_error)?;
        file.persist(&target).map_err(|failed| write_error(failed.error))?;
        Ok(())
    })
    .await
    .map_err(|source| ReplaceError::Worker {
        path: path.to_path_buf(),
        source,
    })?
}
