//! The causes of the daemon's own log that the first event names left without a name: each is provoked through the code
//! that has it — a running daemon with fake CLIs and a fake GitHub server, a bare owner, or a child process of this test
//! binary — and its row is read back from `history.db` with its name, level and IDs.

use std::collections::BTreeSet;
use std::path::Path;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use agent_core::config::{AbsolutePath, Program, Secrets};
use agent_core::history::{HistoryEvent, MaxTextBytes};
use agent_core::ids::{AgentName, TaskId};
use agent_core::log::{LogEntry, LogEvent, LogLevel};
use agent_core::protocol::{
    AgentList, Client, Command, ControlRequest, FailureKind, RunRequest, StatusRequest, Target,
};
use agent_core::state::{ItemPhase, MergeMode, State, TaskListText};
use agent_core::status::TaskStatus;
use agent_runtime::agentbus::{AgentBus, Delegation};
use agent_runtime::clock::SharedClock;
use agent_runtime::history::{DEFAULT_QUEUE, History, HistoryDb, HistoryLayer, Problem, Reader, Reporter, Setup};
use tracing::instrument::WithSubscriber;
use tracing_subscriber::layer::SubscriberExt;

use super::rig::{ONE, PATIENCE, Rig, git, investigator, normal};
use crate::daemon::owner::Message;
use crate::daemon::owner::fixture::{Daemon, answer};
use crate::fixtures::{solo_state, task, team_id};

/// The names this file proves, each by its cause.
const PROVED: [LogEvent; 17] = [
    LogEvent::DaemonTaskFailed,
    LogEvent::TeamNotStarted,
    LogEvent::NotificationFailed,
    LogEvent::HistoryRecordIncomplete,
    LogEvent::DaemonStarted,
    LogEvent::DaemonStopRequested,
    LogEvent::DaemonStopped,
    LogEvent::RequestDelegated,
    LogEvent::QuestionsAnswered,
    LogEvent::ControlUsed,
    LogEvent::ChecksDecided,
    LogEvent::ReviewsRequested,
    LogEvent::ReviewReportLate,
    LogEvent::NotificationSent,
    LogEvent::QuestionsFileOpened,
    LogEvent::PeerGone,
    LogEvent::BuildDirectoryRemoved,
];

/// The variable that makes [`signalled_daemon_child_process`] run `agent daemon` in the home it names.
const SIGNALLED_HOME: &str = "AGENT_TEST_SIGNALLED_DAEMON_HOME";

/// The variable of `daemon_log_tests::spawned_daemon_panic_child_process`: the history file its panicking side task
/// is logged into.
const SPAWNED_PANIC_DB: &str = "AGENT_TEST_SPAWNED_PANIC_DB";

/// Reviewers of which the first to run fails and every later run approves.
const FAILS_FIRST: &str = "fail-first:VERDICT: APPROVE\\nSUMMARY: fine";

/// Reviewers that never give a verdict.
const SILENT: &str = "sleep";

/// What a script of the stand-in CLI starts with: commits without signing.
const PRELUDE: &str = "set -eu\ncommit() { git -c user.name=Fake -c user.email=fake@example.com -c commit.gpgsign=false commit -q \"$@\"; }\n";

/// A task list with no items.
const PLAIN: &str = "Fix the typo in the README.\n";

/// A reporter that says nothing: these tests read the rows.
#[derive(Debug)]
struct SilentReporter;

impl Reporter for SilentReporter {
    fn report(&self, _problem: &Problem) {}
}

/// The rows of `daemon_logs` in the history file `path`.
fn logs_in(path: &Path) -> Vec<LogEntry> {
    Reader::open(path).unwrap().daemon_logs().unwrap()
}

/// The first row of `event` in `logs` that `matches`; the test fails with every row when there is none.
fn row(logs: &[LogEntry], event: LogEvent, matches: impl Fn(&LogEntry) -> bool) -> &LogEntry {
    logs.iter()
        .find(|entry| entry.event() == event && matches(entry))
        .unwrap_or_else(|| panic!("no {event} row as expected in {logs:#?}"))
}

/// The text of the field `name` of `entry`.
fn field<'a>(entry: &'a LogEntry, name: &str) -> &'a str {
    entry
        .fields()
        .get(name)
        .and_then(serde_json::Value::as_str)
        .unwrap_or_else(|| panic!("{} has no text field {name}: {entry:?}", entry.event()))
}

/// Records `event` as proved, after checking that `entry` is a row of it at its level.
fn proved(observed: &mut BTreeSet<LogEvent>, entry: &LogEntry, event: LogEvent) {
    assert_eq!(entry.event(), event);
    assert_eq!(entry.level(), event.level(), "{entry:?}");
    observed.insert(event);
}

/// A control request for task 1.
fn control() -> ControlRequest {
    ControlRequest {
        target: Target::Task(task(1)),
    }
}

/// An absolute path from a path of a test.
fn abs(path: &Path) -> AbsolutePath {
    AbsolutePath::from_str(path.to_str().unwrap()).unwrap()
}

// ---- a daemon's life, with questions, an editor, a control and notifications -------------------------------------

/// A daemon that opens a questions file in VS Code, notifies the human, takes the answer, pauses the run that follows, and
/// is stopped: `daemon_started`, `questions_file_opened`, `notification_sent`, `questions_answered`, `control_used` and
/// `daemon_stopped`, the last record of the daemon.
async fn record_daemon_life(observed: &mut BTreeSet<LogEvent>) {
    let mut daemon = Daemon::new(&solo_state(), None);
    let servers = daemon.surroundings.runtime_dir.join("servers");
    Daemon::vscode_server(&servers);
    let _opened = daemon.vscode_window();
    daemon.configure(&format!(
        "[questions]\nopen = \"vscode\"\nvscode_server_dirs = [\"{}\"]",
        servers.display()
    ));
    let file = daemon.questions_file(1, 1);
    daemon.scenario(1, &format!("questions {}", file.display()));
    daemon.start().await;
    daemon.status_is(1, TaskStatus::Questions).await;
    daemon.notification_with("questions-1.md").await;
    daemon.scenario(1, "busy");
    daemon.ask(answer(1, 1, &[(1, "ok")])).await.unwrap();
    daemon.status_is(1, TaskStatus::Running).await;
    daemon.ask(Command::Pause(control())).await.unwrap();
    daemon.status_is(1, TaskStatus::Paused).await;
    daemon.stop().await.unwrap();

    let logs = logs_in(&daemon.home.history_file());
    let started = row(&logs, LogEvent::DaemonStarted, |_| true);
    assert_eq!(started.team(), None);
    assert_eq!(field(started, "home"), daemon.home.root().display().to_string());
    proved(observed, started, LogEvent::DaemonStarted);

    let opened = row(&logs, LogEvent::QuestionsFileOpened, |entry| {
        entry.fields().get("file").and_then(serde_json::Value::as_str) == Some(&*file.display().to_string())
    });
    assert_eq!(field(opened, "outcome"), "window");
    proved(observed, opened, LogEvent::QuestionsFileOpened);

    let sent = row(&logs, LogEvent::NotificationSent, |entry| {
        entry.task() == Some(&task(1))
    });
    assert!(
        Path::new(field(sent, "file"))
            .extension()
            .is_some_and(|extension| extension == "md"),
        "{sent:?}"
    );
    proved(observed, sent, LogEvent::NotificationSent);

    let used = row(&logs, LogEvent::ControlUsed, |entry| {
        entry.fields().get("action").and_then(serde_json::Value::as_str) == Some("Pause")
    });
    assert_eq!(used.fields().get("tasks"), Some(&serde_json::json!(1)), "{used:?}");
    proved(observed, used, LogEvent::ControlUsed);

    let answered = row(&logs, LogEvent::QuestionsAnswered, |entry| {
        entry.task() == Some(&task(1))
    });
    assert_eq!(field(answered, "channel"), "agent answer");
    assert_eq!(field(answered, "number"), "1");
    proved(observed, answered, LogEvent::QuestionsAnswered);

    let stopped = logs.last().unwrap();
    assert_eq!(stopped.event(), LogEvent::DaemonStopped, "the last record: {logs:#?}");
    assert_eq!(field(stopped, "outcome"), "requested");
    assert_eq!(stopped.team(), None);
    proved(observed, stopped, LogEvent::DaemonStopped);
    assert!(
        logs.iter().position(|entry| entry.event() == LogEvent::DaemonStarted)
            < logs
                .iter()
                .position(|entry| entry.event() == LogEvent::TaskStatusChanged),
        "the daemon is started before its tasks change: {logs:#?}"
    );
}

#[tokio::test]
async fn a_daemon_records_its_start_the_editor_the_notification_the_answer_a_control_and_its_stop() {
    let mut observed = BTreeSet::new();
    record_daemon_life(&mut observed).await;
    assert_eq!(
        observed,
        BTreeSet::from([
            LogEvent::DaemonStarted,
            LogEvent::QuestionsFileOpened,
            LogEvent::NotificationSent,
            LogEvent::ControlUsed,
            LogEvent::QuestionsAnswered,
            LogEvent::DaemonStopped,
        ])
    );
}

// ---- what goes wrong in a running daemon --------------------------------------------------------------------------

/// A daemon whose notifications cannot be written, and a run that reports questions without their file: the notification
/// that tells the human is `notification_failed`, not `state_save_failed`.
async fn record_lost_notification(observed: &mut BTreeSet<LogEvent>) {
    let mut daemon = Daemon::new(&solo_state(), None);
    // A file where the directory of the notifications would be: no notification can be written.
    std::fs::write(daemon.home.notifications_dir(), "not a directory\n").unwrap();
    daemon.scenario(1, "questions-nofile");
    daemon.start().await;
    daemon
        .state_where("the run without its questions file ended", |state| {
            state
                .find_task(&task(1))
                .is_some_and(|found| found.member.runs().len() == 1 && found.member.status() != TaskStatus::Running)
        })
        .await;
    daemon.stop().await.unwrap();

    let logs = logs_in(&daemon.home.history_file());
    let failed = row(&logs, LogEvent::NotificationFailed, |entry| {
        entry.task() == Some(&task(1))
    });
    assert!(failed.fields().contains_key("error"), "{failed:?}");
    proved(observed, failed, LogEvent::NotificationFailed);
    assert!(
        !logs
            .iter()
            .any(|entry| entry.event() == LogEvent::StateSaveFailed && entry.message().contains("notification")),
        "a notification is not the state: {logs:#?}"
    );
}

#[tokio::test]
async fn a_notification_that_cannot_be_written_is_logged_by_name() {
    let mut observed = BTreeSet::new();
    record_lost_notification(&mut observed).await;
    assert_eq!(observed, BTreeSet::from([LogEvent::NotificationFailed]));
}

#[tokio::test]
async fn notification_logs_carry_the_delivery_team_and_round_for_success_and_failure() {
    let mut delivered = Rig::new(
        &[
            normal("claude-1", "questions"),
            investigator("inv-r1", SILENT),
            investigator("inv-r2", SILENT),
        ],
        5,
        false,
    )
    .await;
    delivered.start().await;
    let (delivered_team, delivered_tasks) = delivered.run(&["claude-1"], ONE, MergeMode::Autopilot, None).await;
    let delivered_state = delivered
        .state_where("the successful notification to be attempted", |state| {
            state
                .find_task(&delivered_tasks[0])
                .is_some_and(|found| found.member.runs().len() == 1 && found.member.status() != TaskStatus::Running)
        })
        .await;
    let delivered_round = delivered_state.teams()[0].rounds()[0].id().clone();
    delivered.stop().await.unwrap();

    let delivered_logs = logs_in(&delivered.home.history_file());
    let sent = row(&delivered_logs, LogEvent::NotificationSent, |entry| {
        entry.task() == Some(&delivered_tasks[0])
    });
    assert_eq!(sent.team(), Some(&delivered_team));
    assert_eq!(sent.rounds(), std::slice::from_ref(&delivered_round));

    let mut undelivered = Rig::new(
        &[
            normal("claude-1", "questions"),
            investigator("inv-r1", SILENT),
            investigator("inv-r2", SILENT),
        ],
        5,
        false,
    )
    .await;
    std::fs::write(undelivered.home.notifications_dir(), "not a directory\n").unwrap();
    undelivered.start().await;
    let (undelivered_team, undelivered_tasks) = undelivered.run(&["claude-1"], ONE, MergeMode::Autopilot, None).await;
    let undelivered_state = undelivered
        .state_where("the failed notification to be attempted", |state| {
            state
                .find_task(&undelivered_tasks[0])
                .is_some_and(|found| found.member.runs().len() == 1 && found.member.status() != TaskStatus::Running)
        })
        .await;
    let undelivered_round = undelivered_state.teams()[0].rounds()[0].id().clone();
    undelivered.stop().await.unwrap();

    let undelivered_logs = logs_in(&undelivered.home.history_file());
    let failed = row(&undelivered_logs, LogEvent::NotificationFailed, |entry| {
        entry.task() == Some(&undelivered_tasks[0])
    });
    assert_eq!(failed.team(), Some(&undelivered_team));
    assert_eq!(failed.rounds(), std::slice::from_ref(&undelivered_round));
}

/// A daemon whose state file becomes a directory while a run works: the owner fails on its next save, which is
/// `daemon_task_failed` with the cause, and the daemon stops with `daemon_stopped` and the outcome `failed`.
async fn record_failed_owner(observed: &mut BTreeSet<LogEvent>) {
    let mut daemon = Daemon::new(&solo_state(), None);
    daemon.scenario(1, "busy");
    daemon.start().await;
    daemon.status_is(1, TaskStatus::Running).await;
    let state = daemon.home.state_file();
    std::fs::remove_file(&state).unwrap();
    std::fs::create_dir(&state).unwrap();
    // The pause is saved: the save fails and ends the owner, and with it the daemon.
    drop(daemon.ask(Command::Pause(control())).await);
    daemon.stop().await.unwrap_err();

    let logs = logs_in(&daemon.home.history_file());
    let owner = row(&logs, LogEvent::DaemonTaskFailed, |entry| {
        entry.module() == "agent::daemon"
    });
    assert!(!field(owner, "error").is_empty(), "{owner:?}");
    proved(observed, owner, LogEvent::DaemonTaskFailed);
    let stopped = logs.last().unwrap();
    assert_eq!(stopped.event(), LogEvent::DaemonStopped, "the last record: {logs:#?}");
    assert_eq!(field(stopped, "outcome"), "failed");
    proved(observed, stopped, LogEvent::DaemonStopped);
}

#[tokio::test]
async fn an_owner_that_fails_is_logged_with_its_cause_and_the_daemon_stops_with_the_outcome_failed() {
    let mut observed = BTreeSet::new();
    record_failed_owner(&mut observed).await;
    assert_eq!(
        observed,
        BTreeSet::from([LogEvent::DaemonTaskFailed, LogEvent::DaemonStopped])
    );
}

// ---- a bare owner: a client that went away, a team the daemon cannot start ------------------------------------------

/// An owner answers a request whose client went away (`peer_gone`), then, with a clock that gives no time that can be
/// recorded, cannot form the team of a request (`team_not_started`, and the client is told it is internal) and cannot
/// time an event of the history (`history_record_incomplete`).
async fn record_gone_client_and_unstarted_team(observed: &mut BTreeSet<LogEvent>) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("history.db");
    let started = History::start(
        HistoryDb::open(&path, MaxTextBytes::default()).unwrap(),
        Setup {
            reporter: Arc::new(SilentReporter),
            capacity: DEFAULT_QUEUE,
        },
    )
    .unwrap();
    // The layer keeps its own clock: the owner's is broken on purpose below.
    let layer_clock = Daemon::new(&State::new(), None);
    let layer = HistoryLayer::new(
        LogLevel::Debug,
        SharedClock::new(layer_clock.clock.clone()),
        Secrets::default(),
        Some(started.history.clone()),
    );
    let dispatch: tracing::Dispatch = tracing_subscriber::registry().with(layer).into();
    let daemon = Daemon::new(&State::new(), None);
    // Two investigators outside the team: the plan of a solo task is not refused, and forming it reads the clock.
    daemon.configure("[investigators.inv-2]\ncli = \"claude\"");
    let (owner, _sinks) = daemon.bare_owner().await;
    let mut owner = owner.with_history(started.history.clone());
    let checkout = tempfile::tempdir().unwrap();

    let refused = async {
        let (reply, answer) = tokio::sync::oneshot::channel();
        drop(answer);
        owner
            .handle(Message::Request {
                client: Client::Cli,
                command: Command::Status(StatusRequest { target: None }),
                reply,
            })
            .await
            .unwrap();

        // A year before the year 0: no timestamp of the history can hold it.
        daemon.clock.advance(-time::Duration::days(366 * 2100));
        let (reply, answer) = tokio::sync::oneshot::channel();
        owner
            .handle(Message::Request {
                client: Client::Cli,
                command: Command::Run(RunRequest {
                    repo: "team-agents".parse().unwrap(),
                    checkout: abs(checkout.path()),
                    agents: AgentList::try_from(vec!["claude-1".parse::<AgentName>().unwrap()]).unwrap(),
                    task_list: TaskListText::new(PLAIN.to_owned()).unwrap(),
                    coordinator: None,
                    merge: MergeMode::Autopilot,
                }),
                reply,
            })
            .await
            .unwrap();
        let refused = answer.await.unwrap().unwrap_err();
        owner.record_event(
            &team_id(0),
            Vec::new(),
            HistoryEvent::Notified {
                task: None,
                title: "a title".to_owned(),
                body: "a body".to_owned(),
            },
        );
        refused
    }
    .with_subscriber(dispatch)
    .await;
    started.writer.close().unwrap();

    assert_eq!(refused.kind, FailureKind::Internal);
    let logs = logs_in(&path);
    let gone = row(&logs, LogEvent::PeerGone, |entry| {
        entry.module() == "agent::daemon::owner"
    });
    proved(observed, gone, LogEvent::PeerGone);
    let unstarted = row(&logs, LogEvent::TeamNotStarted, |_| true);
    assert!(!field(unstarted, "error").is_empty(), "{unstarted:?}");
    proved(observed, unstarted, LogEvent::TeamNotStarted);
    let incomplete = row(&logs, LogEvent::HistoryRecordIncomplete, |entry| {
        entry.team() == Some(&team_id(0))
    });
    assert!(!field(incomplete, "error").is_empty(), "{incomplete:?}");
    proved(observed, incomplete, LogEvent::HistoryRecordIncomplete);
}

#[tokio::test]
async fn a_client_that_went_away_a_team_the_daemon_cannot_form_and_an_untimed_record_are_logged_by_name() {
    let mut observed = BTreeSet::new();
    record_gone_client_and_unstarted_team(&mut observed).await;
    assert_eq!(
        observed,
        BTreeSet::from([
            LogEvent::PeerGone,
            LogEvent::TeamNotStarted,
            LogEvent::HistoryRecordIncomplete,
        ])
    );
}

// ---- child processes: a signal, a panicking side task --------------------------------------------------------------

/// Runs `agent daemon` in the home `SIGNALLED_HOME` names, as the command does, until it is signalled.
#[tokio::test]
async fn signalled_daemon_child_process() {
    let Some(home) = std::env::var_os(SIGNALLED_HOME) else {
        return;
    };
    let environment = crate::environment::Environment::new(Some(home));
    let prompts = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../prompts");
    crate::commands::daemon::daemon(&environment, Some(prompts))
        .await
        .unwrap();
}

/// `agent daemon` in a child process is sent `SIGTERM`: `daemon_started`, `daemon_stop_requested` with the signal, and
/// `daemon_stopped` last.
async fn record_signalled_daemon(observed: &mut BTreeSet<LogEvent>) {
    let daemon = Daemon::new(&State::new(), None);
    let child = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "daemon::owner::rounds::daemon_log_cause_tests::signalled_daemon_child_process",
            "--nocapture",
        ])
        .env(SIGNALLED_HOME, daemon.home.root())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let deadline = std::time::Instant::now() + PATIENCE;
    while !daemon.home.socket_file().exists() {
        assert!(std::time::Instant::now() < deadline, "the child daemon has no socket");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let pid = rustix::process::Pid::from_child(&child);
    rustix::process::kill_process(pid, rustix::process::Signal::TERM).unwrap();
    let output = tokio::task::spawn_blocking(move || child.wait_with_output())
        .await
        .unwrap()
        .unwrap();
    assert!(
        output.status.success(),
        "the child daemon failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let logs = logs_in(&daemon.home.history_file());
    let started = row(&logs, LogEvent::DaemonStarted, |_| true);
    proved(observed, started, LogEvent::DaemonStarted);
    let requested = row(&logs, LogEvent::DaemonStopRequested, |_| true);
    assert_eq!(field(requested, "signal"), "SIGTERM");
    assert_eq!(requested.team(), None);
    proved(observed, requested, LogEvent::DaemonStopRequested);
    let stopped = logs.last().unwrap();
    assert_eq!(stopped.event(), LogEvent::DaemonStopped, "the last record: {logs:#?}");
    assert_eq!(field(stopped, "outcome"), "requested");
    proved(observed, stopped, LogEvent::DaemonStopped);
}

#[tokio::test]
async fn sigterm_to_agent_daemon_is_recorded_between_its_start_and_its_stop() {
    let mut observed = BTreeSet::new();
    record_signalled_daemon(&mut observed).await;
    assert_eq!(
        observed,
        BTreeSet::from([
            LogEvent::DaemonStarted,
            LogEvent::DaemonStopRequested,
            LogEvent::DaemonStopped,
        ])
    );
}

/// A side task of the daemon panics in a child process: the panic is `daemon_panicked`, and the task that failed is
/// `daemon_task_failed` with the cause, where the daemon joins it.
async fn record_failed_side_task(observed: &mut BTreeSet<LogEvent>) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("history.db");
    let child_path = path.clone();
    let output = tokio::task::spawn_blocking(move || {
        std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "daemon::owner::rounds::daemon_log_tests::spawned_daemon_panic_child_process",
                "--nocapture",
            ])
            .env(SPAWNED_PANIC_DB, &child_path)
            .output()
    })
    .await
    .unwrap()
    .unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));

    let logs = logs_in(&path);
    let failed = row(&logs, LogEvent::DaemonTaskFailed, |entry| {
        entry.module() == "agent::daemon"
    });
    assert!(field(failed, "error").contains("panicked"), "{failed:?}");
    assert_eq!(
        logs.iter()
            .filter(|entry| entry.event() == LogEvent::DaemonPanicked)
            .count(),
        1,
        "the panic itself is one row: {logs:#?}"
    );
    proved(observed, failed, LogEvent::DaemonTaskFailed);
}

#[tokio::test]
async fn a_side_task_that_panics_is_a_failed_task_of_the_daemon_besides_the_panic() {
    let mut observed = BTreeSet::new();
    record_failed_side_task(&mut observed).await;
    assert_eq!(observed, BTreeSet::from([LogEvent::DaemonTaskFailed]));
}

// ---- rounds: the checks, the reviews, a late report, a worker ------------------------------------------------------

/// Writes the script `name` of the stand-in CLI of `rig`.
fn script(rig: &Rig, name: &str, body: &str) {
    let dir = rig.home.root().join("fake-scripts");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join(format!("{name}.sh")), format!("{PRELUDE}{body}")).unwrap();
}

/// A round whose first head changes code without a test: the checks send it back (`checks_decided`), and the fixed head
/// has its reviews asked for (`reviews_requested`).
async fn record_checks_and_reviews(observed: &mut BTreeSet<LogEvent>) {
    let mut rig = Rig::new(
        &[
            normal("claude-1", "script:code-only,script:add-test"),
            investigator("inv-r1", SILENT),
            investigator("inv-r2", SILENT),
        ],
        5,
        false,
    )
    .await;
    let text = std::fs::read_to_string(rig.home.config_file()).unwrap();
    std::fs::write(
        rig.home.config_file(),
        format!(
            "{}[repos.example.checks]\ntest_paths = [\"tests/\"]\ncode_paths = [\"src/\"]\nprotected_paths = [\"ci/\"]\n",
            text.replace("fix_passes = 5", "fix_passes = 5\ncheck_passes = 2")
        ),
    )
    .unwrap();
    for (name, body) in [("tests/existing.txt", "a\n"), ("src/code.txt", "code\n")] {
        let file = rig.repo.join(name);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(file, body).unwrap();
    }
    git(&rig.repo, &["add", "-A"]);
    git(&rig.repo, &["commit", "-q", "-m", "Seed the tests and the code"]);
    git(&rig.repo, &["push", "-q", "origin", "main"]);
    script(
        &rig,
        "code-only",
        "echo 'fn a() {}' > src/a.txt\ngit add src/a.txt\ncommit -m 'Add code'\n",
    );
    script(
        &rig,
        "add-test",
        "echo case > tests/new.txt\ngit add tests/new.txt\ncommit -m 'Add a test'\n",
    );
    rig.start().await;
    let (team, _tasks) = rig.run(&["claude-1"], ONE, MergeMode::Autopilot, None).await;
    rig.item_where(
        "1.1",
        "asked for reviews",
        |phase| matches!(phase, ItemPhase::InReview { reviews, .. } if reviews.len() == 2),
    )
    .await;
    rig.stop().await.unwrap();

    let logs = logs_in(&rig.home.history_file());
    let decided = row(&logs, LogEvent::ChecksDecided, |entry| entry.team() == Some(&team));
    assert_eq!(field(decided, "outcome"), "sent_back");
    assert_eq!(decided.fields().get("pr"), Some(&serde_json::json!("1")), "{decided:?}");
    proved(observed, decided, LogEvent::ChecksDecided);
    let asked = row(&logs, LogEvent::ReviewsRequested, |entry| entry.team() == Some(&team));
    assert_eq!(
        asked.fields().get("reviewers"),
        Some(&serde_json::json!(2)),
        "{asked:?}"
    );
    proved(observed, asked, LogEvent::ReviewsRequested);
}

#[tokio::test]
async fn checks_that_send_a_round_back_and_the_reviews_asked_afterwards_are_logged_by_name() {
    let mut observed = BTreeSet::new();
    record_checks_and_reviews(&mut observed).await;
    assert_eq!(
        observed,
        BTreeSet::from([LogEvent::ChecksDecided, LogEvent::ReviewsRequested])
    );
}

/// A reviewer that failed and was replaced, and that the human resumes afterwards: it reports with no review asked of
/// it any more, `review_report_late`, of its task.
async fn record_late_report(observed: &mut BTreeSet<LogEvent>) {
    let mut rig = Rig::new(
        &[
            normal("claude-1", "work"),
            investigator("inv-r1", FAILS_FIRST),
            investigator("inv-r2", FAILS_FIRST),
            investigator("inv-r3", FAILS_FIRST),
        ],
        5,
        false,
    )
    .await;
    rig.start().await;
    // The pull request is not merged, so the team goes on and the task of the reviewer that failed can be resumed.
    let (team, _tasks) = rig.run(&["claude-1"], ONE, MergeMode::NoMerge, None).await;
    let state = rig
        .item_where("1.1", "approved", |phase| matches!(phase, ItemPhase::Approved { .. }))
        .await;
    let reviewers = state.team(&team).unwrap().reviewers();
    let replaced: TaskId = reviewers
        .iter()
        .find(|member| member.status() == TaskStatus::Failed)
        .unwrap_or_else(|| panic!("no reviewer failed: {reviewers:?}"))
        .task()
        .clone();
    let mut connection = crate::client::Connection::connect(&rig.home.socket_file())
        .await
        .unwrap();
    connection
        .request(
            Client::Cli,
            Command::Resume(ControlRequest {
                target: Target::Task(replaced.clone()),
            }),
        )
        .await
        .unwrap();
    rig.status_is(&replaced, TaskStatus::Waiting).await;
    rig.stop().await.unwrap();

    let logs = logs_in(&rig.home.history_file());
    let late = row(&logs, LogEvent::ReviewReportLate, |entry| {
        entry.task() == Some(&replaced)
    });
    assert_eq!(late.team(), Some(&team));
    proved(observed, late, LogEvent::ReviewReportLate);
}

#[tokio::test]
async fn a_report_of_a_reviewer_with_no_review_asked_of_it_is_logged_as_late() {
    let mut observed = BTreeSet::new();
    record_late_report(&mut observed).await;
    assert_eq!(observed, BTreeSet::from([LogEvent::ReviewReportLate]));
}

/// A team of a coordinator, a member and a worker; the coordinator asks for the worker on the bus: `request_delegated`.
async fn record_delegated_request(observed: &mut BTreeSet<LogEvent>) {
    let mut rig = Rig::new(
        &[
            normal("claude-1", "sleep"),
            normal("claude-2", "sleep"),
            investigator("inv-r1", SILENT),
            investigator("inv-r2", SILENT),
        ],
        5,
        true,
    )
    .await;
    let bus = crate::daemon::owner::fixture::agentbus_binary();
    let text = std::fs::read_to_string(rig.home.config_file()).unwrap();
    let worker = format!(
        "[agents.glm-1]\ncli = \"claude\"\nargs = [\"--scenario\", \"sleep,done:Result: three files.\", \"--as\", \"glm-1\", \
         \"--home\", \"{home}\", \"--bus-bin\", \"{bus}\", \"--bus-dir\", \"{home}/bus\"]\n",
        home = rig.home.root().display(),
        bus = bus.display(),
    );
    std::fs::write(
        rig.home.config_file(),
        format!(
            "{worker}{}",
            text.replace("[tiers]\n", "[tiers]\nworkers = [\"glm-1\"]\n")
        ),
    )
    .unwrap();
    rig.start().await;
    let (team, tasks) = rig
        .run(&["claude-1", "claude-2", "glm-1"], PLAIN, MergeMode::Autopilot, None)
        .await;
    for task in &tasks {
        rig.status_is(task, TaskStatus::Waiting).await;
    }
    AgentBus::new(
        Program::from_str(bus.to_str().unwrap()).unwrap(),
        rig.home.absolute_root().clone(),
    )
    .delegate(&Delegation {
        team: team.clone(),
        requester: "claude-1".parse().unwrap(),
        task: tasks[0].clone(),
        repo: "example".parse().unwrap(),
        branch: format!("agents/claude-1/{}", tasks[0]).parse().unwrap(),
        worktree: abs(&rig.worktree(&tasks[0])),
        dispatcher: Program::from_str("true").unwrap(),
        request: "List the files under src/.".to_owned(),
        context: "None.".to_owned(),
    })
    .await
    .unwrap();
    rig.prompt(&tasks[2], 2).await;
    rig.stop().await.unwrap();

    let logs = logs_in(&rig.home.history_file());
    let delegated = row(&logs, LogEvent::RequestDelegated, |entry| entry.team() == Some(&team));
    assert_eq!(field(delegated, "worker"), "glm-1");
    proved(observed, delegated, LogEvent::RequestDelegated);
}

#[tokio::test]
async fn a_request_given_to_a_worker_is_logged_with_the_worker() {
    let mut observed = BTreeSet::new();
    record_delegated_request(&mut observed).await;
    assert_eq!(observed, BTreeSet::from([LogEvent::RequestDelegated]));
}

// ---- every name of this file --------------------------------------------------------------------------------------

async fn record_build_directory_removed(observed: &mut BTreeSet<LogEvent>) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("history.db");
    let db = HistoryDb::open(&path, MaxTextBytes::default()).unwrap();
    let started = History::start(
        db,
        Setup {
            reporter: Arc::new(SilentReporter),
            capacity: DEFAULT_QUEUE,
        },
    )
    .unwrap();
    let daemon = Daemon::new(&solo_state(), None);
    let layer = HistoryLayer::new(
        LogLevel::Debug,
        SharedClock::new(daemon.clock.clone()),
        Secrets::default(),
        Some(started.history.clone()),
    );
    let dispatch: tracing::Dispatch = tracing_subscriber::registry().with(layer).into();

    let worktree_dir = daemon.home.worktree_dir(&crate::fixtures::repo(), &task(1));
    let target = worktree_dir.join("target");
    std::fs::create_dir_all(&target).unwrap();
    std::fs::write(target.join("output.bin"), b"build output").unwrap();

    let (owner, _sinks) = daemon.bare_owner().await;
    let owner = owner.with_history(started.history.clone());
    async {
        owner.remove_target(&task(1), &worktree_dir);
    }
    .with_subscriber(dispatch)
    .await;

    started.writer.close().unwrap();
    let logs = logs_in(&path);
    let removed = row(&logs, LogEvent::BuildDirectoryRemoved, |_| true);
    proved(observed, removed, LogEvent::BuildDirectoryRemoved);
}

/// Provokes the cause of every name in [`PROVED`] and adds each name whose row was found to `observed`.
pub(super) async fn record_named_daemon_causes(observed: &mut BTreeSet<LogEvent>) {
    record_daemon_life(observed).await;
    record_lost_notification(observed).await;
    record_failed_owner(observed).await;
    record_gone_client_and_unstarted_team(observed).await;
    record_signalled_daemon(observed).await;
    record_failed_side_task(observed).await;
    record_checks_and_reviews(observed).await;
    record_late_report(observed).await;
    record_delegated_request(observed).await;
    record_build_directory_removed(observed).await;
}

#[tokio::test]
async fn every_name_of_this_file_is_produced_by_its_cause_with_its_level() {
    let mut observed = BTreeSet::new();
    record_named_daemon_causes(&mut observed).await;
    assert_eq!(observed, BTreeSet::from(PROVED));
}
