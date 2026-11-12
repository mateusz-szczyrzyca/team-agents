//! Lines on a stream: the framing of the control protocol (one JSON message per line), with a bound on their length.

use std::io;
use std::string::FromUtf8Error;

use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncWrite, AsyncWriteExt};

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
/// the next line begins. Not cancel safe: a call that is dropped loses what it read of a line; [`read_line_into`] is.
///
/// # Errors
/// [`LineError`].
pub(crate) async fn read_line<R: AsyncBufRead + Unpin>(
    reader: &mut R,
    max: usize,
) -> Result<Option<String>, LineError> {
    read_line_into(reader, &mut Vec::new(), max).await
}

/// Reads one line of at most `max` bytes like [`read_line`], keeping the bytes of a line that is not complete yet in
/// `line`.
///
/// Cancel safe: a call that is dropped before the end of the line (in a `select!`) leaves what it read in `line`, and
/// the next call with the same `line` goes on from there. `line` is empty again when a line is returned.
///
/// # Errors
/// [`LineError`].
pub(crate) async fn read_line_into<R: AsyncBufRead + Unpin>(
    reader: &mut R,
    line: &mut Vec<u8>,
    max: usize,
) -> Result<Option<String>, LineError> {
    loop {
        // The only await: everything after it, up to the next one, changes `line` and the reader together.
        let available = reader.fill_buf().await.map_err(LineError::Read)?;
        if available.is_empty() {
            return if line.is_empty() {
                Ok(None)
            } else {
                Err(LineError::Truncated)
            };
        }
        let newline = available.iter().position(|byte| *byte == b'\n');
        let end = newline.unwrap_or(available.len());
        if line.len().saturating_add(end) > max {
            return Err(LineError::TooLong { max });
        }
        // `end` is at most the length of `available`, so the slice is there.
        line.extend_from_slice(available.get(..end).unwrap_or_default());
        let consumed = if newline.is_some() { end.saturating_add(1) } else { end };
        reader.consume(consumed);
        if newline.is_some() {
            break;
        }
    }
    String::from_utf8(std::mem::take(line))
        .map(Some)
        .map_err(LineError::NotUtf8)
}

/// Writes `line`, which already ends with its newline, and flushes.
///
/// # Errors
/// [`LineError::Write`].
pub(crate) async fn write_line<W: AsyncWrite + Unpin>(writer: &mut W, line: &str) -> Result<(), LineError> {
    writer.write_all(line.as_bytes()).await.map_err(LineError::Write)?;
    writer.flush().await.map_err(LineError::Write)
}

#[cfg(test)]
mod tests;
