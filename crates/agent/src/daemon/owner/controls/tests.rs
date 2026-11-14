//! The controls of the human through the daemon: `stop`, `pause`, `unpause`, `resume` and `say`, for a task and for a
//! team — what each does to the status, to the run that is going and to the activity, and what it says when it cannot.

use agent_core::protocol::{
    Client, Command, ControlAction, ControlRequest, EventKind, FailureKind, Reply, SayRequest, Target,
};
use agent_core::state::{RunEnd, RunState, State};
use agent_core::status::TaskStatus;
use serde_json::json;

use super::super::fixture::{Daemon, agentbus_binary, pair_state, reviewed_pair_state};
use crate::client::ClientError;
use crate::daemon::control::Steer;
use crate::fixtures::{solo_state, task, team_id};

/// A daemon whose task 1 is running a CLI that never ends by itself.
async fn running() -> Daemon {
    let mut daemon = Daemon::new(&solo_state(), None);
    daemon.scenario(1, "busy");
    daemon.start().await;
    daemon.status_is(1, TaskStatus::Running).await;
    daemon
}

/// A daemon whose two tasks, a coordinator and a member of one team, are running CLIs that never end by themselves.
async fn running_pair() -> Daemon {
    let mut daemon = Daemon::new(&pair_state(), None);
    daemon.scenario(1, "busy");
    daemon.scenario(2, "busy");
    daemon.start().await;
    daemon.status_is(1, TaskStatus::Running).await;
    daemon.status_is(2, TaskStatus::Running).await;
    daemon
}

fn control(action: ControlAction, target: Target) -> Command {
    let request = ControlRequest { target };
    match action {
        ControlAction::Stop => Command::Stop(request),
        ControlAction::Pause => Command::Pause(request),
        ControlAction::Unpause => Command::Unpause(request),
        ControlAction::Resume => Command::Resume(request),
        ControlAction::Say => panic!("`say` has a request of its own"),
    }
}

fn of_task(n: u32) -> Target {
    Target::Task(task(n))
}

fn refusal(result: Result<Reply, ClientError>) -> (FailureKind, String) {
    match result {
        Err(ClientError::Refused(failure)) => (failure.kind, failure.message),
        other => panic!("a refusal was expected: {other:?}"),
    }
}

fn status(state: &State, n: u32) -> TaskStatus {
    state.find_task(&task(n)).unwrap().member.status()
}

/// How the last run of task `n` ended.
fn last_end(state: &State, n: u32) -> RunEnd {
    let member = state.find_task(&task(n)).unwrap().member;
    match member.runs().last().unwrap().state() {
        RunState::Ended { end, .. } => end.clone(),
        RunState::Running => panic!("the run is still going"),
    }
}

/// The control lines of the activity log of task `n`.
fn controls(daemon: &Daemon, n: u32) -> Vec<serde_json::Value> {
    daemon
        .activity(n)
        .into_iter()
        .filter(|line| line["kind"] == "control")
        .map(|mut line| {
            line.as_object_mut().unwrap().remove("at");
            line
        })
        .collect()
}

#[tokio::test]
async fn stop_ends_the_run_and_the_status_says_stopped_when_the_answer_comes() {
    let mut daemon = running().await;

    let reply = daemon.ask(control(ControlAction::Stop, of_task(1))).await.unwrap();

    assert_eq!(reply, Reply::Applied { tasks: vec![task(1)] });
    let state = daemon.state();
    assert_eq!(status(&state, 1), TaskStatus::Stopped);
    assert_eq!(last_end(&state, 1), RunEnd::Stopped);
    daemon.stop().await.unwrap();
}

#[tokio::test]
async fn the_activity_names_who_stopped_the_task_and_the_event_is_published() {
    let mut daemon = running().await;
    let mut events = daemon.subscribe().await;

    daemon.ask(control(ControlAction::Stop, of_task(1))).await.unwrap();

    assert_eq!(
        controls(&daemon, 1),
        [json!({"kind": "control", "action": "stop", "by": {"kind": "cli"}})]
    );
    let (task_of, action, by) = events
        .until("the control", |event| match &event.event {
            EventKind::Controlled { task, action, by, .. } => Some((task.clone(), *action, by.clone())),
            _ => None,
        })
        .await;
    assert_eq!((task_of, action, by), (Some(task(1)), ControlAction::Stop, Client::Cli));
    daemon.stop().await.unwrap();
}

#[tokio::test]
async fn the_activity_names_the_dashboard_login_that_paused_the_task() {
    let mut daemon = running().await;
    let client = Client::Dashboard {
        login: "you@example.com".parse().unwrap(),
    };

    daemon
        .connect()
        .await
        .request(client, control(ControlAction::Pause, of_task(1)))
        .await
        .unwrap();

    assert_eq!(
        controls(&daemon, 1),
        [json!({"kind": "control", "action": "pause", "by": {"kind": "dashboard", "login": "you@example.com"}})]
    );
    daemon.stop().await.unwrap();
}

#[tokio::test]
async fn pause_holds_the_task_and_unpause_resumes_its_session() {
    let mut daemon = running().await;
    let session = daemon.state().find_task(&task(1)).unwrap().member.runs()[0]
        .session()
        .cloned();

    let reply = daemon.ask(control(ControlAction::Pause, of_task(1))).await.unwrap();

    assert_eq!(reply, Reply::Applied { tasks: vec![task(1)] });
    let state = daemon.state();
    assert_eq!(status(&state, 1), TaskStatus::Paused);
    assert_eq!(last_end(&state, 1), RunEnd::Stopped);
    // A paused task stays so however long it takes: no wake-up is set for a pause of the human.
    daemon.clock.advance(time::Duration::hours(24));
    daemon
        .ask(Command::Status(agent_core::protocol::StatusRequest { target: None }))
        .await
        .unwrap();
    assert_eq!(status(&daemon.state(), 1), TaskStatus::Paused);

    daemon.scenario(1, "done");
    let reply = daemon.ask(control(ControlAction::Unpause, of_task(1))).await.unwrap();

    assert_eq!(reply, Reply::Applied { tasks: vec![task(1)] });
    let state = daemon.status_is(1, TaskStatus::Done).await;
    assert_eq!(state.find_task(&task(1)).unwrap().member.runs().len(), 2);
    let args = daemon.recorded(1, ".fake-args");
    let session = session.expect("Claude Code is given its session when the run starts");
    assert!(args.contains("--resume") && args.contains(session.as_str()), "{args}");
    assert_eq!(
        controls(&daemon, 1)
            .iter()
            .map(|line| line["action"].clone())
            .collect::<Vec<_>>(),
        [json!("pause"), json!("unpause")]
    );
    daemon.stop().await.unwrap();
}

#[tokio::test]
async fn resume_starts_a_stopped_task_again() {
    let mut daemon = running_pair().await;
    daemon.ask(control(ControlAction::Stop, of_task(1))).await.unwrap();
    daemon.scenario(1, "done");

    let reply = daemon.ask(control(ControlAction::Resume, of_task(1))).await.unwrap();

    assert_eq!(reply, Reply::Applied { tasks: vec![task(1)] });
    let state = daemon
        .state_where("task 1 to have run again", |state| {
            let runs = state.find_task(&task(1)).unwrap().member.runs();
            runs.len() == 2 && matches!(runs.last().map(|run| run.state()), Some(RunState::Ended { .. }))
        })
        .await;
    assert_eq!(
        last_end(&state, 1),
        RunEnd::Reported {
            outcome: agent_core::status::RunOutcome::Done,
        }
    );
    assert_eq!(
        controls(&daemon, 1)
            .iter()
            .map(|line| line["action"].clone())
            .collect::<Vec<_>>(),
        [json!("stop"), json!("resume")]
    );
    daemon.stop().await.unwrap();
}

#[tokio::test]
async fn stopping_a_team_stops_every_task_of_it_and_the_event_is_for_the_team() {
    let mut daemon = running_pair().await;
    let mut events = daemon.subscribe().await;

    let reply = daemon
        .ask(control(ControlAction::Stop, Target::Team(team_id(0))))
        .await
        .unwrap();

    assert_eq!(
        reply,
        Reply::Applied {
            tasks: vec![task(1), task(2)]
        }
    );
    let state = daemon.state();
    assert_eq!(
        (status(&state, 1), status(&state, 2)),
        (TaskStatus::Stopped, TaskStatus::Stopped)
    );
    let (task_of, action) = events
        .until("the control", |event| match &event.event {
            EventKind::Controlled { task, action, .. } => Some((task.clone(), *action)),
            _ => None,
        })
        .await;
    assert_eq!((task_of, action), (None, ControlAction::Stop));
    for n in [1, 2] {
        assert_eq!(
            controls(&daemon, n),
            [json!({"kind": "control", "action": "stop", "by": {"kind": "cli"}})],
            "task {n}"
        );
    }
    daemon.stop().await.unwrap();
}

#[tokio::test]
async fn stopping_a_team_stops_its_reviewers_too_and_a_word_to_it_is_not_for_them() {
    let mut daemon = Daemon::new(&reviewed_pair_state(), Some(agentbus_binary()));
    for n in [1, 2, 3] {
        daemon.scenario(n, "busy");
    }
    daemon.start().await;
    for n in [1, 2, 3] {
        daemon.status_is(n, TaskStatus::Running).await;
    }

    let said = daemon.ask(say(Target::Team(team_id(0)), "Hello.")).await.unwrap();
    let stopped = daemon
        .ask(control(ControlAction::Stop, Target::Team(team_id(0))))
        .await
        .unwrap();

    assert_eq!(
        said,
        Reply::Applied {
            tasks: vec![task(1), task(2)]
        }
    );
    assert_eq!(
        stopped,
        Reply::Applied {
            tasks: vec![task(1), task(2), task(3)]
        }
    );
    let state = daemon.state();
    assert_eq!(status(&state, 3), TaskStatus::Stopped);
    daemon.stop().await.unwrap();
}

#[tokio::test]
async fn a_task_that_waits_for_answers_is_not_stopped_so_that_its_round_stays_the_one_to_answer() {
    let mut daemon = Daemon::new(&pair_state(), None);
    daemon.scenario(1, "busy");
    let file = daemon.questions_file(2, 1);
    daemon.scenario(2, &format!("questions {}", file.display()));
    daemon.start().await;
    daemon.status_is(1, TaskStatus::Running).await;
    daemon.status_is(2, TaskStatus::Questions).await;

    let (kind, message) = refusal(daemon.ask(control(ControlAction::Stop, of_task(2))).await);
    let reply = daemon
        .ask(control(ControlAction::Stop, Target::Team(team_id(0))))
        .await
        .unwrap();

    assert_eq!(kind, FailureKind::Conflict);
    assert!(message.contains("waits for your answers"), "{message}");
    assert_eq!(reply, Reply::Applied { tasks: vec![task(1)] });
    let state = daemon.state();
    assert_eq!(
        (status(&state, 1), status(&state, 2)),
        (TaskStatus::Stopped, TaskStatus::Questions)
    );
    assert!(state.find_task(&task(2)).unwrap().member.open_question().is_some());
    daemon.stop().await.unwrap();
}

#[tokio::test]
async fn pausing_and_unpausing_a_team_reaches_every_task_of_it() {
    let mut daemon = running_pair().await;

    daemon
        .ask(control(ControlAction::Pause, Target::Team(team_id(0))))
        .await
        .unwrap();

    let state = daemon.state();
    assert_eq!(
        (status(&state, 1), status(&state, 2)),
        (TaskStatus::Paused, TaskStatus::Paused)
    );
    daemon.scenario(1, "done");
    daemon.scenario(2, "done");
    let reply = daemon
        .ask(control(ControlAction::Unpause, Target::Team(team_id(0))))
        .await
        .unwrap();
    assert_eq!(
        reply,
        Reply::Applied {
            tasks: vec![task(1), task(2)]
        }
    );
    daemon
        .state_where("both tasks run again", |state| {
            state.find_task(&task(1)).unwrap().member.runs().len() == 2
                && state.find_task(&task(2)).unwrap().member.runs().len() == 2
        })
        .await;
    daemon.stop().await.unwrap();
}

#[tokio::test]
async fn a_control_that_does_nothing_to_a_task_says_why() {
    let mut daemon = running().await;

    let (kind, message) = refusal(daemon.ask(control(ControlAction::Unpause, of_task(1))).await);
    assert_eq!(kind, FailureKind::Conflict);
    assert!(
        message.contains(task(1).as_str()) && message.contains("not paused"),
        "{message}"
    );

    let (kind, message) = refusal(daemon.ask(control(ControlAction::Resume, of_task(1))).await);
    assert_eq!(kind, FailureKind::Conflict);
    assert!(message.contains("already going"), "{message}");

    daemon.ask(control(ControlAction::Stop, of_task(1))).await.unwrap();
    let (kind, message) = refusal(daemon.ask(control(ControlAction::Stop, of_task(1))).await);
    assert_eq!(kind, FailureKind::Conflict);
    assert!(message.contains("already stopped"), "{message}");
    assert_eq!(
        controls(&daemon, 1).len(),
        1,
        "a control that did nothing is not in the activity"
    );
    daemon.stop().await.unwrap();
}

#[tokio::test]
async fn a_control_of_a_task_or_team_that_is_not_there_is_not_found() {
    let mut daemon = running().await;

    let (kind, _) = refusal(daemon.ask(control(ControlAction::Stop, of_task(9))).await);
    assert_eq!(kind, FailureKind::NotFound);
    let (kind, _) = refusal(
        daemon
            .ask(control(ControlAction::Pause, Target::Team(team_id(9))))
            .await,
    );
    assert_eq!(kind, FailureKind::NotFound);
    daemon.stop().await.unwrap();
}

#[tokio::test]
async fn a_stopped_task_stays_stopped_across_a_restart_of_the_daemon() {
    let mut daemon = running().await;
    daemon.ask(control(ControlAction::Stop, of_task(1))).await.unwrap();
    daemon.stop().await.unwrap();

    daemon.start().await;

    // The status is asked for, which the owner answers after its first turn: nothing started the task.
    daemon
        .ask(Command::Status(agent_core::protocol::StatusRequest { target: None }))
        .await
        .unwrap();
    assert_eq!(status(&daemon.state(), 1), TaskStatus::Stopped);
    daemon.stop().await.unwrap();
}

// ---- say ------------------------------------------------------------------------------------------------------

fn say(target: Target, text: &str) -> Command {
    Command::Say(SayRequest {
        target,
        text: text.to_owned().try_into().unwrap(),
    })
}

/// The messages the bus holds, a line each.
fn messages(daemon: &Daemon) -> String {
    std::fs::read_to_string(daemon.home.root().join("bus/messages.jsonl")).unwrap_or_default()
}

#[tokio::test]
async fn a_word_to_a_task_is_a_message_to_its_agent_from_the_human() {
    let mut daemon = Daemon::new(&solo_state(), Some(agentbus_binary()));
    daemon.scenario(1, "busy");
    daemon.start().await;
    daemon.status_is(1, TaskStatus::Running).await;

    let reply = daemon.ask(say(of_task(1), "Prefer the disk.")).await.unwrap();

    assert_eq!(reply, Reply::Applied { tasks: vec![task(1)] });
    let held = messages(&daemon);
    assert!(
        held.contains("Prefer the disk.") && held.contains("claude-1") && held.contains("human"),
        "{held}"
    );
    assert_eq!(
        controls(&daemon, 1),
        [json!({"kind": "control", "action": "say", "by": {"kind": "cli"}})]
    );
    assert_eq!(
        status(&daemon.state(), 1),
        TaskStatus::Running,
        "talking changes no status"
    );
    daemon.stop().await.unwrap();
}

#[tokio::test]
async fn a_word_to_a_team_is_one_message_to_the_team() {
    let mut daemon = Daemon::new(&pair_state(), Some(agentbus_binary()));
    daemon.scenario(1, "busy");
    daemon.scenario(2, "busy");
    daemon.start().await;
    daemon.status_is(2, TaskStatus::Running).await;

    let reply = daemon
        .ask(say(Target::Team(team_id(0)), "Stop at the tests."))
        .await
        .unwrap();

    assert_eq!(
        reply,
        Reply::Applied {
            tasks: vec![task(1), task(2)]
        }
    );
    let held = messages(&daemon);
    assert_eq!(
        held.lines().filter(|line| line.contains("Stop at the tests.")).count(),
        1,
        "{held}"
    );
    assert!(held.contains(&format!("team:{}", team_id(0))), "{held}");
    daemon.stop().await.unwrap();
}

#[tokio::test]
async fn a_word_to_a_solo_team_reaches_its_only_agent() {
    let mut daemon = Daemon::new(&solo_state(), Some(agentbus_binary()));
    daemon.scenario(1, "busy");
    daemon.start().await;
    daemon.status_is(1, TaskStatus::Running).await;

    daemon.ask(say(Target::Team(team_id(0)), "Hello.")).await.unwrap();

    let held = messages(&daemon);
    assert!(held.contains("Hello.") && held.contains("claude-1"), "{held}");
    daemon.stop().await.unwrap();
}

#[tokio::test]
async fn a_word_that_the_bus_does_not_take_is_refused_and_not_recorded() {
    let mut daemon = Daemon::new(&solo_state(), Some(std::path::Path::new("/nonexistent/agentbus")));
    daemon.scenario(1, "busy");
    daemon.start().await;
    daemon.status_is(1, TaskStatus::Running).await;

    let (kind, message) = refusal(daemon.ask(say(of_task(1), "Hello.")).await);

    assert_eq!(kind, FailureKind::Internal);
    assert!(message.contains("agentbus"), "{message}");
    assert_eq!(controls(&daemon, 1), Vec::<serde_json::Value>::new());
    daemon.stop().await.unwrap();
}

#[tokio::test]
async fn a_word_to_a_task_that_is_done_is_refused() {
    let mut daemon = Daemon::new(&solo_state(), Some(agentbus_binary()));
    daemon.scenario(1, "done");
    daemon.start().await;
    daemon.status_is(1, TaskStatus::Done).await;

    let (kind, message) = refusal(daemon.ask(say(of_task(1), "Hello.")).await);

    assert_eq!(kind, FailureKind::Conflict);
    assert!(message.contains("done"), "{message}");
    daemon.stop().await.unwrap();
}

/// A daemon whose task 1 hangs: the watchdog has told the run to stop and the run is taking its time, so that the run
/// ends by itself, as a hung one, while the human's word arrives.
async fn hung_and_ending() -> Daemon {
    let mut daemon = Daemon::new(&solo_state(), None);
    let dir = daemon.scenario(1, "hang-slow-stop");
    daemon.start().await;
    let told = dir.join(".fake-term");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    while !told.exists() {
        assert!(
            std::time::Instant::now() < deadline,
            "the watchdog never stopped the run"
        );
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    daemon
}

#[tokio::test]
async fn a_stop_that_arrives_as_a_hung_run_ends_is_recorded_as_the_end_of_the_run() {
    let mut daemon = hung_and_ending().await;

    let reply = daemon.ask(control(ControlAction::Stop, of_task(1))).await.unwrap();

    assert_eq!(reply, Reply::Applied { tasks: vec![task(1)] });
    let state = daemon.state();
    assert_eq!(status(&state, 1), TaskStatus::Stopped);
    assert_eq!(
        last_end(&state, 1),
        RunEnd::Stopped,
        "the human stopped it, the watchdog did not"
    );
    daemon.stop().await.unwrap();
}

#[tokio::test]
async fn a_pause_that_arrives_as_a_hung_run_ends_holds_the_task_and_is_recorded_as_the_end_of_the_run() {
    let mut daemon = hung_and_ending().await;

    daemon.ask(control(ControlAction::Pause, of_task(1))).await.unwrap();

    let state = daemon.state();
    assert_eq!(status(&state, 1), TaskStatus::Paused);
    assert_eq!(last_end(&state, 1), RunEnd::Stopped);
    daemon.stop().await.unwrap();
}

// ---- what a task that is queued has been told ------------------------------------------------------------------

/// A queued task 1 that its team gave a reason to start with, in an owner that has not started it.
async fn queued_with_a_reason() -> (Daemon, super::super::Owner, super::super::fixture::Sinks) {
    let daemon = Daemon::new(&solo_state(), None);
    let (mut owner, sinks) = daemon.bare_owner().await;
    owner.reasons.insert(task(1), crate::daemon::team::Why::Woken);
    (daemon, owner, sinks)
}

#[tokio::test]
async fn a_task_that_is_stopped_before_it_starts_forgets_the_reason_it_was_queued_with() {
    let (_daemon, mut owner, _sinks) = queued_with_a_reason().await;

    owner
        .control_request(&Client::Cli, Steer::Stop, &of_task(1))
        .await
        .unwrap()
        .unwrap();

    assert!(owner.reasons.is_empty(), "{:?}", owner.reasons);
}

#[tokio::test]
async fn a_task_that_is_paused_before_it_starts_keeps_the_reason_it_was_queued_with() {
    let (_daemon, mut owner, _sinks) = queued_with_a_reason().await;

    owner
        .control_request(&Client::Cli, Steer::Pause, &of_task(1))
        .await
        .unwrap()
        .unwrap();

    assert_eq!(owner.reasons.get(&task(1)), Some(&crate::daemon::team::Why::Woken));
}

// ---- a halt whose run is gone ----------------------------------------------------------------------------------

#[tokio::test]
async fn a_stop_of_a_task_that_has_no_run_is_not_left_to_stop_its_next_run() {
    let daemon = Daemon::new(&solo_state(), None);
    let (mut owner, _sinks) = daemon.bare_owner().await;
    // The status says a run is going; no run of the owner is: what a run that ended before the control came leaves.
    owner
        .member_mut(&task(1))
        .unwrap()
        .1
        .transition(TaskStatus::Running)
        .unwrap();

    owner
        .control_request(&Client::Cli, Steer::Stop, &of_task(1))
        .await
        .unwrap()
        .unwrap();

    assert!(owner.halts.is_empty(), "{:?}", owner.halts);
}
