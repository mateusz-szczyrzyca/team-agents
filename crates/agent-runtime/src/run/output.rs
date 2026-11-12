//! Following the output files of a run: the new complete lines of its standard output, and the end of its standard
//! error.
//!
//! The supervisor appends to the files; the engine reads them from where it stopped last time. Everything is bounded: a
//! poll reads at most [`MAX_READ_PER_POLL`] bytes, a line at most [`MAX_LINE_BYTES`] (a longer one is dropped whole and
//! counted), and the error output is read only at its end.

use std::io::SeekFrom;
use std::path::{Path, PathBuf};

use tokio::fs::File;
use tokio::io::{AsyncReadExt, AsyncSeekExt};

use super::RunError;

/// The most bytes one poll reads, so that a CLI printing without pause cannot keep the engine from its watchdog.
const MAX_READ_PER_POLL: usize = 16 << 20;

/// The size of one read.
const CHUNK_BYTES: usize = 64 << 10;

/// The longest line the engine keeps, in bytes: a tool's whole output can be one JSON line, but not without bound.
const MAX_LINE_BYTES: usize = 8 << 20;

/// The most bytes of the error output that are read: the end of it holds what the classifier looks for.
pub(super) const MAX_STDERR_BYTES: u64 = 256 << 10;

/// The output file of a run, read in its order, line by line.
#[derive(Debug)]
pub(super) struct OutputTail {
    /// The file.
    path: PathBuf,
    /// Where the next read starts.
    offset: u64,
    /// The start of a line that is not complete yet.
    partial: Vec<u8>,
    /// Whether the line being read is over [`MAX_LINE_BYTES`] and is thrown away up to its end.
    skipping: bool,
    /// The lines thrown away: too long, or not UTF-8 text.
    dropped: u64,
}

impl OutputTail {
    /// A tail of `path`, from its start.
    pub(super) const fn new(path: PathBuf) -> Self {
        Self {
            path,
            offset: 0,
            partial: Vec::new(),
            skipping: false,
            dropped: 0,
        }
    }

    /// The lines thrown away so far.
    pub(super) const fn dropped(&self) -> u64 {
        self.dropped
    }

    /// The complete lines written since the last call.
    pub(super) async fn read_new(&mut self) -> Result<Vec<String>, RunError> {
        let path = self.path.clone();
        let output_error = |source| RunError::Output {
            path: path.clone(),
            source,
        };
        let mut file = File::open(&self.path).await.map_err(output_error)?;
        file.seek(SeekFrom::Start(self.offset)).await.map_err(output_error)?;
        let mut lines = Vec::new();
        let mut buffer = vec![0_u8; CHUNK_BYTES];
        let mut read = 0_usize;
        while read < MAX_READ_PER_POLL {
            let count = file.read(&mut buffer).await.map_err(output_error)?;
            if count == 0 {
                break;
            }
            read = read.saturating_add(count);
            if let Some(data) = buffer.get(..count) {
                self.feed(data, &mut lines);
            }
        }
        self.offset = file.stream_position().await.map_err(output_error)?;
        Ok(lines)
    }

    /// The last line, when the output ended without a newline.
    pub(super) fn finish(&mut self) -> Option<String> {
        let mut lines = Vec::new();
        if !self.skipping {
            self.complete(&mut lines);
        }
        lines.pop()
    }

    /// Splits `data` into lines: the ones it completes go to `lines`, the rest waits in `partial`.
    fn feed(&mut self, data: &[u8], lines: &mut Vec<String>) {
        let mut pieces = data.split(|byte| *byte == b'\n');
        let Some(last) = pieces.next_back() else {
            return;
        };
        for piece in pieces {
            self.append(piece);
            self.complete(lines);
        }
        self.append(last);
    }

    /// Adds `piece` to the line that is being read.
    fn append(&mut self, piece: &[u8]) {
        if self.skipping {
            return;
        }
        if self.partial.len().saturating_add(piece.len()) > MAX_LINE_BYTES {
            self.partial.clear();
            self.skipping = true;
            self.dropped = self.dropped.saturating_add(1);
            return;
        }
        self.partial.extend_from_slice(piece);
    }

    /// Ends the line that is being read: it goes to `lines` unless it is blank or not UTF-8 text.
    fn complete(&mut self, lines: &mut Vec<String>) {
        if std::mem::take(&mut self.skipping) {
            return;
        }
        match String::from_utf8(std::mem::take(&mut self.partial)) {
            Ok(line) if line.trim().is_empty() => {}
            Ok(line) => lines.push(line),
            Err(_not_text) => self.dropped = self.dropped.saturating_add(1),
        }
    }
}

/// The last [`MAX_STDERR_BYTES`] of the file at `path` as text.
///
/// The error output is read only to recognise a provider's limit in it, so what is not text is replaced and a character
/// cut at the start of the span is lost: losing that is the specified behaviour.
pub(super) async fn read_end(path: &Path) -> Result<String, RunError> {
    let output_error = |source| RunError::Output {
        path: path.to_path_buf(),
        source,
    };
    let mut file = File::open(path).await.map_err(output_error)?;
    let length = file.metadata().await.map_err(output_error)?.len();
    file.seek(SeekFrom::Start(length.saturating_sub(MAX_STDERR_BYTES)))
        .await
        .map_err(output_error)?;
    let mut bytes = Vec::new();
    file.take(MAX_STDERR_BYTES)
        .read_to_end(&mut bytes)
        .await
        .map_err(output_error)?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}
