//! A stand-in for the daemon's socket, for the tests of its clients.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};

use agent_core::protocol::Request;
use tokio::io::BufReader;
use tokio::net::UnixListener;
use tokio::task::JoinHandle;

use crate::lines::{read_line, write_line};

/// What the stand-in does with a request.
pub(crate) enum Answer {
    /// Writes these lines, in order.
    Lines(Vec<String>),
    /// Reads the request and says nothing.
    Silence,
    /// Closes the connection.
    Close,
}

/// A daemon that answers as a function says; it records the requests it was sent.
pub(crate) struct FakeDaemon {
    /// The socket it listens on.
    socket: PathBuf,
    /// The requests it received, in order.
    seen: Arc<Mutex<Vec<Request>>>,
    /// The task that accepts connections.
    accepting: JoinHandle<()>,
}

impl FakeDaemon {
    /// Listens on `socket` and answers each request with what `answer` says.
    pub(crate) fn serve(socket: &Path, answer: impl Fn(&Request) -> Answer + Send + Sync + 'static) -> Self {
        let listener = UnixListener::bind(socket).unwrap();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let record = Arc::clone(&seen);
        let answer = Arc::new(answer);
        let accepting = tokio::spawn(async move {
            let mut connections = tokio::task::JoinSet::new();
            loop {
                let (stream, _) = listener.accept().await.unwrap();
                let (read, mut write) = stream.into_split();
                let record = Arc::clone(&record);
                let answer = Arc::clone(&answer);
                connections.spawn(async move {
                    let mut reader = BufReader::new(read);
                    while let Some(line) = read_line(&mut reader, 1 << 20).await.unwrap() {
                        let request = Request::from_line(&line).unwrap();
                        let reply = answer(&request);
                        record.lock().unwrap_or_else(PoisonError::into_inner).push(request);
                        match reply {
                            Answer::Lines(lines) => {
                                for line in lines {
                                    write_line(&mut write, &line).await.unwrap();
                                }
                            }
                            Answer::Silence => {}
                            Answer::Close => return,
                        }
                    }
                });
            }
        });
        Self {
            socket: socket.to_path_buf(),
            seen,
            accepting,
        }
    }

    /// The socket.
    pub(crate) fn socket(&self) -> &Path {
        &self.socket
    }

    /// The requests received so far.
    pub(crate) fn requests(&self) -> Vec<Request> {
        self.seen.lock().unwrap_or_else(PoisonError::into_inner).clone()
    }
}

impl Drop for FakeDaemon {
    fn drop(&mut self) {
        self.accepting.abort();
    }
}
