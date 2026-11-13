//! `gate` through the owner: the job it puts on the queue, what it refuses, and, with a daemon and its fake CLI, a run
//! that waits on its gate (not cut by the watchdog) and a gate that is stopped with its task.

use std::path::Path;
use std::process::Command as Process;
use std::time::{Duration, Instant};

use agent_core::protocol::{Client, Command, FailureKind, GateRequest, Reply, Target};
use agent_core::state::{GateOutcome, GateTree, RunEnd, RunState};
use agent_core::status::TaskStatus;
use agent_runtime::run::WatchdogHold;
use tokio::sync::oneshot;
use tokio_util::sync::CancellationToken;

use super::super::fixture::{Daemon, PATIENCE, pair_state};
use super::super::{Message, Owner};
use crate::client::ClientError;
use crate::daemon::gate::{GateJob, GateQueue, QUEUE};
use crate::fixtures::{repo, solo_state, task, team_id};

type Answer = oneshot::Receiver<Result<Reply, agent_core::protocol::Failure>>;

/// Sends the request of the gate of task `n` to the owner and returns the client's end of the answer.
async fn ask(owner: &mut Owner, n: u32) -> Answer {
    let (reply, answer) = oneshot::channel();
    owner
        .handle(Message::Request {
            client: Client::Cli,
            command: Command::Gate(GateRequest { task: task(n) }),
            reply,
        })
        .await
        .unwrap();
    answer
}

/// An owner of the home of `daemon` with a queue nobody serves, and the queue's other end.
async fn queued_owner(daemon: &Daemon) -> (Owner, tokio::sync::mpsc::Receiver<GateJob>) {
    let (owner, _sinks) = daemon.bare_owner().await;
    let (queue, waiting) = GateQueue::new();
    (owner.with_gate(queue), waiting)
}

fn refusal(answer: &mut Answer) -> (FailureKind, String) {
    match answer.try_recv().unwrap() {
        Err(failure) => (failure.kind, failure.message),
        Ok(reply) => panic!("a refusal was expected: {reply:?}"),
    }
}

#[tokio::test]
async fn a_gate_is_queued_with_the_gate_command_the_timeout_and_the_worktree_of_the_task_and_its_client_waits() {
    let daemon = Daemon::new(&pair_state(), None);
    let (mut owner, mut waiting) = queued_owner(&daemon).await;

    let mut answer = ask(&mut owner, 2).await;

    assert!(
        matches!(answer.try_recv(), Err(oneshot::error::TryRecvError::Empty)),
        "the owner answered a gate that has not run"
    );
    let job = waiting.try_recv().unwrap();
    assert_eq!(job.task, task(2));
    assert_eq!(job.team, team_id(0));
    assert_eq!(job.worktree.as_path(), daemon.home.worktree_dir(&repo(), &task(2)));
    assert_eq!(job.command.program().as_path().to_str(), Some("true"));
    assert_eq!(job.timeout, Duration::from_mins(30), "the default of the repository");
    assert!(
        job.hold.is_none(),
        "a task with no run going has no watchdog to hold off"
    );
    assert!(!job.cancel.is_cancelled());
    let told = Reply::Applied { tasks: vec![task(2)] };
    job.reply.send(Ok(told.clone())).unwrap();
    assert_eq!(
        answer.await.unwrap().unwrap(),
        told,
        "the client gets what the service answers"
    );
}

#[tokio::test]
async fn the_timeout_of_the_gate_is_the_one_the_repository_configures() {
    let daemon = Daemon::new(&solo_state(), None);
    let text = std::fs::read_to_string(daemon.home.config_file()).unwrap().replace(
        "gate = [\"true\"]",
        "gate = [\"make\", \"gate\"]\ngate_timeout_minutes = 7",
    );
    std::fs::write(daemon.home.config_file(), text).unwrap();
    let (mut owner, mut waiting) = queued_owner(&daemon).await;

    ask(&mut owner, 1).await;

    let job = waiting.try_recv().unwrap();
    assert_eq!(job.timeout, Duration::from_mins(7));
    assert_eq!(job.command.args().as_slice(), ["gate"]);
}

#[tokio::test]
async fn a_task_with_a_run_going_has_its_watchdog_held_and_its_gate_stopped_with_its_run() {
    let daemon = Daemon::new(&solo_state(), None);
    let (mut owner, mut waiting) = queued_owner(&daemon).await;
    let hold = WatchdogHold::new();
    owner.holds.insert(task(1), hold.clone());
    owner.cancels.insert(task(1), CancellationToken::new());

    ask(&mut owner, 1).await;

    let job = waiting.try_recv().unwrap();
    assert!(
        job.hold.is_some(),
        "the gate holds the watchdog off the run that waits for it"
    );
    assert!(hold.last_held().is_some());
    assert!(!job.cancel.is_cancelled());
    control(
        &mut owner,
        Command::Stop(agent_core::protocol::ControlRequest {
            target: Target::Task(task(1)),
        }),
    )
    .await;
    assert!(job.cancel.is_cancelled(), "stopping the task stops its gate");
    drop(job);
    let released = hold.last_held();
    assert_eq!(hold.last_held(), released, "the hold ends with the job");
}

#[tokio::test]
async fn a_gate_of_a_task_with_no_run_ends_with_the_daemon() {
    let daemon = Daemon::new(&solo_state(), None);
    let (mut owner, mut waiting) = queued_owner(&daemon).await;
    ask(&mut owner, 1).await;
    let job = waiting.try_recv().unwrap();

    owner.shutdown.cancel();

    assert!(job.cancel.is_cancelled());
}

#[tokio::test]
async fn a_gate_of_an_unknown_task_is_not_found_and_nothing_is_queued() {
    let daemon = Daemon::new(&solo_state(), None);
    let (mut owner, mut waiting) = queued_owner(&daemon).await;

    let mut answer = ask(&mut owner, 9).await;

    let (kind, message) = refusal(&mut answer);
    assert_eq!(kind, FailureKind::NotFound);
    assert!(message.contains(&task(9).to_string()), "{message}");
    assert!(
        matches!(waiting.try_recv(), Err(tokio::sync::mpsc::error::TryRecvError::Empty)),
        "a job was queued"
    );
}

#[tokio::test]
async fn a_gate_of_a_repository_the_configuration_no_longer_has_is_refused() {
    let daemon = Daemon::new(&solo_state(), None);
    let text = std::fs::read_to_string(daemon.home.config_file())
        .unwrap()
        .replace("[repos.team-agents]", "[repos.elsewhere]");
    std::fs::write(daemon.home.config_file(), text).unwrap();
    let (mut owner, mut waiting) = queued_owner(&daemon).await;

    let mut answer = ask(&mut owner, 1).await;

    let (kind, message) = refusal(&mut answer);
    assert_eq!(kind, FailureKind::Refused);
    assert!(message.contains("gate") && message.contains("team-agents"), "{message}");
    assert!(
        matches!(waiting.try_recv(), Err(tokio::sync::mpsc::error::TryRecvError::Empty)),
        "a job was queued"
    );
}

#[tokio::test]
async fn a_gate_asked_while_the_queue_is_full_is_refused_and_the_ones_waiting_stay() {
    let daemon = Daemon::new(&solo_state(), None);
    let (mut owner, mut waiting) = queued_owner(&daemon).await;
    let mut accepted = Vec::new();
    for _ in 0..QUEUE {
        accepted.push(ask(&mut owner, 1).await);
    }

    let mut one_too_many = ask(&mut owner, 1).await;

    let (kind, message) = refusal(&mut one_too_many);
    assert_eq!(kind, FailureKind::Refused);
    assert!(message.contains("queue is full"), "{message}");
    for answer in &mut accepted {
        assert!(matches!(answer.try_recv(), Err(oneshot::error::TryRecvError::Empty)));
    }
    let mut jobs = 0;
    while waiting.try_recv().is_ok() {
        jobs += 1;
    }
    assert_eq!(jobs, QUEUE);
}

#[tokio::test]
async fn an_owner_without_a_gate_service_refuses_every_gate() {
    let daemon = Daemon::new(&solo_state(), None);
    let (mut owner, _sinks) = daemon.bare_owner().await;

    let mut answer = ask(&mut owner, 1).await;

    let (kind, message) = refusal(&mut answer);
    assert_eq!(kind, FailureKind::Internal);
    assert!(message.contains("gate"), "{message}");
}

// ---- with a daemon, its gate service and its fake CLI ----------------------------------------------------------

/// Runs git in `dir` for the setup of a test, apart from the configuration of the machine.
fn git(dir: &Path, args: &[&str]) {
    let output = Process::new("git")
        .args(["-c", "commit.gpgsign=false"])
        .args(args)
        .current_dir(dir)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_AUTHOR_NAME", "Fixture Author")
        .env("GIT_AUTHOR_EMAIL", "fixture@example.com")
        .env("GIT_COMMITTER_NAME", "Fixture Author")
        .env("GIT_COMMITTER_EMAIL", "fixture@example.com")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// A daemon whose task 1 runs the fake CLI `scenario` in a git worktree (the files of the fake CLI are ignored, so the
/// tree is clean) and whose repository's gate is `sh -c script`.
fn gated(scenario: &str, script: &str) -> Daemon {
    assert!(!script.contains('\''), "the script is a TOML literal string");
    let daemon = Daemon::new(&solo_state(), None);
    let tree = daemon.scenario(1, scenario);
    git(&tree, &["init", "-q", "-b", "main"]);
    std::fs::write(tree.join(".git/info/exclude"), ".fake-*\n").unwrap();
    std::fs::write(tree.join("README.md"), "Hello\n").unwrap();
    git(&tree, &["add", "README.md"]);
    git(&tree, &["commit", "-q", "-m", "First"]);
    let text = std::fs::read_to_string(daemon.home.config_file())
        .unwrap()
        .replace("gate = [\"true\"]", &format!("gate = [\"sh\", \"-c\", '{script}']"));
    std::fs::write(daemon.home.config_file(), text).unwrap();
    daemon
}

async fn eventually(what: &str, mut check: impl FnMut() -> bool) {
    let deadline = Instant::now() + PATIENCE;
    while !check() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

#[tokio::test]
async fn a_gate_asked_through_the_socket_runs_in_the_worktree_and_answers_with_the_record() {
    let mut daemon = gated("done", "echo gate ran in $PWD");
    daemon.start().await;
    // The item of the solo task is still in flight, so the task waits when its run ends.
    daemon.status_is(1, TaskStatus::Waiting).await;
    let mut events = daemon.subscribe().await;

    let reply = daemon.ask(Command::Gate(GateRequest { task: task(1) })).await.unwrap();

    let Reply::GateRecorded { record } = reply else {
        panic!("{reply:?}");
    };
    assert_eq!(record.tree, GateTree::Clean);
    assert_eq!(record.result.outcome, GateOutcome::Passed);
    assert!(record.counts_as_pass());
    let tree = daemon.home.worktree_dir(&repo(), &task(1));
    assert_eq!(
        std::fs::read_to_string(record.result.log.as_path()).unwrap(),
        format!("gate ran in {}\n", tree.display())
    );
    let finished = events
        .until("the gate's event", |event| match &event.event {
            agent_core::protocol::EventKind::GateFinished { task, outcome, .. } => Some((task.clone(), *outcome)),
            _ => None,
        })
        .await;
    assert_eq!(finished, (task(1), GateOutcome::Passed));
    daemon.stop().await.unwrap();
}

#[tokio::test]
async fn a_run_waiting_on_its_gate_is_not_cut_by_the_watchdog_and_is_when_the_gate_is_over() {
    // The watchdog of the daemon's tests cuts a silent run after one second; the gate takes three.
    let mut daemon = gated("hang", "sleep 3");
    daemon.start().await;
    daemon.status_is(1, TaskStatus::Running).await;
    let worktree = daemon.home.worktree_dir(&repo(), &task(1));
    eventually("the silent run to start", || worktree.join(".fake-child").is_file()).await;
    let mut connection = daemon.connect().await;
    let waiting = tokio::spawn(async move {
        connection
            .request(Client::Cli, Command::Gate(GateRequest { task: task(1) }))
            .await
    });

    tokio::time::sleep(Duration::from_millis(2200)).await;
    let while_waiting = daemon.state();
    let reply = waiting.await.unwrap().unwrap();
    let released = Instant::now();

    let run = &while_waiting.find_task(&task(1)).unwrap().member.runs().to_vec();
    assert_eq!(run.len(), 1, "a second run means the first was cut: {run:?}");
    assert!(run[0].is_running(), "the run was cut while its gate waited: {run:?}");
    assert!(matches!(reply, Reply::GateRecorded { .. }), "{reply:?}");
    daemon
        .state_where("the run to be cut once the gate is over", |state| {
            state.find_task(&task(1)).is_some_and(|found| {
                found
                    .member
                    .runs()
                    .iter()
                    .any(|run| matches!(run.state(), RunState::Ended { end: RunEnd::Hung, .. }))
            })
        })
        .await;
    assert!(
        released.elapsed() >= Duration::from_millis(800),
        "the run was cut {:?} after its gate, before the silence of the limit",
        released.elapsed()
    );
    daemon.stop().await.unwrap();
}

#[tokio::test]
async fn stopping_a_task_stops_its_gate_which_answers_the_client_and_records_nothing() {
    let marker = "started";
    let mut daemon = gated("hang", &format!("echo $$ > {marker}; sleep 30"));
    daemon.start().await;
    daemon.status_is(1, TaskStatus::Running).await;
    let worktree = daemon.home.worktree_dir(&repo(), &task(1));
    let mut connection = daemon.connect().await;
    let waiting = tokio::spawn(async move {
        connection
            .request(Client::Cli, Command::Gate(GateRequest { task: task(1) }))
            .await
    });
    eventually("the gate to start", || worktree.join(marker).is_file()).await;

    daemon
        .ask(Command::Stop(agent_core::protocol::ControlRequest {
            target: Target::Task(task(1)),
        }))
        .await
        .unwrap();
    let answer = waiting.await.unwrap();

    assert!(
        matches!(&answer, Err(ClientError::Refused(failure)) if failure.kind == FailureKind::Conflict),
        "{answer:?}"
    );
    assert!(!daemon.home.gate_record_file(&task(1)).exists());
    daemon.stop().await.unwrap();
}

/// The owner's answer to a control of the human for the task 1.
async fn control(owner: &mut Owner, command: Command) {
    let (reply, mut answer) = oneshot::channel();
    owner
        .handle(Message::Request {
            client: Client::Cli,
            command,
            reply,
        })
        .await
        .unwrap();
    assert!(matches!(answer.try_recv(), Ok(Ok(Reply::Applied { .. }))));
}

#[tokio::test]
async fn stopping_a_task_with_no_run_stops_its_gate() {
    let daemon = Daemon::new(&solo_state(), None);
    let (mut owner, mut waiting) = queued_owner(&daemon).await;
    ask(&mut owner, 1).await;
    let job = waiting.try_recv().unwrap();
    assert!(!job.cancel.is_cancelled());

    control(
        &mut owner,
        Command::Stop(agent_core::protocol::ControlRequest {
            target: Target::Task(task(1)),
        }),
    )
    .await;

    assert!(job.cancel.is_cancelled(), "the gate of a stopped task runs on");
}

#[tokio::test]
async fn pausing_a_task_with_no_run_stops_its_gate() {
    let daemon = Daemon::new(&solo_state(), None);
    let (mut owner, mut waiting) = queued_owner(&daemon).await;
    ask(&mut owner, 1).await;
    let job = waiting.try_recv().unwrap();

    control(
        &mut owner,
        Command::Pause(agent_core::protocol::ControlRequest {
            target: Target::Task(task(1)),
        }),
    )
    .await;

    assert!(job.cancel.is_cancelled(), "the gate of a paused task runs on");
}

#[tokio::test]
async fn stopping_one_task_leaves_the_gate_of_another_task_with_no_run_going() {
    let daemon = Daemon::new(&pair_state(), None);
    let (mut owner, mut waiting) = queued_owner(&daemon).await;
    ask(&mut owner, 1).await;
    ask(&mut owner, 2).await;
    let first = waiting.try_recv().unwrap();
    let second = waiting.try_recv().unwrap();

    control(
        &mut owner,
        Command::Stop(agent_core::protocol::ControlRequest {
            target: Target::Task(task(2)),
        }),
    )
    .await;

    assert!(!first.cancel.is_cancelled());
    assert!(second.cancel.is_cancelled());
}

#[tokio::test]
async fn stopping_a_task_whose_run_ended_while_its_gate_waited_stops_the_gate() {
    let daemon = Daemon::new(&solo_state(), None);
    let (mut owner, mut waiting) = queued_owner(&daemon).await;
    owner.holds.insert(task(1), WatchdogHold::new());
    owner.cancels.insert(task(1), CancellationToken::new());
    ask(&mut owner, 1).await;
    let job = waiting.try_recv().unwrap();

    // The end of a run drops what it had: the token that stopped it, and its hold.
    owner.cancels.remove(&task(1));
    owner.holds.remove(&task(1));
    control(
        &mut owner,
        Command::Stop(agent_core::protocol::ControlRequest {
            target: Target::Task(task(1)),
        }),
    )
    .await;

    assert!(job.cancel.is_cancelled(), "the gate outlived the run that asked for it");
}

#[tokio::test]
async fn pausing_a_task_whose_run_ended_while_its_gate_waited_stops_the_gate() {
    let daemon = Daemon::new(&solo_state(), None);
    let (mut owner, mut waiting) = queued_owner(&daemon).await;
    owner.cancels.insert(task(1), CancellationToken::new());
    ask(&mut owner, 1).await;
    let job = waiting.try_recv().unwrap();

    owner.cancels.remove(&task(1));
    control(
        &mut owner,
        Command::Pause(agent_core::protocol::ControlRequest {
            target: Target::Task(task(1)),
        }),
    )
    .await;

    assert!(job.cancel.is_cancelled(), "the gate outlived the run that asked for it");
}
