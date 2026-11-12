//! Reading a text file from the home with a bound: the files of the home (settings, prompts, questions) are put there
//! by a human or a model, so how much of them is read is the reader's decision.

use std::io;
use std::os::unix::fs::MetadataExt;
use std::path::Path;
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
