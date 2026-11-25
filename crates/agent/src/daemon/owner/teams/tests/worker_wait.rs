//! A request that no worker of the pool takes for `[workers] wait_seconds` is returned to its requester; the daemon
//! notes since when each queued request waits, and with `wait_seconds = 0` it waits for ever.

use std::collections::BTreeSet;

use agent_runtime::agentbus::DelegationUpdate;

use super::*;

/// What the requester of the second request is told when it has waited 61 seconds: whole minutes, as the text says.
const RETURNED: &str = "Nobody took your request H2 (no step) for 1 minutes: no worker of the pool was free. Do the step \
                        yourself now.";

/// The seconds the clock moves after the second request is queued: one past the limit of the tests that return it.
const PAST_LIMIT: i64 = 61;

/// A fixture with a requester `claude-1` and a pool worker `glm-1` that holds the first request it takes, with the lines
/// of `[workers]` in its configuration.
fn fixture(workers: &str) -> Fixture {
    let f = Fixture::new(&[
        investigator("inv-opus", "sleep"),
        investigator("inv-codex", "sleep"),
        investigator("inv-gemini", "sleep"),
        normal("claude-1", "sleep"),
        worker("glm-1", "hold"),
    ]);
    let config = std::fs::read_to_string(f.home.config_file()).unwrap();
    std::fs::write(f.home.config_file(), format!("{config}\n[workers]\n{workers}\n")).unwrap();
    f
}

/// The requester's team, its task and two requests of it: the first is held by the only worker of the pool, the second
/// has no worker to go to and is noted as waiting.
struct Waiting {
    /// The task of the requester.
    task: TaskId,
    /// The second request, which waits.
    second: RequestId,
}

impl Waiting {
    /// The note of the wait of the second request.
    fn note(&self, f: &Fixture) -> PathBuf {
        f.home
            .task_dir(&self.task)
            .join(format!("worker-wait-{}.json", self.second.number()))
    }
}

/// Starts the daemon of `f`, a team of the requester and two requests, and waits until the second is noted as waiting.
async fn queue_two_requests(f: &mut Fixture) -> Waiting {
    f.start().await;
    let (team, tasks) = f.start_team(&["inv-opus", "claude-1"], PLAIN).await;
    for task in &tasks {
        f.status_is(task, TaskStatus::Waiting).await;
    }
    let pair = Pair {
        team,
        tasks: vec![tasks[1].clone()],
        requester: "claude-1",
        other: "inv-opus",
        worker: "glm-1",
    };
    let first = f.delegate(&pair, "Count the files.").await;
    f.request_with(first, DelegationStatus::Running).await;
    let second = f.delegate(&pair, "Count the lines.").await;
    let waiting = Waiting {
        task: tasks[1].clone(),
        second,
    };
    f.wait_for("the note of the wait", || waiting.note(f).exists()).await;
    waiting
}

/// Asks the daemon for its status and waits for the answer: the owner answers between two turns of its loop.
async fn looked(f: &Fixture) {
    let reply = f
        .connect()
        .await
        .request(Client::Cli, Command::Status(StatusRequest { target: None }))
        .await
        .unwrap();
    assert!(matches!(reply, Reply::Overview { .. }), "{reply:?}");
}

/// The records of the daemon's log for `event`, once the daemon has stopped.
fn logged(f: &Fixture, event: LogEvent) -> Vec<agent_core::log::LogEntry> {
    Reader::open(&f.home.history_file())
        .unwrap()
        .daemon_logs()
        .unwrap()
        .into_iter()
        .filter(|entry| entry.event() == event)
        .collect()
}

#[tokio::test]
async fn a_request_nobody_takes_is_returned_to_its_requester_when_the_wait_is_over() {
    let mut f = fixture("wait_seconds = 60");
    let waiting = queue_two_requests(&mut f).await;
    let since = std::fs::read_to_string(waiting.note(&f)).unwrap();
    assert!(since.contains("\"request\":\"H2\""), "{since}");

    f.clock.advance(time::Duration::seconds(PAST_LIMIT));
    let request = f.request_with(waiting.second, DelegationStatus::Failed).await;
    f.wait_for("the note to go", || !waiting.note(&f).exists()).await;
    f.stop().await.unwrap();

    assert_eq!(request.summary, RETURNED);
    let returned = logged(&f, LogEvent::WorkerWaitReturned);
    assert_eq!(returned.len(), 1, "{returned:?}");
    assert_eq!(returned[0].fields()["request"], "H2");
    assert_eq!(returned[0].task(), Some(&waiting.task));
    assert_eq!(returned[0].fields()["step"], "no step");
    assert_eq!(returned[0].fields()["seconds"], PAST_LIMIT);
}

#[tokio::test]
async fn a_request_waits_for_ever_when_the_limit_is_zero() {
    let mut f = fixture("wait_seconds = 0");
    let waiting = queue_two_requests(&mut f).await;

    f.clock.advance(time::Duration::seconds(700));
    for _ in 0..3 {
        looked(&f).await;
    }
    let request = f.bus().helper_get(waiting.second).await.unwrap();
    f.stop().await.unwrap();

    assert_eq!(request.status, DelegationStatus::Queued);
    assert!(waiting.note(&f).exists());
    assert_eq!(logged(&f, LogEvent::WorkerWaitReturned), []);
}

#[tokio::test]
async fn the_note_of_a_request_that_left_the_queue_is_removed() {
    let mut f = fixture("wait_seconds = 0");
    let waiting = queue_two_requests(&mut f).await;

    f.bus()
        .helper_set(
            waiting.second,
            &DelegationUpdate {
                status: Some(DelegationStatus::Failed),
                task: None,
                summary: Some("Cancelled.".to_owned()),
            },
        )
        .await
        .unwrap();
    f.wait_for("the note to go", || !waiting.note(&f).exists()).await;
    f.stop().await.unwrap();
}

/// Records `worker_wait_returned` of a real request nobody took into `observed`, after checking its row.
pub(in crate::daemon::owner) async fn record_worker_wait_returned(observed: &mut BTreeSet<LogEvent>) {
    let mut f = fixture("wait_seconds = 60");
    let waiting = queue_two_requests(&mut f).await;
    f.clock.advance(time::Duration::seconds(PAST_LIMIT));
    f.request_with(waiting.second, DelegationStatus::Failed).await;
    f.stop().await.unwrap();

    let returned = logged(&f, LogEvent::WorkerWaitReturned);
    assert_eq!(returned.len(), 1, "{returned:?}");
    assert_eq!(returned[0].level(), agent_core::log::LogLevel::Info);
    observed.insert(LogEvent::WorkerWaitReturned);
}
