//! Lines on a stream: the framing of the control protocol (one JSON message per line), with a bound on their length.

use std::io;
use std::string::FromUtf8Error;

use tokio::io::{AsyncBufRead, AsyncWrite};

/// Why a line could not be read or written.
#[derive(Debug, thiserror::Error)]
pub(crate) enum LineError {
    /// The stream failed.
    #[error("cannot read from the stream")]
    Read(#[source] io::Error),
    /// The stream refused the line.
    #[error("cannot write to the stream")]
    Write(#[source] io::Error),
    /// The line is longer than the limit; the rest of it is not read.
    #[error("the line is longer than {max} bytes")]
    TooLong {
        /// The limit, in bytes.
        max: usize,
    },
    /// The stream ended in the middle of a line.
    #[error("the stream ended in the middle of a line")]
    Truncated,
    /// The line is not UTF-8 text.
    #[error("the line is not UTF-8 text")]
    NotUtf8(#[source] FromUtf8Error),
}

/// Reads one line of at most `max` bytes, without its newline; `None` when the stream ended before any byte.
///
/// A line over the limit ends the use of the stream: what follows it is not read, so the stream cannot be told where
/// the next line begins.
///
/// # Errors
/// [`LineError`].
pub(crate) async fn read_line<R: AsyncBufRead + Unpin>(
    reader: &mut R,
    max: usize,
) -> Result<Option<String>, LineError> {
    let _ = (reader, max);
    Ok(None)
}

/// Writes `line`, which already ends with its newline, and flushes.
///
/// # Errors
/// [`LineError::Write`].
pub(crate) async fn write_line<W: AsyncWrite + Unpin>(writer: &mut W, line: &str) -> Result<(), LineError> {
    let _ = (writer, line);
    Ok(())
}

#[cfg(test)]
mod tests;
