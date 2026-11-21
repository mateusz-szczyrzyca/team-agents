use agent_core::protocol::{
    Client, Command, Event, EventKind, Failure, FailureKind, Frame, ProtocolError, Reply, Request, RequestId, Response,
    StatusRequest,
};
use agent_core::state::Timestamp;

use super::*;
use crate::testing::{Answer, FakeDaemon};

mod stale_socket;

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
    let listener = std::os::unix::net::UnixListener::bind(socket(&dir)).unwrap();
    stale_socket::close(listener).unwrap();

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
        Answer::Lines(vec!["{\"type\":\"response\",\"protocol\":4,\"id\":1}\n".to_owned()])
    });
    let mut connection = Connection::connect(daemon.socket()).await.unwrap();

    let error = connection.request(Client::Cli, status()).await.unwrap_err();

    assert!(
        matches!(&error, ClientError::Protocol(ProtocolError::UnsupportedVersion(version)) if version.found == 4),
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

#[tokio::test]
async fn split_requests_fall_back_to_a_legacy_daemon_before_sending_work() {
    let dir = tempfile::tempdir().unwrap();
    let daemon = FakeDaemon::serve(&socket(&dir), |request| Answer::Lines(teams(request.id.0)));
    let mut connection = Connection::connect(daemon.socket()).await.unwrap();
    let result = connection.request_split(Client::Cli, status()).await;
    assert!(matches!(result, Ok(Reply::Teams { teams }) if teams.is_empty()));
    assert_eq!(daemon.requests(), [Request::new(RequestId(1), Client::Cli, status())]);
    let result = connection.request_split(Client::Cli, status()).await;
    assert!(matches!(result, Ok(Reply::Teams { teams }) if teams.is_empty()));
    assert_eq!(daemon.requests().len(), 2);
    assert_eq!(connection.split_acknowledgement(), None);
}

#[tokio::test]
async fn plan_requests_fall_back_to_a_legacy_protocol_three_daemon() {
    let dir = tempfile::tempdir().unwrap();
    let daemon = FakeDaemon::serve(&socket(&dir), |request| Answer::Lines(teams(request.id.0)));
    let mut connection = Connection::connect(daemon.socket()).await.unwrap();
    let result = connection.request_plan(Client::Cli, status()).await;
    assert!(matches!(result, Ok(Reply::Teams { teams }) if teams.is_empty()));
    assert_eq!(daemon.requests(), [Request::new(RequestId(1), Client::Cli, status())]);
    assert_eq!(connection.format, Some(ReplyFormat::Legacy));

    // Subsequent status request reuses the negotiated legacy format:
    let result = connection.request(Client::Cli, status()).await;
    assert!(matches!(result, Ok(Reply::Teams { teams }) if teams.is_empty()));
    assert_eq!(daemon.requests().len(), 2);
    assert_eq!(connection.format, Some(ReplyFormat::Legacy));
}

#[tokio::test]
async fn plan_requests_fall_back_to_split_protocol_four_daemon() {
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    use tokio::net::UnixListener;

    let dir = tempfile::tempdir().unwrap();
    let sock = socket(&dir);
    let listener = UnixListener::bind(&sock).unwrap();

    let server = tokio::spawn(async move {
        loop {
            let Ok((stream, _)) = listener.accept().await else {
                break;
            };
            tokio::spawn(async move {
                let (read, mut write) = stream.into_split();
                let mut reader = BufReader::new(read);
                let mut line = String::new();
                while reader.read_line(&mut line).await.unwrap_or(0) > 0 {
                    let Ok(val) = serde_json::from_str::<serde_json::Value>(&line) else {
                        break;
                    };
                    let protocol = val.get("protocol").and_then(serde_json::Value::as_u64).unwrap_or(3);
                    let id = val.get("id").and_then(serde_json::Value::as_u64).unwrap_or(0);
                    if protocol == 5 {
                        // Protocol 4 daemon rejects protocol 5 probe:
                        let refusal = concat!(
                            "{\"protocol\":4,\"id\":0,\"type\":\"response\",",
                            "\"result\":{\"status\":\"error\",\"error\":{\"kind\":\"invalid\",",
                            "\"message\":\"unsupported protocol version: 5\"}}}\n"
                        );
                        drop(write.write_all(refusal.as_bytes()).await);
                        break;
                    }
                    let answer = format!(
                        "{{\"protocol\":4,\"id\":{id},\"type\":\"response\",\"result\":{{\"status\":\"ok\",\"reply\":{{\"kind\":\"teams\",\"teams\":[]}}}}}}\n"
                    );
                    drop(write.write_all(answer.as_bytes()).await);
                    line.clear();
                }
            });
        }
    });

    let mut connection = Connection::connect(&sock).await.unwrap();
    let result = connection.request_plan(Client::Cli, status()).await;
    assert!(matches!(result, Ok(Reply::Teams { teams }) if teams.is_empty()));
    assert_eq!(connection.format, Some(ReplyFormat::Split));

    // Subsequent request reuses the negotiated split protocol 4 format:
    let result = connection.request(Client::Cli, status()).await;
    assert!(matches!(result, Ok(Reply::Teams { teams }) if teams.is_empty()));
    assert_eq!(connection.format, Some(ReplyFormat::Split));

    server.abort();
}

#[tokio::test]
async fn a_plan_status_probe_is_returned_without_a_second_status_request() {
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    use tokio::net::UnixListener;

    let dir = tempfile::tempdir().unwrap();
    let sock = socket(&dir);
    let listener = UnixListener::bind(&sock).unwrap();
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let (read, mut write) = stream.into_split();
        let mut reader = BufReader::new(read);
        let mut line = String::new();
        let mut requests = Vec::new();
        while reader.read_line(&mut line).await.unwrap() != 0 {
            let request = Request::from_line_negotiated(&line).unwrap().0;
            let response = Frame::Response(Response::ok(request.id, Reply::Teams { teams: Vec::new() }))
                .to_line_for(ReplyFormat::Plan)
                .unwrap();
            requests.push(request);
            write.write_all(response.as_bytes()).await.unwrap();
            line.clear();
        }
        requests
    });
    let mut connection = Connection::connect(&sock).await.unwrap();
    let reply = connection.request(Client::Cli, status()).await.unwrap();
    drop(connection);
    let requests = server.await.unwrap();

    assert_eq!(reply, Reply::Teams { teams: Vec::new() });
    assert_eq!(requests, [Request::new(RequestId(1), Client::Cli, status())]);
}
