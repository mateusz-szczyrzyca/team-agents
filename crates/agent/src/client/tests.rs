use agent_core::protocol::{
    Client, Command, Event, EventKind, Failure, FailureKind, Frame, ProtocolError, Reply, Request, RequestId, Response,
    StatusRequest,
};
use agent_core::state::Timestamp;

use super::*;
use crate::testing::{Answer, FakeDaemon};

fn status() -> Command {
    Command::Status(StatusRequest { target: None })
}

fn line(response: &Response) -> String {
    Frame::Response(response.clone()).to_line().unwrap()
}

fn teams(id: u64) -> Vec<String> {
    vec![line(&Response::ok(RequestId(id), Reply::Teams { teams: Vec::new() }))]
}

fn socket(dir: &tempfile::TempDir) -> PathBuf {
    dir.path().join("agent.sock")
}

#[tokio::test]
async fn a_request_goes_out_as_one_line_and_its_reply_comes_back() {
    let dir = tempfile::tempdir().unwrap();
    let daemon = FakeDaemon::serve(&socket(&dir), |request| Answer::Lines(teams(request.id.0)));
    let mut connection = Connection::connect(daemon.socket()).await.unwrap();

    let reply = connection.request(Client::Cli, status()).await.unwrap();

    assert_eq!(reply, Reply::Teams { teams: Vec::new() });
    assert_eq!(daemon.requests(), [Request::new(RequestId(1), Client::Cli, status())]);
}

#[tokio::test]
async fn the_requests_of_one_connection_are_numbered_in_order() {
    let dir = tempfile::tempdir().unwrap();
    let daemon = FakeDaemon::serve(&socket(&dir), |request| Answer::Lines(teams(request.id.0)));
    let mut connection = Connection::connect(daemon.socket()).await.unwrap();

    connection.request(Client::Cli, status()).await.unwrap();
    connection.request(Client::Cli, status()).await.unwrap();

    let ids: Vec<u64> = daemon.requests().iter().map(|request| request.id.0).collect();
    assert_eq!(ids, [1, 2]);
}

#[tokio::test]
async fn a_missing_socket_means_the_daemon_is_not_running() {
    let dir = tempfile::tempdir().unwrap();

    let error = Connection::connect(&socket(&dir)).await.unwrap_err();

    assert!(
        matches!(&error, ClientError::NotRunning { socket: s, .. } if *s == socket(&dir)),
        "{error:?}"
    );
}

#[tokio::test]
async fn a_socket_file_nobody_listens_on_means_the_daemon_is_not_running() {
    let dir = tempfile::tempdir().unwrap();
    drop(tokio::net::UnixListener::bind(socket(&dir)).unwrap());

    let error = Connection::connect(&socket(&dir)).await.unwrap_err();

    assert!(matches!(error, ClientError::NotRunning { .. }), "{error:?}");
}

#[tokio::test]
async fn a_file_that_is_no_socket_cannot_be_connected_to() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(socket(&dir), "").unwrap();

    let error = Connection::connect(&socket(&dir)).await.unwrap_err();

    assert!(
        matches!(error, ClientError::Connect { .. } | ClientError::NotRunning { .. }),
        "{error:?}"
    );
}

#[tokio::test]
async fn a_refusal_of_the_daemon_is_an_error_with_its_message() {
    let dir = tempfile::tempdir().unwrap();
    let failure = Failure {
        kind: FailureKind::NotFound,
        message: "no such team".to_owned(),
    };
    let reply = failure.clone();
    let daemon = FakeDaemon::serve(&socket(&dir), move |request| {
        Answer::Lines(vec![line(&Response::error(request.id, reply.clone()))])
    });
    let mut connection = Connection::connect(daemon.socket()).await.unwrap();

    let error = connection.request(Client::Cli, status()).await.unwrap_err();

    assert!(
        matches!(&error, ClientError::Refused(found) if *found == failure),
        "{error:?}"
    );
}

#[tokio::test]
async fn a_daemon_that_hangs_up_before_it_answers_is_an_error() {
    let dir = tempfile::tempdir().unwrap();
    let daemon = FakeDaemon::serve(&socket(&dir), |_| Answer::Close);
    let mut connection = Connection::connect(daemon.socket()).await.unwrap();

    let error = connection.request(Client::Cli, status()).await.unwrap_err();

    assert!(matches!(error, ClientError::Closed), "{error:?}");
}

#[tokio::test]
async fn an_answer_that_is_not_a_message_is_a_protocol_error() {
    let dir = tempfile::tempdir().unwrap();
    let daemon = FakeDaemon::serve(&socket(&dir), |_| Answer::Lines(vec!["not json\n".to_owned()]));
    let mut connection = Connection::connect(daemon.socket()).await.unwrap();

    let error = connection.request(Client::Cli, status()).await.unwrap_err();

    assert!(
        matches!(error, ClientError::Protocol(ProtocolError::Malformed { .. })),
        "{error:?}"
    );
}

#[tokio::test]
async fn an_answer_of_another_protocol_version_is_said_so() {
    let dir = tempfile::tempdir().unwrap();
    let daemon = FakeDaemon::serve(&socket(&dir), |_| {
        Answer::Lines(vec!["{\"type\":\"response\",\"protocol\":2,\"id\":1}\n".to_owned()])
    });
    let mut connection = Connection::connect(daemon.socket()).await.unwrap();

    let error = connection.request(Client::Cli, status()).await.unwrap_err();

    assert!(
        matches!(&error, ClientError::Protocol(ProtocolError::UnsupportedVersion(version)) if version.found == 2),
        "{error:?}"
    );
}

#[tokio::test]
async fn an_answer_to_another_request_is_an_error() {
    let dir = tempfile::tempdir().unwrap();
    let daemon = FakeDaemon::serve(&socket(&dir), |_| Answer::Lines(teams(77)));
    let mut connection = Connection::connect(daemon.socket()).await.unwrap();

    let error = connection.request(Client::Cli, status()).await.unwrap_err();

    assert!(
        matches!(
            error,
            ClientError::WrongAnswer {
                expected: RequestId(1),
                found: RequestId(77)
            }
        ),
        "{error:?}"
    );
}

#[tokio::test]
async fn events_that_come_before_the_answer_are_skipped() {
    let dir = tempfile::tempdir().unwrap();
    let daemon = FakeDaemon::serve(&socket(&dir), |request| {
        let at: Timestamp = "2026-11-11T19:15:00Z".parse().unwrap();
        let team = "team-20261111-153600-t5m8".parse().unwrap();
        let event = Frame::Event(Event::new(at, EventKind::TeamStarted { team }))
            .to_line()
            .unwrap();
        Answer::Lines(vec![event, teams(request.id.0).remove(0)])
    });
    let mut connection = Connection::connect(daemon.socket()).await.unwrap();

    let reply = connection.request(Client::Cli, status()).await.unwrap();

    assert_eq!(reply, Reply::Teams { teams: Vec::new() });
}

#[tokio::test(start_paused = true)]
async fn a_daemon_that_never_answers_is_given_up_after_the_timeout() {
    let dir = tempfile::tempdir().unwrap();
    let daemon = FakeDaemon::serve(&socket(&dir), |_| Answer::Silence);
    let mut connection = Connection::connect(daemon.socket()).await.unwrap();

    let error = connection.request(Client::Cli, status()).await.unwrap_err();

    assert!(
        matches!(error, ClientError::Timeout { waited } if waited == REQUEST_TIMEOUT),
        "{error:?}"
    );
}
