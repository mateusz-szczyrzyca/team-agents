//! Socket contracts for the swap command's output.

use agent_core::kinds::Role;
use agent_core::protocol::{
    Command, Failure, FailureKind, Frame, Reply, ReplyFormat, Request, RequestId, Response, Target,
};
use agent_core::status::TaskStatus;
use std::path::{Path, PathBuf};
use tokio::io::BufReader;
use tokio::net::UnixListener;
use tokio::task::JoinHandle;

use super::*;
use crate::fixtures::{at, member, normal, run, task, team_id};
use crate::lines::{read_line, write_line};

/// Negotiated socket server, including legacy unsupported-version replies.
struct SwapDaemon {
    /// Socket used by the client.
    socket: PathBuf,
    /// Server task, aborted when the fixture is dropped.
    accepting: JoinHandle<()>,
}

impl SwapDaemon {
    fn socket(&self) -> &Path {
        &self.socket
    }
}

impl Drop for SwapDaemon {
    fn drop(&mut self) {
        self.accepting.abort();
    }
}

/// Answers discovery and task inspection before the requested swap outcome.
fn daemon(dir: &tempfile::TempDir, running: bool, refusal: bool, older: bool) -> SwapDaemon {
    let socket = dir.path().join("agent.sock");
    let listener = UnixListener::bind(&socket).unwrap();
    let accepting = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let (read, mut write) = stream.into_split();
        let mut reader = BufReader::new(read);
        while let Some(line) = read_line(&mut reader, 1 << 20).await.unwrap() {
            if older {
                let failure = Request::from_line(&line).unwrap_err();
                let response = Response::error(
                    RequestId(0),
                    Failure {
                        kind: FailureKind::Invalid,
                        message: failure.to_string(),
                    },
                );
                write_line(
                    &mut write,
                    &Frame::Response(response).to_line_for(ReplyFormat::Legacy).unwrap(),
                )
                .await
                .unwrap();
                break;
            }
            let (request, format) = Request::from_line_negotiated(&line).unwrap();
            let response = match &request.command {
                Command::Status(_) => Response::ok(request.id, Reply::Teams { teams: Vec::new() }),
                Command::Show(show) => {
                    assert_eq!(show.target, Target::Task(task(1)));
                    let mut holder = member(1, normal("claude-1"));
                    if running {
                        holder.begin_run(run(1), at(0), None).unwrap();
                        holder.transition(TaskStatus::Running).unwrap();
                    }
                    Response::ok(
                        request.id,
                        Reply::Task {
                            team: team_id(0),
                            role: Role::Solo,
                            member: Box::new(holder),
                        },
                    )
                }
                Command::Swap(swap) => {
                    assert_eq!(swap.task, task(1));
                    assert_eq!(swap.agent.as_str(), "glm-1");
                    if refusal {
                        Response::error(
                            request.id,
                            Failure {
                                kind: FailureKind::Conflict,
                                message: format!("{} is done: nothing to swap", task(1)),
                            },
                        )
                    } else {
                        Response::ok(request.id, Reply::Applied { tasks: vec![task(1)] })
                    }
                }
                command => panic!("unexpected command: {command:?}"),
            };
            write_line(&mut write, &Frame::Response(response).to_line_for(format).unwrap())
                .await
                .unwrap();
        }
    });
    SwapDaemon { socket, accepting }
}

#[tokio::test]
async fn success_line_with_run_stopped() {
    let dir = tempfile::tempdir().unwrap();
    let daemon = daemon(&dir, true, false, false);
    let mut connection = Connection::connect(daemon.socket()).await.unwrap();
    let mut out = Vec::new();
    swap(&mut connection, task(1), "glm-1".parse().unwrap(), &mut out)
        .await
        .unwrap();
    assert_eq!(
        String::from_utf8(out).unwrap(),
        format!("{}: claude-1 → glm-1 (run stopped)\n", task(1))
    );
}

#[tokio::test]
async fn success_line_without_run_stopped() {
    let dir = tempfile::tempdir().unwrap();
    let daemon = daemon(&dir, false, false, false);
    let mut connection = Connection::connect(daemon.socket()).await.unwrap();
    let mut out = Vec::new();
    swap(&mut connection, task(1), "glm-1".parse().unwrap(), &mut out)
        .await
        .unwrap();
    assert_eq!(
        String::from_utf8(out).unwrap(),
        format!("{}: claude-1 → glm-1\n", task(1))
    );
}

/// Runs the public command while the asynchronous fake daemon answers it.
async fn command(dir: &tempfile::TempDir) -> (std::process::ExitCode, String, String) {
    let home = dir.path().to_owned();
    tokio::task::spawn_blocking(move || {
        let environment = crate::Environment::new(Some(home.into_os_string()));
        let mut out = Vec::new();
        let mut err = Vec::new();
        let target = task(1).to_string();
        let code = crate::run_in(&environment, ["agent", "swap", &target, "glm-1"], &mut out, &mut err).unwrap();
        (code, String::from_utf8(out).unwrap(), String::from_utf8(err).unwrap())
    })
    .await
    .unwrap()
}

#[tokio::test]
async fn refusal_is_printed_exit_1() {
    let dir = tempfile::tempdir().unwrap();
    let _daemon = daemon(&dir, false, true, false);
    let (code, out, err) = command(&dir).await;
    assert_eq!(code, std::process::ExitCode::from(1));
    assert_eq!(err, format!("agent: {} is done: nothing to swap\n", task(1)));
    assert_eq!(out, "");
}

#[tokio::test]
async fn older_daemon_says_restart() {
    let dir = tempfile::tempdir().unwrap();
    let _daemon = daemon(&dir, false, false, true);
    let (code, out, err) = command(&dir).await;
    assert_eq!(code, std::process::ExitCode::from(1));
    assert_eq!(
        err,
        "agent: the daemon does not know swap; restart it with this version\n"
    );
    assert_eq!(out, "");
}
