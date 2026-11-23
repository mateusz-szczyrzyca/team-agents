//! The controls of the human through the daemon: `stop`, `pause`, `unpause`, `resume` and `say`, for a task and for a
//! team — what each does to the status, to the run that is going and to the activity, and what it says when it cannot.

use agent_core::protocol::{
    Client, Command, ControlAction, ControlRequest, EventKind, FailureKind, Reply, SayRequest, Target,
};
use agent_core::state::{RunEnd, RunState, State};
use agent_core::status::TaskStatus;
use serde_json::json;
use std::time::{Duration, Instant};

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
async fn controls(daemon: &Daemon, n: u32, expected: usize) -> Vec<serde_json::Value> {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let controls: Vec<_> = daemon
            .activity(n)
            .into_iter()
            .filter(|line| line["kind"] == "control")
            .map(|mut line| {
                line.as_object_mut().unwrap().remove("at");
                line
            })
            .collect();
        if controls.len() >= expected {
            return controls;
        }
        assert!(
            Instant::now() < deadline,
            "history controls did not arrive: {controls:?}"
        );
        tokio::task::yield_now().await;
    }
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
        controls(&daemon, 1, 1).await,
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
        controls(&daemon, 1, 1).await,
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
        controls(&daemon, 1, 2)
            .await
            .iter()
            .map(|line| line["action"].clone())
            .collect::<Vec<_>>(),
        [json!("pause"), json!("unpause")]
    );
    daemon.stop().await.unwrap();
}

#[tokio::test]
async fn resume_starts_a_stopped_task_again() {
    let mut daemon = running().await;
    daemon.ask(control(ControlAction::Stop, of_task(1))).await.unwrap();
    daemon.scenario(1, "done");

    let reply = daemon.ask(control(ControlAction::Resume, of_task(1))).await.unwrap();

    assert_eq!(reply, Reply::Applied { tasks: vec![task(1)] });
    daemon.status_is(1, TaskStatus::Done).await;
    assert_eq!(
        controls(&daemon, 1, 2)
            .await
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
            controls(&daemon, n, 1).await,
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
        controls(&daemon, 1, 1).await.len(),
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
        controls(&daemon, 1, 1).await,
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
    daemon.stop().await.unwrap();
    assert_eq!(controls(&daemon, 1, 0).await, Vec::<serde_json::Value>::new());
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

#[tokio::test]
async fn a_stop_that_arrives_as_a_no_status_run_ends_is_recorded_as_stopped_and_not_retried() {
    let mut daemon = Daemon::new(&solo_state(), None);
    daemon.scenario(1, "nostatus");
    daemon.start().await;

    daemon.ask(control(ControlAction::Stop, of_task(1))).await.unwrap();

    daemon.status_is(1, TaskStatus::Stopped).await;
    let state = daemon.state();
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

#[tokio::test]
async fn a_clock_failure_while_recording_a_control_is_returned_not_silently_dropped() {
    let daemon = Daemon::new(&solo_state(), None);
    let (mut owner, _sinks) = daemon.bare_owner().await;
    let target = of_task(1);
    let aimed = owner.aim(&target, super::Reach::CrewAndReviewers).unwrap();
    daemon.clock.advance(time::Duration::days(-1_000_000));

    let result = owner
        .record_control(&Client::Cli, ControlAction::Pause, &target, &aimed, &[task(1)])
        .await;

    assert!(matches!(result, Err(super::super::OwnerError::Clock(_))), "{result:?}");
}

#[tokio::test]
async fn a_pending_stop_overrules_an_already_classified_no_status_end() {
    use agent_core::decide::outcome::{Classified, ExitStatus, RunFinish, StoppedBy};
    use agent_runtime::run::{FinishedRun, Progress};

    let daemon = Daemon::new(&solo_state(), None);
    let (mut owner, _sinks) = daemon.bare_owner().await;
    owner
        .member_mut(&task(1))
        .unwrap()
        .1
        .transition(TaskStatus::Running)
        .unwrap();
    owner
        .member_mut(&task(1))
        .unwrap()
        .1
        .begin_run(crate::fixtures::run(1), crate::fixtures::at(0), None)
        .unwrap();
    owner.halts.insert(task(1), crate::daemon::control::Halt::Stop);
    let run = FinishedRun {
        classified: Classified {
            end: RunEnd::NoStatus { exit_code: Some(0) },
            pause: None,
        },
        finish: RunFinish {
            exit: ExitStatus::Code(0),
            stopped_by: StoppedBy::Nobody,
        },
        session: None,
        final_message: None,
        progress: Progress::Made,
        used_tool: true,
    };

    owner.finished(&task(1), run).await.unwrap();

    assert_eq!(last_end(&owner.parts.state, 1), RunEnd::Stopped);
    assert_eq!(status(&owner.parts.state, 1), TaskStatus::Stopped);
    assert!(owner.reasons.is_empty());
}

/// A failure that did not report `STATUS: FAILED` is never retried at the owner boundary, even when the run called no
/// tool on a fresh, not-yet-retried session: the decision gets the run's real end, not a fabricated one.
#[tokio::test]
async fn a_failure_that_did_not_report_failed_is_not_retried() {
    use crate::daemon::notice::Notice;

    let daemon = Daemon::new(&solo_state(), None);
    let (mut owner, _sinks) = daemon.bare_owner().await;
    let notice = Notice::Failed {
        message: "I could not do it.".to_owned(),
    };

    let (status, told) = owner
        .settle_failed_without_tool(
            &task(1),
            &RunEnd::NoStatus { exit_code: Some(1) },
            false,
            crate::fixtures::at(0),
            notice,
        )
        .await
        .unwrap();

    assert_eq!(status, TaskStatus::Failed);
    assert!(told.is_some(), "the human is told of a real failure");
}

/// A second run that reported `STATUS: FAILED` without calling a tool — the task was already retried once for a no-tool
/// failure — fails the task with the diagnostic that names the no-tool runs, not the generic failure notice.
#[tokio::test]
async fn a_failed_run_without_a_tool_that_was_already_retried_tells_the_named_reason() {
    use crate::daemon::notice::Notice;

    let daemon = Daemon::new(&solo_state(), None);
    let (mut owner, _sinks) = daemon.bare_owner().await;
    owner
        .member_mut(&task(1))
        .unwrap()
        .1
        .transition(TaskStatus::Running)
        .unwrap();
    owner
        .member_mut(&task(1))
        .unwrap()
        .1
        .begin_run(crate::fixtures::run(1), crate::fixtures::at(0), None)
        .unwrap();
    // A fallen run without a tool call was already retried once: the durable "already retried" flag is that renewal.
    owner
        .parts
        .state
        .renew_task(
            &task(1),
            agent_core::decide::ladder::Trouble::NoToolCall,
            crate::fixtures::at(1),
        )
        .unwrap();
    let notice = Notice::Failed {
        message: "I could not do it.".to_owned(),
    };

    let (status, told) = owner
        .settle_failed_without_tool(
            &task(1),
            &RunEnd::Reported {
                outcome: agent_core::status::RunOutcome::Failed,
            },
            false,
            crate::fixtures::at(3),
            notice,
        )
        .await
        .unwrap();

    assert_eq!(status, TaskStatus::Failed);
    assert!(
        matches!(told.as_ref(), Some(Notice::FailedNoTool { .. })),
        "the human is told the no-tool reason, not a generic failure: {told:?}"
    );
}

/// A later independent new session of the task — begun for another trouble after the one no-tool retry was used — has
/// the retry again: the durable marker is the reason of the last renewal, so the first failed run without a tool of
/// that new session is retried once, as the brief requires, and the renewal of that retry marks it again.
#[tokio::test]
async fn a_first_no_tool_failure_of_a_later_independent_session_is_retried_once() {
    use crate::daemon::notice::Notice;

    let daemon = Daemon::new(&solo_state(), None);
    let (mut owner, _sinks) = daemon.bare_owner().await;
    // run 1 reported FAILED without a tool: the task is started anew once, its one retry.
    let mut member = owner.member_mut(&task(1)).unwrap().1;
    member.transition(TaskStatus::Running).unwrap();
    member
        .begin_run(crate::fixtures::run(1), crate::fixtures::at(0), None)
        .unwrap();
    member
        .end_run(
            crate::fixtures::at(1),
            RunEnd::Reported {
                outcome: agent_core::status::RunOutcome::Failed,
            },
        )
        .unwrap();
    owner
        .parts
        .state
        .renew_task(
            &task(1),
            agent_core::decide::ladder::Trouble::NoToolCall,
            crate::fixtures::at(2),
        )
        .unwrap();
    // run 2, the retried run, ends hung; the ladder starts the task anew for the hang.
    let mut member = owner.member_mut(&task(1)).unwrap().1;
    member
        .begin_run(crate::fixtures::run(2), crate::fixtures::at(3), None)
        .unwrap();
    member.end_run(crate::fixtures::at(4), RunEnd::Hung).unwrap();
    owner
        .parts
        .state
        .renew_task(
            &task(1),
            agent_core::decide::ladder::Trouble::Hang,
            crate::fixtures::at(5),
        )
        .unwrap();
    // run 3, the first run of that independent new session, reported FAILED without a tool: it is retried once.
    let mut member = owner.member_mut(&task(1)).unwrap().1;
    member
        .begin_run(crate::fixtures::run(3), crate::fixtures::at(6), None)
        .unwrap();
    member
        .end_run(
            crate::fixtures::at(7),
            RunEnd::Reported {
                outcome: agent_core::status::RunOutcome::Failed,
            },
        )
        .unwrap();
    let notice = Notice::Failed {
        message: "I could not do it.".to_owned(),
    };

    let (status, told) = owner
        .settle_failed_without_tool(
            &task(1),
            &RunEnd::Reported {
                outcome: agent_core::status::RunOutcome::Failed,
            },
            false,
            crate::fixtures::at(7),
            notice,
        )
        .await
        .unwrap();

    assert_eq!(status, TaskStatus::Queued);
    assert!(told.is_none(), "a retry tells nobody: {told:?}");
    // The renewal of the retry is the marker now: the run that follows it cannot be retried again.
    let renewals = owner
        .parts
        .state
        .find_task(&task(1))
        .unwrap()
        .member
        .renewals()
        .unwrap();
    assert_eq!(renewals.used(), 3);
    assert_eq!(renewals.reason(), agent_core::decide::ladder::Trouble::NoToolCall);
}

/// A later no-tool FAILED — while the durable marker of the exhausted retry survives — is not the retry: a run that
/// succeeded in between ended the retry's session, so the task fails plainly and the human is told the failure, not
/// the no-tool reason.
#[tokio::test]
async fn a_no_tool_failure_after_a_successful_retry_fails_plainly() {
    use crate::daemon::notice::Notice;

    let daemon = Daemon::new(&solo_state(), None);
    let (mut owner, _sinks) = daemon.bare_owner().await;
    // run 1 reported FAILED without a tool: the task is started anew once, its one retry.
    let mut member = owner.member_mut(&task(1)).unwrap().1;
    member.transition(TaskStatus::Running).unwrap();
    member
        .begin_run(crate::fixtures::run(1), crate::fixtures::at(0), None)
        .unwrap();
    member
        .end_run(
            crate::fixtures::at(1),
            RunEnd::Reported {
                outcome: agent_core::status::RunOutcome::Failed,
            },
        )
        .unwrap();
    owner
        .parts
        .state
        .renew_task(
            &task(1),
            agent_core::decide::ladder::Trouble::NoToolCall,
            crate::fixtures::at(2),
        )
        .unwrap();
    // run 2, the retried run, succeeded: the session of the retry is over.
    let mut member = owner.member_mut(&task(1)).unwrap().1;
    member
        .begin_run(crate::fixtures::run(2), crate::fixtures::at(3), None)
        .unwrap();
    member
        .end_run(
            crate::fixtures::at(4),
            RunEnd::Reported {
                outcome: agent_core::status::RunOutcome::Done,
            },
        )
        .unwrap();
    // run 3, later, reported FAILED without a tool while the durable marker of the old retry survives.
    let mut member = owner.member_mut(&task(1)).unwrap().1;
    member
        .begin_run(crate::fixtures::run(3), crate::fixtures::at(5), None)
        .unwrap();
    member
        .end_run(
            crate::fixtures::at(6),
            RunEnd::Reported {
                outcome: agent_core::status::RunOutcome::Failed,
            },
        )
        .unwrap();
    let notice = Notice::Failed {
        message: "I could not do it.".to_owned(),
    };

    let (status, told) = owner
        .settle_failed_without_tool(
            &task(1),
            &RunEnd::Reported {
                outcome: agent_core::status::RunOutcome::Failed,
            },
            false,
            crate::fixtures::at(6),
            notice,
        )
        .await
        .unwrap();

    assert_eq!(status, TaskStatus::Failed);
    assert!(
        matches!(told.as_ref(), Some(Notice::Failed { .. })),
        "a run that is not the retry is told plainly, not as the no-tool reason: {told:?}"
    );
}
