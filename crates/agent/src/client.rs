//! The client of the daemon's socket: what every command but `agent daemon` talks through.
//!
//! One [`Connection`] sends requests, one at a time, and reads their answers: the control protocol of
//! `docs/state.md`, one JSON line each way. Every wait is bounded, except the wait for a command whose answer comes when
//! work is over ([`Connection::request_until_answered`]): the daemon bounds that one.

use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

use agent_core::protocol::{
    Client, Command, EncodeError, Failure, Frame, MAX_FRAME_BYTES, Outcome, ProtocolError, Reply, ReplyFormat, Request,
    RequestId, Response,
};
use tokio::io::BufReader;
use tokio::net::UnixStream;
use tokio::net::unix::{OwnedReadHalf, OwnedWriteHalf};
use tokio::time::timeout;

use crate::lines::{LineError, read_line, write_line};

/// The number of a connection's first request.
const FIRST_REQUEST: u64 = 1;

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
    /// Split interpretation attached to the latest response.
    split: Option<String>,
    /// Socket used for a single reconnect after a rejected read-only negotiation.
    socket: PathBuf,
    /// Negotiation result, cached for subsequent split-aware requests.
    format: Option<ReplyFormat>,
}

impl Connection {
    /// The split interpretation of the latest response, when it answered a split proposal.
    pub(crate) fn split_acknowledgement(&self) -> Option<&str> {
        self.split.as_deref()
    }

    /// Connects to the daemon at `socket`.
    ///
    /// # Errors
    /// [`ClientError::NotRunning`] when nothing listens there, [`ClientError::Connect`] and
    /// [`ClientError::ConnectTimeout`].
    pub(crate) async fn connect(socket: &Path) -> Result<Self, ClientError> {
        let stream = match timeout(CONNECT_TIMEOUT, UnixStream::connect(socket)).await {
            Ok(Ok(stream)) => stream,
            Ok(Err(source))
                if matches!(
                    source.kind(),
                    io::ErrorKind::NotFound | io::ErrorKind::ConnectionRefused
                ) =>
            {
                return Err(ClientError::NotRunning {
                    socket: socket.to_path_buf(),
                    source,
                });
            }
            Ok(Err(source)) => {
                return Err(ClientError::Connect {
                    socket: socket.to_path_buf(),
                    source,
                });
            }
            // The elapsed time carries nothing but the fact; the error names how long it was waited.
            Err(_elapsed) => {
                return Err(ClientError::ConnectTimeout {
                    socket: socket.to_path_buf(),
                    waited: CONNECT_TIMEOUT,
                });
            }
        };
        let (read, writer) = stream.into_split();
        Ok(Self {
            reader: BufReader::new(read),
            writer,
            next_id: FIRST_REQUEST,
            split: None,
            socket: socket.to_owned(),
            format: None,
        })
    }

    /// Sends `command` as `client` and returns the daemon's reply.
    ///
    /// # Errors
    /// [`ClientError`]: the conversation failed, or the daemon answered with an error.
    pub(crate) async fn request(&mut self, client: Client, command: Command) -> Result<Reply, ClientError> {
        if matches!(command, Command::Answer(_)) {
            self.request_split(client, command).await
        } else {
            self.converse(client, command, Some(REQUEST_TIMEOUT)).await
        }
    }

    /// Sends `command` like [`Connection::request`] and waits for the answer for as long as the daemon takes.
    ///
    /// For a command whose answer comes when work is over: a gate waits for the gates asked before it, and then runs;
    /// a start fetches the base from `origin` first. The wait is bounded by the daemon (its queue, the timeout of the
    /// gate, the timeout of the fetch), and a daemon that ends closes the
    /// connection, which [`ClientError::Closed`] reports.
    ///
    /// # Errors
    /// [`ClientError`]: the conversation failed, or the daemon answered with an error.
    pub(crate) async fn request_until_answered(
        &mut self,
        client: Client,
        command: Command,
    ) -> Result<Reply, ClientError> {
        self.converse(client, command, None).await
    }

    /// Requests split-aware version-four replies.
    pub(crate) async fn request_split(&mut self, client: Client, command: Command) -> Result<Reply, ClientError> {
        let format = self.negotiate(&client).await?;
        self.converse_format(client, command, Some(REQUEST_TIMEOUT), format)
            .await
    }

    /// Probes with a read-only status request, so a closed connection never retries a mutating command.
    async fn negotiate(&mut self, client: &Client) -> Result<ReplyFormat, ClientError> {
        if let Some(format) = self.format {
            return Ok(format);
        }
        let probe = Command::Status(agent_core::protocol::StatusRequest { target: None });
        let format = match self
            .converse_format(client.clone(), probe, Some(REQUEST_TIMEOUT), ReplyFormat::Split)
            .await
        {
            Ok(_) => ReplyFormat::Split,
            Err(error)
                if matches!(
                    &error,
                    ClientError::Closed
                        | ClientError::WrongAnswer {
                            found: RequestId(0),
                            ..
                        }
                        | ClientError::Refused(Failure {
                            kind: agent_core::protocol::FailureKind::Invalid,
                            ..
                        })
                ) =>
            {
                // Old peers may reject the tag with request zero, or close it before parsing the request.
                tracing::debug!(error = %agent_runtime::process::error_chain(&error), "legacy daemon refused split protocol; reconnecting once");
                *self = Self::connect(&self.socket).await?;
                ReplyFormat::Legacy
            }
            Err(error) => return Err(error),
        };
        self.format = Some(format);
        Ok(format)
    }

    /// Sends `command` and reads its answer, giving up after `limit` when there is one.
    async fn converse(
        &mut self,
        client: Client,
        command: Command,
        limit: Option<Duration>,
    ) -> Result<Reply, ClientError> {
        self.converse_format(client, command, limit, ReplyFormat::Legacy).await
    }

    /// Uses the selected wire format for both the request and its response.
    async fn converse_format(
        &mut self,
        client: Client,
        command: Command,
        limit: Option<Duration>,
        format: ReplyFormat,
    ) -> Result<Reply, ClientError> {
        let id = RequestId(self.next_id);
        self.next_id = self.next_id.saturating_add(1);
        let line = Request::new(id, client, command).to_line_for(format)?;
        let conversation = async {
            write_line(&mut self.writer, &line).await?;
            self.read_response(id, format).await
        };
        let response = match limit {
            Some(limit) => match timeout(limit, conversation).await {
                Ok(response) => response?,
                // The elapsed time carries nothing but the fact; the error names how long it was waited.
                Err(_elapsed) => return Err(ClientError::Timeout { waited: limit }),
            },
            None => conversation.await?,
        };
        self.split = response.split;
        match response.result {
            Outcome::Ok { reply } => Ok(reply),
            Outcome::Error { error } => Err(ClientError::Refused(error)),
        }
    }

    /// Reads frames until the answer to request `id`; events a subscription would send are skipped.
    async fn read_response(&mut self, id: RequestId, format: ReplyFormat) -> Result<Response, ClientError> {
        loop {
            let line = read_line(&mut self.reader, MAX_FRAME_BYTES)
                .await?
                .ok_or(ClientError::Closed)?;
            let frame = match format {
                ReplyFormat::Legacy => Frame::from_line(&line)?,
                ReplyFormat::Split => Frame::from_line_negotiated(&line)?,
            };
            match frame {
                Frame::Response(response) if response.id == id => return Ok(response),
                Frame::Response(response) => {
                    return Err(ClientError::WrongAnswer {
                        expected: id,
                        found: response.id,
                    });
                }
                Frame::Event(_) => {}
            }
        }
    }
}

#[cfg(test)]
mod tests;
