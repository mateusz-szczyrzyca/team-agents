//! A fake HTTP server on a loopback port for the tests of the GitHub client: it records every request and answers
//! each one as a handler says. It speaks just enough HTTP/1.1 for `reqwest`: one request per connection, a body
//! given by `Content-Length`, `Connection: close` in every answer.

use std::future::pending;
use std::io;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex, PoisonError};

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::{JoinHandle, JoinSet};

/// What the server saw of one request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Recorded {
    /// The method: `GET`, `POST`, ...
    pub method: String,
    /// The path and the query, as the request line has them.
    pub target: String,
    /// The headers, names in lowercase, in the order they came.
    pub headers: Vec<(String, String)>,
    /// The body, as text.
    pub body: String,
}

impl Recorded {
    /// The value of the header `name` (lowercase), if the request had it.
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.as_str())
    }
}

/// What the server answers.
#[derive(Debug, Clone)]
pub enum Reply {
    /// A status and a JSON body, with these extra headers.
    Json {
        /// The status code.
        status: u16,
        /// The body.
        body: String,
        /// Extra headers, such as `Retry-After`.
        headers: Vec<(&'static str, String)>,
    },
    /// The request is read and never answered: the connection stays open.
    Hang,
    /// A body that announces more than it sends, and then the connection is closed.
    Truncated,
}

impl Reply {
    /// A JSON answer with no extra headers.
    pub fn json(status: u16, body: &str) -> Self {
        Self::Json {
            status,
            body: body.to_owned(),
            headers: Vec::new(),
        }
    }

    /// The same, with one more header.
    pub fn with_header(self, name: &'static str, value: &str) -> Self {
        match self {
            Self::Json {
                status,
                body,
                mut headers,
            } => {
                headers.push((name, value.to_owned()));
                Self::Json { status, body, headers }
            }
            other => other,
        }
    }
}

/// A running fake server; it stops when it is dropped.
#[derive(Debug)]
pub struct FakeServer {
    /// Where it listens.
    addr: SocketAddr,
    /// Every request seen so far, in the order their bodies were complete.
    seen: Arc<Mutex<Vec<Recorded>>>,
    /// The accept loop; aborted on drop, which also ends the connections it owns.
    accept_loop: JoinHandle<()>,
}

impl FakeServer {
    /// Starts a server that answers the request number `n` (counted from 0) with `handler(n, &request)`.
    pub async fn start<H>(handler: H) -> io::Result<Self>
    where
        H: Fn(usize, &Recorded) -> Reply + Send + Sync + 'static,
    {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let addr = listener.local_addr()?;
        let seen = Arc::new(Mutex::new(Vec::new()));
        let handler = Arc::new(handler);
        let accept_loop = tokio::spawn(accept_loop(listener, Arc::clone(&seen), handler));
        Ok(Self {
            addr,
            seen,
            accept_loop,
        })
    }

    /// Starts a server that answers every request with `reply`.
    pub async fn always(reply: Reply) -> io::Result<Self> {
        Self::start(move |_, _| reply.clone()).await
    }

    /// Starts a server that answers the requests in turn with `replies`, and the last one for all that follow.
    pub async fn sequence(replies: Vec<Reply>) -> io::Result<Self> {
        Self::start(move |n, _| {
            replies
                .get(n)
                .or_else(|| replies.last())
                .cloned()
                .unwrap_or(Reply::Hang)
        })
        .await
    }

    /// The base URL of the server: `http://127.0.0.1:<port>`.
    pub fn url(&self) -> String {
        format!("http://{}", self.addr)
    }

    /// The requests seen so far.
    pub fn requests(&self) -> Vec<Recorded> {
        self.seen.lock().unwrap_or_else(PoisonError::into_inner).clone()
    }
}

impl Drop for FakeServer {
    fn drop(&mut self) {
        self.accept_loop.abort();
    }
}

type Handler = Arc<dyn Fn(usize, &Recorded) -> Reply + Send + Sync>;

async fn accept_loop(listener: TcpListener, seen: Arc<Mutex<Vec<Recorded>>>, handler: Handler) {
    let mut connections = JoinSet::new();
    loop {
        let Ok((stream, _peer)) = listener.accept().await else {
            return;
        };
        connections.spawn(serve_connection(stream, Arc::clone(&seen), Arc::clone(&handler)));
    }
}

async fn serve_connection(mut stream: TcpStream, seen: Arc<Mutex<Vec<Recorded>>>, handler: Handler) {
    let Some(request) = read_request(&mut stream).await else {
        return;
    };
    let index = {
        let mut seen = seen.lock().unwrap_or_else(PoisonError::into_inner);
        seen.push(request.clone());
        seen.len() - 1
    };
    let reply = handler(index, &request);
    let wire = match reply {
        Reply::Hang => pending::<String>().await,
        Reply::Truncated => "HTTP/1.1 200 OK\r\nContent-Length: 100\r\nConnection: close\r\n\r\n{\"half\"".to_owned(),
        Reply::Json { status, body, headers } => {
            let mut head = format!(
                "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n",
                body.len()
            );
            for (name, value) in headers {
                head.push_str(&format!("{name}: {value}\r\n"));
            }
            format!("{head}\r\n{body}")
        }
    };
    if stream.write_all(wire.as_bytes()).await.is_ok() {
        // A failed flush or shutdown means the client has gone; there is nobody to tell.
        drop(stream.shutdown().await);
    }
}

/// Reads one request; `None` when the client closes before it is complete.
async fn read_request(stream: &mut TcpStream) -> Option<Recorded> {
    let mut buffer = Vec::new();
    let mut chunk = [0_u8; 4096];
    let head_end = loop {
        if let Some(at) = buffer.windows(4).position(|window| window == b"\r\n\r\n") {
            break at;
        }
        let read = stream.read(&mut chunk).await.ok()?;
        if read == 0 {
            return None;
        }
        buffer.extend_from_slice(chunk.get(..read)?);
    };
    let head = String::from_utf8_lossy(buffer.get(..head_end)?).into_owned();
    let mut lines = head.split("\r\n");
    let mut request_line = lines.next()?.split(' ');
    let method = request_line.next()?.to_owned();
    let target = request_line.next()?.to_owned();
    let headers: Vec<(String, String)> = lines
        .filter_map(|line| line.split_once(':'))
        .map(|(name, value)| (name.trim().to_ascii_lowercase(), value.trim().to_owned()))
        .collect();
    let length: usize = headers
        .iter()
        .find(|(name, _)| name == "content-length")
        .and_then(|(_, value)| value.parse().ok())
        .unwrap_or(0);
    let body_start = head_end + 4;
    while buffer.len() < body_start + length {
        let read = stream.read(&mut chunk).await.ok()?;
        if read == 0 {
            return None;
        }
        buffer.extend_from_slice(chunk.get(..read)?);
    }
    let body = String::from_utf8_lossy(buffer.get(body_start..body_start + length)?).into_owned();
    Some(Recorded {
        method,
        target,
        headers,
        body,
    })
}
