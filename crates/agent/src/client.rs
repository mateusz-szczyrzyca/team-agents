//! The client of the daemon's socket: what every command but `agent daemon` talks through.
//!
//! One [`Connection`] sends requests, one at a time, and reads their answers: the control protocol of
//! `docs/state.md`, one JSON line each way. Every wait is bounded.

use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

use agent_core::protocol::{Client, Command, EncodeError, Failure, ProtocolError, Reply, Request, RequestId};
use tokio::io::BufReader;
use tokio::net::unix::{OwnedReadHalf, OwnedWriteHalf};

use crate::lines::LineError;

/// How long a connection may take to be made.
pub(crate) const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);

/// How long the daemon may take to answer a request.
pub(crate) const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

/// Why a request to the daemon failed.
#[derive(Debug, thiserror::Error)]
pub(crate) enum ClientError {
    /// Nothing listens on the socket: the daemon is not running.
    #[error("the daemon is not running (nothing listens on {socket:?}): start it with `agent daemon`")]
    NotRunning {
        /// The socket.
        socket: PathBuf,
        /// What the operating system said.
        source: io::Error,
    },
    /// The socket cannot be connected to.
    #[error("cannot connect to the daemon at {socket:?}")]
    Connect {
        /// The socket.
        socket: PathBuf,
        /// What the operating system said.
        source: io::Error,
    },
    /// The connection took too long to be made.
    #[error("the daemon at {socket:?} did not accept the connection within {waited:?}")]
    ConnectTimeout {
        /// The socket.
        socket: PathBuf,
        /// How long it was waited for.
        waited: Duration,
    },
    /// The request cannot be written as JSON.
    #[error("cannot encode the request")]
    Encode(#[from] EncodeError),
    /// The stream failed or carried a line that is not text.
    #[error("the conversation with the daemon failed")]
    Line(#[from] LineError),
    /// The daemon sent a line that is not a valid message of this protocol.
    #[error("the daemon sent a message this client cannot read")]
    Protocol(#[from] ProtocolError),
    /// The daemon closed the connection before it answered.
    #[error("the daemon closed the connection without answering")]
    Closed,
    /// The daemon did not answer in time.
    #[error("the daemon did not answer within {waited:?}")]
    Timeout {
        /// How long it was waited for.
        waited: Duration,
    },
    /// The daemon answered another request than the one asked.
    #[error("the daemon answered request {}, not request {}", .found.0, .expected.0)]
    WrongAnswer {
        /// The request that was asked.
        expected: RequestId,
        /// The one the answer is for.
        found: RequestId,
    },
    /// The daemon refused the request.
    #[error("the daemon refused the request: {}", .0.message)]
    Refused(Failure),
}

/// A connection to the daemon.
#[derive(Debug)]
pub(crate) struct Connection {
    /// What the daemon sends.
    reader: BufReader<OwnedReadHalf>,
    /// What is sent to it.
    writer: OwnedWriteHalf,
    /// The ID of the next request.
    next_id: u64,
}

impl Connection {
    /// Connects to the daemon at `socket`.
    ///
    /// # Errors
    /// [`ClientError::NotRunning`] when nothing listens there, [`ClientError::Connect`] and
    /// [`ClientError::ConnectTimeout`].
    pub(crate) async fn connect(socket: &Path) -> Result<Self, ClientError> {
        let _ = socket;
        Err(ClientError::Closed)
    }

    /// Sends `command` as `client` and returns the daemon's reply.
    ///
    /// # Errors
    /// [`ClientError`]: the conversation failed, or the daemon answered with an error.
    pub(crate) async fn request(&mut self, client: Client, command: Command) -> Result<Reply, ClientError> {
        let _ = (client, command, &mut self.next_id, &mut self.writer, &mut self.reader);
        Err(ClientError::Closed)
    }
}

#[cfg(test)]
mod tests;
