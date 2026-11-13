//! The socket of the daemon: binding it privately, accepting connections and speaking the control protocol on them.
//!
//! A connection reads one request line at a time (bounded), hands it to the owner of the state and writes the answer.
//! A connection that subscribed also gets the events of the owner, until it closes or falls too far behind.

use std::future::pending;
use std::io;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use agent_core::ids::TeamId;
use agent_core::protocol::{
    Client, Command, Event, EventKind, Failure, FailureKind, Frame, MAX_REQUEST_BYTES, ProtocolError, Reply, Request,
    RequestId, Response, TeamSummary,
};
use agent_runtime::store::Home;
use tokio::io::BufReader;
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::{OwnedSemaphorePermit, Semaphore, broadcast, mpsc, oneshot};
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;

use super::owner::Message;
use crate::lines::{LineError, read_line_into, write_line};

/// How many connections are served at once; a client over the limit waits in the kernel's backlog.
pub(super) const MAX_CONNECTIONS: usize = 64;

/// The mode of the socket: its owner alone may connect.
const SOCKET_MODE: u32 = 0o600;

/// The mode of the directory the socket is made in, so that it is never reachable before its mode is set.
const STAGING_MODE: u32 = 0o700;

/// The name of the socket inside its staging directory.
const STAGED_SOCKET: &str = "s";

/// How long the accept loop waits after a failed `accept` before it tries again.
const ACCEPT_RETRY: Duration = Duration::from_millis(100);

/// Why the socket cannot be made.
#[derive(Debug, thiserror::Error)]
pub(crate) enum ServerError {
    /// The private directory the socket is made in cannot be prepared.
    #[error("cannot prepare the directory {path:?} for the socket")]
    Staging {
        /// The directory.
        path: PathBuf,
        /// What the operating system said.
        source: io::Error,
    },
    /// The socket cannot be bound.
    #[error("cannot bind the socket {path:?}")]
    Bind {
        /// The socket.
        path: PathBuf,
        /// What the operating system said.
        source: io::Error,
    },
    /// The socket cannot be made private or put in place.
    #[error("cannot put the socket in place at {path:?}")]
    Place {
        /// The socket.
        path: PathBuf,
        /// What the operating system said.
        source: io::Error,
    },
}

/// Binds the socket of `home`, replacing a file a dead daemon left there.
///
/// The caller holds the lock of the home, so no other daemon is using the file. The socket is made in a directory only
/// its owner can enter, given its mode and then renamed into place, so that it is never reachable with another mode.
///
/// # Errors
/// [`ServerError`].
pub(super) async fn bind(home: &Home) -> Result<UnixListener, ServerError> {
    let target = home.socket_file();
    let staging = home.root().join(format!(".socket-{}", std::process::id()));
    let staging_error = |source| ServerError::Staging {
        path: staging.clone(),
        source,
    };
    match tokio::fs::remove_dir_all(&staging).await {
        Ok(()) => {}
        // The usual case: a directory of a run that ended well is not there.
        Err(source) if source.kind() == io::ErrorKind::NotFound => {}
        Err(source) => return Err(staging_error(source)),
    }
    tokio::fs::DirBuilder::new()
        .mode(STAGING_MODE)
        .create(&staging)
        .await
        .map_err(staging_error)?;
    let staged = staging.join(STAGED_SOCKET);
    let placed = bind_staged(&staged, &target).await;
    // The directory has done its work, or the attempt failed; either way it goes.
    let removed = tokio::fs::remove_dir_all(&staging).await;
    let listener = placed?;
    removed.map_err(staging_error)?;
    Ok(listener)
}

/// Binds `staged`, makes it private and renames it to `target`.
async fn bind_staged(staged: &std::path::Path, target: &std::path::Path) -> Result<UnixListener, ServerError> {
    let listener = UnixListener::bind(staged).map_err(|source| ServerError::Bind {
        path: staged.to_path_buf(),
        source,
    })?;
    let place_error = |source| ServerError::Place {
        path: target.to_path_buf(),
        source,
    };
    tokio::fs::set_permissions(staged, std::fs::Permissions::from_mode(SOCKET_MODE))
        .await
        .map_err(place_error)?;
    tokio::fs::rename(staged, target).await.map_err(place_error)?;
    Ok(listener)
}

/// Removes the socket of `home` when the daemon ends.
///
/// # Errors
/// The error of the removal, unless the socket is already gone.
pub(super) async fn remove(home: &Home) -> io::Result<()> {
    match tokio::fs::remove_file(home.socket_file()).await {
        Ok(()) => Ok(()),
        // Gone already is what was wanted.
        Err(source) if source.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(source) => Err(source),
    }
}

/// Accepts connections until `shutdown` is cancelled, then waits for the ones that are open to end.
pub(super) async fn serve(listener: UnixListener, inbox: mpsc::Sender<Message>, shutdown: CancellationToken) {
    let permits = Arc::new(Semaphore::new(MAX_CONNECTIONS));
    let mut connections = JoinSet::new();
    loop {
        let permit = tokio::select! {
            () = shutdown.cancelled() => break,
            permit = Arc::clone(&permits).acquire_owned() => match permit {
                Ok(permit) => permit,
                // The semaphore is never closed: this does not happen, and the loop would only spin.
                Err(error) => {
                    tracing::error!(%error, "the connection limit of the daemon is closed");
                    break;
                }
            },
        };
        let accepted = tokio::select! {
            () = shutdown.cancelled() => break,
            accepted = listener.accept() => accepted,
        };
        match accepted {
            Ok((stream, _address)) => {
                connections.spawn(connection(stream, inbox.clone(), shutdown.clone(), permit));
            }
            Err(error) => {
                // A failed accept (no descriptors left, a client that went away) is not the end of the daemon.
                tracing::warn!(%error, "cannot accept a connection");
                tokio::time::sleep(ACCEPT_RETRY).await;
            }
        }
        reap(&mut connections);
    }
    while let Some(joined) = connections.join_next().await {
        report(joined);
    }
}

/// Collects the connections that have ended, so that a long-lived daemon does not keep their results.
fn reap(connections: &mut JoinSet<()>) {
    while let Some(joined) = connections.try_join_next() {
        report(joined);
    }
}

/// Logs a connection task that panicked.
fn report(joined: Result<(), tokio::task::JoinError>) {
    if let Err(error) = joined {
        tracing::error!(%error, "a connection task failed");
    }
}

/// What a subscribed connection receives.
struct Subscription {
    /// The team the events are of; every team when `None`.
    team: Option<TeamId>,
    /// The events.
    events: broadcast::Receiver<Event>,
}

/// What a connection waited for and got.
enum Step {
    /// A line of the client, or the end or failure of its stream.
    Line(Result<Option<String>, LineError>),
    /// An event for the subscriber, or the end of the subscription.
    Event(Result<Event, broadcast::error::RecvError>),
}

/// The team an event is about.
const fn team_of(kind: &EventKind) -> &TeamId {
    match kind {
        EventKind::TeamUpdated { summary } => &summary.id,
        EventKind::TeamStarted { team }
        | EventKind::TaskStatus { team, .. }
        | EventKind::ItemStatus { team, .. }
        | EventKind::RunStarted { team, .. }
        | EventKind::RunEnded { team, .. }
        | EventKind::QuestionAsked { team, .. }
        | EventKind::QuestionAnswered { team, .. }
        | EventKind::PullRequestOpened { team, .. }
        | EventKind::ReviewSubmitted { team, .. }
        | EventKind::GateFinished { team, .. }
        | EventKind::Controlled { team, .. } => team,
    }
}

/// Serves one connection until it ends, `shutdown` is cancelled or the owner is gone. The shutdown ends it wherever it
/// waits, also in a write to a client that does not read.
async fn connection(
    stream: UnixStream,
    inbox: mpsc::Sender<Message>,
    shutdown: CancellationToken,
    _permit: OwnedSemaphorePermit,
) {
    tokio::select! {
        () = shutdown.cancelled() => {}
        () = exchange(stream, &inbox) => {}
    }
}

/// Reads requests and answers them, and forwards the events of a subscription, until the connection ends or the owner
/// is gone.
async fn exchange(stream: UnixStream, inbox: &mpsc::Sender<Message>) {
    let (read, mut write) = stream.into_split();
    let mut reader = BufReader::new(read);
    let mut subscription: Option<Subscription> = None;
    // The part of a request that has come so far: an event that wins the `select!` must not drop it.
    let mut partial = Vec::new();
    loop {
        let step = tokio::select! {
            line = read_line_into(&mut reader, &mut partial, MAX_REQUEST_BYTES) => Step::Line(line),
            event = next_event(&mut subscription) => Step::Event(event),
        };
        let sent = match step {
            Step::Line(Ok(Some(line))) => answer(&line, inbox, &mut subscription, &mut write).await,
            Step::Line(Ok(None)) => return,
            Step::Line(Err(LineError::TooLong { max })) => {
                let failure = Failure {
                    kind: FailureKind::Invalid,
                    message: format!("the request is longer than {max} bytes"),
                };
                // The stream cannot be told where the next line begins: the answer is the last thing it carries.
                if let Err(error) = respond(&mut write, Response::error(RequestId(0), failure)).await {
                    tracing::debug!(%error, "cannot tell a client that its request was too long");
                }
                return;
            }
            Step::Line(Err(error)) => {
                tracing::debug!(%error, "a connection ended with an error");
                return;
            }
            Step::Event(Ok(event)) => forward(subscription.as_ref(), event, &mut write).await,
            Step::Event(Err(error)) => {
                // A subscriber that cannot keep up is let go: it subscribes again and starts from the state.
                tracing::warn!(%error, "a subscription ended");
                return;
            }
        };
        if let Err(error) = sent {
            tracing::debug!(%error, "a client went away");
            return;
        }
    }
}

/// The next event of the subscription; never, for a connection that has none.
async fn next_event(subscription: &mut Option<Subscription>) -> Result<Event, broadcast::error::RecvError> {
    match subscription {
        Some(subscription) => subscription.events.recv().await,
        None => pending().await,
    }
}

/// Sends `event` to the subscriber when it is about a team it follows.
async fn forward(
    subscription: Option<&Subscription>,
    event: Event,
    write: &mut tokio::net::unix::OwnedWriteHalf,
) -> Result<(), LineError> {
    let wanted = subscription.as_ref().is_some_and(|subscription| {
        subscription
            .team
            .as_ref()
            .is_none_or(|team| team == team_of(&event.event))
    });
    if !wanted {
        return Ok(());
    }
    send(write, &Frame::Event(event)).await
}

/// Reads `line` as a request, has the owner answer it and writes the answer.
async fn answer(
    line: &str,
    inbox: &mpsc::Sender<Message>,
    subscription: &mut Option<Subscription>,
    write: &mut tokio::net::unix::OwnedWriteHalf,
) -> Result<(), LineError> {
    let request = match Request::from_line(line) {
        Ok(request) => request,
        Err(error) => {
            let failure = Failure {
                kind: FailureKind::Invalid,
                message: refusal_text(&error),
            };
            return respond(write, Response::error(RequestId(0), failure)).await;
        }
    };
    let id = request.id;
    let client = request.client;
    let result = match request.command {
        Command::Subscribe(subscribe) => subscribe_to(inbox, subscribe.team, subscription).await,
        command => ask(inbox, client, command).await,
    };
    let response = match result {
        Ok(reply) => Response::ok(id, reply),
        Err(failure) => Response::error(id, failure),
    };
    respond(write, response).await
}

/// What a client is told about a line that is not a request.
fn refusal_text(error: &ProtocolError) -> String {
    match error {
        ProtocolError::TooLong { max, .. } => format!("the request is longer than {max} bytes"),
        ProtocolError::UnsupportedVersion(version) => version.to_string(),
        ProtocolError::Malformed { .. } => "the line is not a valid request: send one JSON request per line".to_owned(),
    }
}

/// Puts `command` to the owner and waits for its answer. A channel that is closed or an answer that is dropped means the
/// owner is gone, which [`gone`] tells the client.
async fn ask(inbox: &mpsc::Sender<Message>, client: Client, command: Command) -> Result<Reply, Failure> {
    let (reply, answer) = oneshot::channel();
    inbox
        .send(Message::Request { client, command, reply })
        .await
        .map_err(|_closed| gone())?;
    answer.await.map_err(|_dropped| gone())?
}

/// Asks the owner for the teams and for the events that follow them.
async fn subscribe_to(
    inbox: &mpsc::Sender<Message>,
    team: Option<TeamId>,
    subscription: &mut Option<Subscription>,
) -> Result<Reply, Failure> {
    let (reply, answer) = oneshot::channel();
    inbox
        .send(Message::Subscribe {
            team: team.clone(),
            reply,
        })
        .await
        .map_err(|_closed| gone())?;
    let (teams, events): (Vec<TeamSummary>, broadcast::Receiver<Event>) = answer.await.map_err(|_dropped| gone())?;
    *subscription = Some(Subscription { team, events });
    Ok(Reply::Subscribed { teams })
}

/// The answer when the owner is not there any more: the daemon is shutting down.
fn gone() -> Failure {
    Failure {
        kind: FailureKind::Internal,
        message: "the daemon is shutting down".to_owned(),
    }
}

/// Writes a response frame.
async fn respond(write: &mut tokio::net::unix::OwnedWriteHalf, response: Response) -> Result<(), LineError> {
    send(write, &Frame::Response(response)).await
}

/// Writes a frame as one line.
async fn send(write: &mut tokio::net::unix::OwnedWriteHalf, frame: &Frame) -> Result<(), LineError> {
    let line = frame.to_line().map_err(|error| {
        // A frame of the daemon's own types cannot fail to encode; if one does, the connection ends.
        LineError::Write(io::Error::new(io::ErrorKind::InvalidData, error))
    })?;
    write_line(write, &line).await
}
