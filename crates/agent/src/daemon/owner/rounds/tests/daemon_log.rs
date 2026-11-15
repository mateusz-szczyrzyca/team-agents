//! Integration tests for the daemon's own log (`daemon_logs` and `daemon_log_rounds` in `history.db`, `HistoryLayer`,
//! span scopes, identical-warning coalescing, idle-tick silence, `stderr` fallback when `history.db` is unwritable, and
//! coverage of every [`LogEvent`] variant).

use std::collections::BTreeSet;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use agent_core::config::Secrets;
use agent_core::history::MaxTextBytes;
use agent_core::ids::{RoundId, RunId, TaskId, TeamId};
use agent_core::log::{LogEvent, LogLevel};
use agent_core::protocol::{Client, Command, Reply, StatusRequest, Target};
use agent_core::state::{Completion, ItemPhase, MergeMode};
use agent_runtime::clock::SharedClock;
use agent_runtime::history::{
    DEFAULT_QUEUE, History, HistoryDb, HistoryLayer, Problem, Reader, Reporter, Setup, install_panic_hook,
    log_decision, log_outside_call,
};
use agent_runtime::store::StoreError;
use serde_json::json;
use tracing::Instrument;
use tracing::instrument::WithSubscriber;
use tracing_subscriber::Layer;
use tracing_subscriber::layer::{Context, SubscriberExt};
use tracing_subscriber::registry::LookupSpan;

use super::rig::{ONE, Rig, investigator, normal};
use crate::daemon::owner::fixture::{Daemon, pair_state};
use crate::daemon::owner::{Message, OwnerError};
use crate::fixtures::{at, solo_state, task, team_id};

/// Silent reporter for tests that inspect `daemon_logs` or `stderr` directly.
#[derive(Debug)]
struct SilentReporter;

impl Reporter for SilentReporter {
    fn report(&self, _problem: &Problem) {}
}

/// Records the span names around owner events for request instrumentation tests.
#[derive(Clone, Default)]
struct OwnerScopes(Arc<Mutex<Vec<Vec<String>>>>);

impl<S> Layer<S> for OwnerScopes
where
    S: tracing::Subscriber + for<'lookup> LookupSpan<'lookup>,
{
    fn on_event(&self, event: &tracing::Event<'_>, context: Context<'_, S>) {
        if event.metadata().target() != "agent::daemon::owner" {
            return;
        }
        let spans = context
            .event_scope(event)
            .into_iter()
            .flatten()
            .map(|span| span.name().to_owned())
            .collect();
        self.0.lock().unwrap().push(spans);
    }
}

#[tokio::test]
async fn malformed_cli_line_leaves_cli_output_unreadable_warn_row_and_debug_assignment_logs_decision_taken() {
    let mut rig = Rig::new(
        &[
            normal("claude-1", "script:badline"),
            investigator("inv-r1", "done:VERDICT: APPROVE\\nSUMMARY: fine"),
            investigator("inv-r2", "done:VERDICT: APPROVE\\nSUMMARY: fine"),
        ],
        5,
        false,
    )
    .await;

    let mut cfg = std::fs::read_to_string(rig.home.config_file()).unwrap();
    cfg.push_str("\n[log]\nlevel = \"debug\"\n");
    std::fs::write(rig.home.config_file(), cfg).unwrap();

    let scripts = rig.home.root().join("fake-scripts");
    std::fs::create_dir_all(&scripts).unwrap();
    std::fs::write(
        scripts.join("badline.sh"),
        r"set -eu
printf 'not-valid-json-line-from-cli\n'
printf 'implemented feature\n' > work.txt
git add work.txt
git -c user.name=Fake -c user.email=fake@example.com -c commit.gpgsign=false commit -qm 'Implement item 1.1'
",
    )
    .unwrap();

    rig.start().await;
    rig.run(&["claude-1"], ONE, MergeMode::Autopilot, None).await;
    let state = rig
        .item_where("1.1", "merged", |phase| matches!(phase, ItemPhase::Done { .. }))
        .await;
    assert!(matches!(
        state.teams()[0].item("1.1".parse().unwrap()).unwrap().phase(),
        ItemPhase::Done {
            completion: Completion::Merged { .. }
        }
    ));
    rig.stop().await.unwrap();

    let team = state.teams()[0].id().clone();
    let round = state.teams()[0].rounds()[0].id().clone();
    let author_member = &state.teams()[0].crew().roster()[0].1;
    let author_task = author_member.task().clone();
    let author_run = author_member.runs()[0].id().clone();

    let reader = Reader::open(&rig.home.history_file()).unwrap();
    let logs = reader.daemon_logs().unwrap();

    // A1 part 1: malformed CLI line leaves a `warn` row (`cli_output_unreadable`) with team, round, task, and run.
    let unreadable = logs
        .iter()
        .find(|row| row.event() == LogEvent::CliOutputUnreadable)
        .expect("expected cli_output_unreadable row in daemon_logs");
    assert_eq!(unreadable.level(), LogLevel::Warn);
    assert_eq!(unreadable.team(), Some(&team));
    assert_eq!(unreadable.rounds(), std::slice::from_ref(&round));
    assert_eq!(unreadable.task(), Some(&author_task));
    assert_eq!(unreadable.run(), Some(&author_run));

    // A1 part 2: with `[log] level = "debug"`, item assignment is a `decision_taken` row with its input and result.
    let assignment = logs
        .iter()
        .find(|row| {
            row.event() == LogEvent::DecisionTaken
                && row.fields().get("decision").and_then(|v| v.as_str()) == Some("assignable")
        })
        .expect("expected decision_taken row for assignable in daemon_logs");
    assert_eq!(assignment.level(), LogLevel::Debug);
    assert_eq!(assignment.team(), Some(&team));
    assert!(assignment.fields().get("input").is_some(), "{assignment:?}");
    let result_text = assignment.fields()["result"].to_string();
    assert!(result_text.contains("1.1"), "{result_text}");
    assert!(result_text.contains("claude-1"), "{result_text}");

    // Outside calls (`git` and `github`) are also recorded at `debug`.
    assert!(
        logs.iter().any(|row| {
            row.event() == LogEvent::OutsideCallCompleted
                && row.fields().get("tool").and_then(|v| v.as_str()) == Some("git")
        }),
        "expected git outside_call_completed row"
    );
    assert!(
        logs.iter().any(|row| {
            row.event() == LogEvent::OutsideCallCompleted
                && row.fields().get("tool").and_then(|v| v.as_str()) == Some("github")
        }),
        "expected github outside_call_completed row"
    );
}

#[tokio::test]
async fn team_start_records_the_initial_round_and_item_lifecycle() {
    let mut rig = Rig::new(
        &[
            normal("claude-1", "work"),
            investigator("inv-r1", "done:VERDICT: APPROVE\\nSUMMARY: fine"),
            investigator("inv-r2", "done:VERDICT: APPROVE\\nSUMMARY: fine"),
        ],
        5,
        false,
    )
    .await;
    rig.start().await;
    let (team, tasks) = rig.run(&["claude-1"], ONE, MergeMode::Autopilot, None).await;
    let state = rig.state();
    let initial_round = state.teams()[0].rounds()[0].id().clone();
    let author_task = tasks[0].clone();
    rig.stop().await.unwrap();

    let logs = Reader::open(&rig.home.history_file()).unwrap().daemon_logs().unwrap();
    let opening = logs.iter().find(|row| {
        row.event() == LogEvent::RoundChanged
            && row.team() == Some(&team)
            && row.rounds().contains(&initial_round)
            && row.fields().get("change").and_then(|value| value.as_str()) == Some("opened")
    });
    let in_progress = logs.iter().find(|row| {
        row.event() == LogEvent::ItemPhaseChanged
            && row.team() == Some(&team)
            && row.rounds().contains(&initial_round)
            && row.task() == Some(&author_task)
            && row.fields().get("item").and_then(|value| value.as_str()) == Some("1.1")
            && row.fields().get("phase").and_then(|value| value.as_str()) == Some("in_progress")
    });
    assert!(opening.is_some(), "initial round opening is missing: {logs:?}");
    assert!(in_progress.is_some(), "initial item phase is missing: {logs:?}");
}

#[tokio::test]
async fn initial_external_calls_carry_the_team_and_workspace_task() {
    let mut rig = Rig::new(
        &[
            normal("claude-1", "work"),
            normal("claude-2", "work"),
            investigator("inv-r1", "done:VERDICT: APPROVE\\nSUMMARY: fine"),
            investigator("inv-r2", "done:VERDICT: APPROVE\\nSUMMARY: fine"),
        ],
        5,
        true,
    )
    .await;
    let mut config = std::fs::read_to_string(rig.home.config_file()).unwrap();
    config.push_str("\n[log]\nlevel = \"debug\"\n");
    std::fs::write(rig.home.config_file(), config).unwrap();

    rig.start().await;
    let (team, tasks) = rig
        .run(&["claude-1", "claude-2"], ONE, MergeMode::Autopilot, None)
        .await;
    rig.stop().await.unwrap();

    let logs = Reader::open(&rig.home.history_file()).unwrap().daemon_logs().unwrap();
    let calls: Vec<_> = logs
        .iter()
        .filter(|row| row.event() == LogEvent::OutsideCallCompleted)
        .collect();
    for task in tasks {
        assert!(
            calls.iter().any(|row| {
                row.team() == Some(&team)
                    && row.task() == Some(&task)
                    && row.run().is_none()
                    && row.fields().get("tool").and_then(|value| value.as_str()) == Some("git")
                    && row.fields().get("operation").and_then(|value| value.as_str()) == Some("worktree add")
            }),
            "initial worktree add for {task} lacks team/task scope: {calls:?}"
        );
    }
    assert!(
        calls.iter().any(|row| {
            row.team() == Some(&team)
                && row.task().is_none()
                && row.run().is_none()
                && row.fields().get("tool").and_then(|value| value.as_str()) == Some("git")
                && row.fields().get("operation").and_then(|value| value.as_str()) == Some("rev-parse")
        }),
        "initial base-revision lookup lacks team-only scope: {calls:?}"
    );
    assert!(
        calls.iter().any(|row| {
            row.team() == Some(&team)
                && row.task().is_none()
                && row.run().is_none()
                && row.fields().get("tool").and_then(|value| value.as_str()) == Some("agentbus")
                && row.fields().get("operation").and_then(|value| value.as_str()) == Some("team-create")
        }),
        "initial team-create lacks team-only scope: {calls:?}"
    );
}

#[tokio::test]
async fn owner_refusals_are_emitted_inside_a_request_span() {
    let state = solo_state();
    let daemon = Daemon::new(&state, None);
    let (mut owner, _sinks) = daemon.bare_owner().await;
    let scopes = OwnerScopes::default();
    let dispatch = tracing::Dispatch::new(tracing_subscriber::registry().with(scopes.clone()));
    let (reply, answer) = tokio::sync::oneshot::channel();
    let message = Message::Request {
        client: Client::Cli,
        command: Command::Status(StatusRequest {
            target: Some(Target::Task("task-20261115-090000-none".parse().unwrap())),
        }),
        reply,
    };

    owner.handle(message).with_subscriber(dispatch).await.unwrap();
    answer.await.unwrap().unwrap_err();

    let events = scopes.0.lock().unwrap();
    assert!(
        events.iter().any(|spans| spans.contains(&"request".to_owned())),
        "owner refusal has no request span: {events:?}"
    );
}

#[tokio::test]
async fn one_hundred_idle_team_looks_write_zero_rows_at_info() {
    let state = solo_state();
    let daemon = Daemon::new(&state, None);
    let (mut owner, _sinks) = daemon.bare_owner().await;
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
    let layer = HistoryLayer::new(
        LogLevel::Info,
        SharedClock::new(daemon.clock.clone()),
        Secrets::default(),
        Some(started.history.clone()),
    );
    let dispatch: tracing::Dispatch = tracing_subscriber::registry().with(layer).into();

    async {
        tracing::info!(
            event = LogEvent::TeamChanged.as_str(),
            team = %team_id(0),
            change = "started",
            "a team is started"
        );
        for _ in 0..100 {
            daemon.clock.advance(time::Duration::seconds(1));
            owner.look_at(&team_id(0));
            owner.team_life().await.unwrap();
        }
    }
    .with_subscriber(dispatch)
    .await;

    started.writer.close().unwrap();
    let rows = Reader::open(&path).unwrap().daemon_logs().unwrap();
    assert_eq!(
        rows.len(),
        1,
        "expected only the initial team_changed row and zero rows across 100 idle looks at info, got {rows:?}"
    );
    assert_eq!(rows[0].event(), LogEvent::TeamChanged);
}

#[tokio::test]
async fn five_repeated_bus_warnings_coalesce_into_one_row_with_count_five() {
    let state = pair_state();
    let daemon = Daemon::new(&state, Some(Path::new("false")));
    let (mut owner, _sinks) = daemon.bare_owner().await;
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
    let layer = HistoryLayer::new(
        LogLevel::Warn,
        SharedClock::new(daemon.clock.clone()),
        Secrets::default(),
        Some(started.history.clone()),
    );
    let dispatch: tracing::Dispatch = tracing_subscriber::registry().with(layer).into();
    let round: RoundId = "round-20261115-090000-bus5".parse().unwrap();
    let run: RunId = "run-20261115-090000-bus5".parse().unwrap();
    let team = team_id(0);
    let task_id = task(1);

    async {
        let span = tracing::error_span!(
            "run",
            team = %team,
            rounds = %round,
            task = %task_id,
            run = %run,
        );
        async {
            daemon.clock.advance(time::Duration::seconds(10));
            for step in 0..5 {
                if step > 0 {
                    daemon.clock.advance(time::Duration::seconds(1));
                }
                owner.look_at(&team);
                owner.team_life().await.unwrap();
            }
        }
        .instrument(span)
        .await;
    }
    .with_subscriber(dispatch)
    .await;

    started.writer.close().unwrap();
    let rows = Reader::open(&path).unwrap().daemon_logs().unwrap();
    let bus_rows: Vec<_> = rows.iter().filter(|row| row.event() == LogEvent::BusDown).collect();
    assert_eq!(bus_rows.len(), 1, "expected 1 coalesced bus_down row, got {bus_rows:?}");
    assert_eq!(bus_rows[0].count().get(), 5);
    assert_eq!(bus_rows[0].first_at(), at(10));
    assert_eq!(bus_rows[0].at(), at(14));
    assert_eq!(bus_rows[0].team(), Some(&team));
    assert_eq!(bus_rows[0].rounds(), [round]);
    assert_eq!(bus_rows[0].task(), Some(&task_id));
    assert_eq!(bus_rows[0].run(), Some(&run));
}

#[tokio::test]
async fn a_state_save_replace_failure_records_state_save_failed() {
    let state = solo_state();
    let daemon = Daemon::new(&state, None);
    let (owner, _sinks) = daemon.bare_owner().await;
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
    let layer = HistoryLayer::new(
        LogLevel::Info,
        SharedClock::new(daemon.clock.clone()),
        Secrets::default(),
        Some(started.history.clone()),
    );
    let dispatch = tracing::Dispatch::new(tracing_subscriber::registry().with(layer));
    let state_path = daemon.home.state_file();
    std::fs::remove_file(&state_path).unwrap();
    std::fs::create_dir(&state_path).unwrap();

    let result = owner.save().with_subscriber(dispatch).await;
    assert!(
        matches!(&result, Err(OwnerError::Store(StoreError::Replace { path, .. })) if path == &state_path),
        "{result:?}"
    );
    started.writer.close().unwrap();

    let logs = Reader::open(&path).unwrap().daemon_logs().unwrap();
    let rows: Vec<_> = logs
        .iter()
        .filter(|row| row.event() == LogEvent::StateSaveFailed)
        .collect();
    assert_eq!(
        rows.len(),
        1,
        "the real replace failure must produce one error: {logs:?}"
    );
    assert_eq!(rows[0].level(), LogLevel::Error);
    assert!(rows[0].team().is_none() && rows[0].task().is_none() && rows[0].run().is_none());
    assert!(
        rows[0]
            .fields()
            .get("error")
            .is_some_and(|error| !error.to_string().is_empty())
    );
}

#[tokio::test]
async fn a_live_daemon_log_write_failure_falls_back_to_stderr_and_the_round_merges() {
    let mut rig = Rig::new(
        &[
            normal("claude-1", "work"),
            investigator("inv-r1", "done:VERDICT: APPROVE\\nSUMMARY: fine"),
            investigator("inv-r2", "done:VERDICT: APPROVE\\nSUMMARY: fine"),
        ],
        5,
        false,
    )
    .await;
    let stderr = Arc::new(Mutex::new(Vec::new()));
    let captured = Arc::clone(&stderr);
    let layer = HistoryLayer::deferred(
        LogLevel::Info,
        SharedClock::new(agent_runtime::clock::SystemClock),
        Secrets::default(),
    );
    layer.with_stderr_sink(Arc::new(move |line: &str| {
        captured.lock().unwrap().push(line.to_owned());
    }));
    rig.start_with_log_layer(layer).await;
    let db = rusqlite::Connection::open(rig.home.history_file()).unwrap();
    db.execute_batch(
        "CREATE TRIGGER reject_live_daemon_logs BEFORE INSERT ON daemon_logs \
         BEGIN SELECT RAISE(FAIL, 'injected live daemon_logs failure'); END;",
    )
    .unwrap();

    rig.run(&["claude-1"], ONE, MergeMode::Autopilot, None).await;
    let state = rig
        .item_where("1.1", "merged", |phase| matches!(phase, ItemPhase::Done { .. }))
        .await;
    assert!(matches!(
        state.teams()[0].item("1.1".parse().unwrap()).unwrap().phase(),
        ItemPhase::Done {
            completion: Completion::Merged { .. }
        }
    ));
    rig.wait_for("the live daemon-log fallback", || {
        let lines = stderr.lock().unwrap().join("\n");
        lines.contains("the history cannot keep a log entry") && lines.contains("event=run_ended")
    })
    .await;
    let reply = crate::client::Connection::connect(&rig.home.socket_file())
        .await
        .unwrap()
        .request(Client::Cli, Command::Status(StatusRequest { target: None }))
        .await
        .unwrap();
    assert!(matches!(reply, Reply::Teams { .. }), "{reply:?}");
    rig.stop().await.unwrap();
    drop(db);

    let joined = stderr.lock().unwrap().join("\n");
    assert_eq!(
        joined.matches("the history cannot keep a log entry").count(),
        1,
        "{joined}"
    );
    assert!(joined.contains("event=run_ended"), "{joined}");
}

/// Database path passed only to the process-isolated spawned-panic child test.
const SPAWNED_PANIC_DB: &str = "AGENT_TEST_SPAWNED_PANIC_DB";

#[test]
fn a_panic_in_a_spawned_daemon_side_task_records_daemon_panicked() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("history.db");
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "daemon::owner::rounds::daemon_log_tests::spawned_daemon_panic_child_process",
            "--nocapture",
        ])
        .env(SPAWNED_PANIC_DB, &path)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "spawned-panic child failed:\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let logs = Reader::open(&path).unwrap().daemon_logs().unwrap();
    let rows: Vec<_> = logs
        .iter()
        .filter(|row| row.event() == LogEvent::DaemonPanicked)
        .collect();
    assert_eq!(rows.len(), 1, "the real spawned panic must produce one row: {logs:?}");
    assert_eq!(rows[0].level(), LogLevel::Error);
    assert!(
        rows[0].fields()["message"]
            .to_string()
            .contains("injected spawned daemon panic")
    );
    assert!(rows[0].fields().get("location").is_some());
    assert!(rows[0].fields().get("thread").is_some());
}

/// Runs in its own test process because installing the panic hook changes process-global state.
#[tokio::test]
async fn spawned_daemon_panic_child_process() {
    let Some(path) = std::env::var_os(SPAWNED_PANIC_DB) else {
        return;
    };
    let db = HistoryDb::open(Path::new(&path), MaxTextBytes::default()).unwrap();
    let started = History::start(
        db,
        Setup {
            reporter: Arc::new(SilentReporter),
            capacity: DEFAULT_QUEUE,
        },
    )
    .unwrap();
    let layer = HistoryLayer::new(
        LogLevel::Info,
        SharedClock::new(agent_runtime::clock::SystemClock),
        Secrets::default(),
        Some(started.history.clone()),
    );
    install_panic_hook(layer.clone());
    let dispatch = tracing::Dispatch::new(tracing_subscriber::registry().with(layer));
    let mut sides = tokio::task::JoinSet::new();
    sides.spawn(async { panic!("injected spawned daemon panic") }.with_subscriber(dispatch.clone()));

    let failure = crate::daemon::join_sides(&mut sides, &dispatch).await;
    assert!(
        matches!(failure, Some(crate::daemon::DaemonError::Join(_))),
        "{failure:?}"
    );
    started.writer.close().unwrap();
}

#[test]
fn an_assignable_decision_with_no_result_writes_no_decision_row() {
    let state = pair_state();
    let daemon = Daemon::new(&state, None);
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
    let layer = HistoryLayer::new(
        LogLevel::Debug,
        SharedClock::new(daemon.clock.clone()),
        Secrets::default(),
        Some(started.history.clone()),
    );
    let subscriber = tracing_subscriber::registry().with(layer);

    let decided = tracing::subscriber::with_default(subscriber, || {
        crate::daemon::team::assignable(&state.teams()[0]).unwrap()
    });
    assert!(decided.is_empty(), "{decided:?}");
    started.writer.close().unwrap();

    let logs = Reader::open(&path).unwrap().daemon_logs().unwrap();
    assert!(
        !logs.iter().any(|row| {
            row.event() == LogEvent::DecisionTaken
                && row.fields().get("decision").and_then(|value| value.as_str()) == Some("assignable")
        }),
        "an empty assignment is not a decision: {logs:?}"
    );
}

#[tokio::test]
async fn unwritable_history_db_writes_events_to_stderr_without_looping_and_finishes_task() {
    let mut rig = Rig::new(
        &[
            normal("claude-1", "work"),
            investigator("inv-r1", "done:VERDICT: APPROVE\\nSUMMARY: fine"),
            investigator("inv-r2", "done:VERDICT: APPROVE\\nSUMMARY: fine"),
        ],
        5,
        false,
    )
    .await;
    rig.start().await;

    let db = rusqlite::Connection::open(rig.home.history_file()).unwrap();
    db.execute_batch(
        "CREATE TRIGGER reject_daemon_logs BEFORE INSERT ON daemon_logs \
         BEGIN SELECT RAISE(FAIL, 'injected daemon_logs failure'); END;",
    )
    .unwrap();

    rig.run(&["claude-1"], ONE, MergeMode::Autopilot, None).await;
    let state = rig
        .item_where("1.1", "merged", |phase| matches!(phase, ItemPhase::Done { .. }))
        .await;
    assert!(matches!(
        state.teams()[0].item("1.1".parse().unwrap()).unwrap().phase(),
        ItemPhase::Done {
            completion: Completion::Merged { .. }
        }
    ));

    let reply = crate::client::Connection::connect(&rig.home.socket_file())
        .await
        .unwrap()
        .request(Client::Cli, Command::Status(StatusRequest { target: None }))
        .await
        .unwrap();
    // Failed daemon-log writes do not increment the domain record loss counter (`HistoryHealth::lost`), so `status`
    // returns `Reply::Teams` rather than `Reply::Overview`.
    assert!(matches!(reply, Reply::Teams { .. }), "{reply:?}");
    rig.stop().await.unwrap();

    // Verify stderr fallback directly when history is unavailable: rate-limited notice + event lines, no recursion.
    let stderr_lines: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let captured = Arc::clone(&stderr_lines);
    let broken_err = std::io::Error::new(std::io::ErrorKind::PermissionDenied, "history.db is read-only");
    let unavailable = History::unavailable(&broken_err, Arc::new(SilentReporter));
    let daemon = Daemon::new(&solo_state(), None);
    let layer = HistoryLayer::new(
        LogLevel::Info,
        SharedClock::new(daemon.clock.clone()),
        Secrets::default(),
        Some(unavailable),
    );
    layer.with_stderr_sink(Arc::new(move |line: &str| {
        captured.lock().unwrap().push(line.to_owned());
    }));
    let subscriber = tracing_subscriber::registry().with(layer);
    tracing::subscriber::with_default(subscriber, || {
        for _ in 0..3 {
            tracing::info!(
                event = LogEvent::TeamChanged.as_str(),
                change = "started",
                "a team is started"
            );
        }
    });
    let joined = stderr_lines.lock().unwrap().join("\n");
    assert_eq!(
        joined.matches("the history cannot keep a log entry").count(),
        1,
        "{joined}"
    );
    assert_eq!(joined.matches("team_changed").count(), 3, "{joined}");
}

#[test]
fn every_log_event_variant_is_produced_with_its_level_and_ids() {
    let state = solo_state();
    let daemon = Daemon::new(&state, None);
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
    let layer = HistoryLayer::new(
        LogLevel::Debug,
        SharedClock::new(daemon.clock.clone()),
        Secrets::default(),
        Some(started.history.clone()),
    );
    install_panic_hook(layer.clone());
    let subscriber = tracing_subscriber::registry().with(layer);

    let team: TeamId = "team-20261115-090000-all1".parse().unwrap();
    let round: RoundId = "round-20261115-090000-all1".parse().unwrap();
    let task_id: TaskId = "task-20261115-090000-all1".parse().unwrap();
    let run: RunId = "run-20261115-090000-all1".parse().unwrap();

    tracing::subscriber::with_default(subscriber, || {
        let span = tracing::error_span!(
            "run",
            team = %team,
            rounds = %round,
            task = %task_id,
            run = %run,
        );
        let _enter = span.enter();

        // error (3)
        daemon.clock.advance(time::Duration::seconds(1));
        tracing::error!(
            event = LogEvent::StateSaveFailed.as_str(),
            error = "io",
            "cannot save state"
        );
        daemon.clock.advance(time::Duration::seconds(1));
        tracing::error!(
            event = LogEvent::ConfigurationNameMissing.as_str(),
            name = "claude-9",
            "missing name"
        );
        daemon.clock.advance(time::Duration::seconds(1));
        assert!(std::panic::catch_unwind(|| panic!("synthetic panic for daemon_panicked")).is_err());

        // warn (14)
        for variant in [
            LogEvent::CliOutputUnreadable,
            LogEvent::RunEndedWithoutStatus,
            LogEvent::RunHung,
            LogEvent::RunQuotaExhausted,
            LogEvent::RunRateLimited,
            LogEvent::BusDown,
            LogEvent::GitCallFailed,
            LogEvent::GithubCallFailed,
            LogEvent::GateCannotStart,
            LogEvent::GateTimedOut,
            LogEvent::GateDirtyTree,
            LogEvent::QuestionsFileInvalid,
            LogEvent::RestartCorrected,
            LogEvent::RequestRefused,
        ] {
            daemon.clock.advance(time::Duration::seconds(1));
            tracing::warn!(
                event = variant.as_str(),
                variant_name = variant.as_str(),
                "warning event"
            );
        }

        // info (6)
        for variant in [
            LogEvent::TeamChanged,
            LogEvent::RoundChanged,
            LogEvent::ItemPhaseChanged,
            LogEvent::TaskStatusChanged,
            LogEvent::RunStarted,
            LogEvent::RunEnded,
        ] {
            daemon.clock.advance(time::Duration::seconds(1));
            tracing::info!(event = variant.as_str(), variant_name = variant.as_str(), "info event");
        }

        // debug (2)
        daemon.clock.advance(time::Duration::seconds(1));
        log_decision("assignable", &json!({"team": team.as_str()}), &json!([]));
        daemon.clock.advance(time::Duration::seconds(1));
        log_outside_call("git", "status", Duration::from_millis(5), Some(0), "ok");
        emit_the_names_of_the_daemon_causes(&daemon);
    });

    started.writer.close().unwrap();
    let rows = Reader::open(&path).unwrap().daemon_logs().unwrap();
    let seen: BTreeSet<LogEvent> = rows.iter().map(agent_core::log::LogEntry::event).collect();
    let expected: BTreeSet<LogEvent> = LogEvent::ALL.iter().copied().collect();
    assert_eq!(seen, expected, "every LogEvent variant must be recorded in daemon_logs");

    for row in &rows {
        assert_eq!(row.level(), row.event().level());
        assert_eq!(row.team(), Some(&team));
        assert_eq!(row.rounds(), std::slice::from_ref(&round));
        assert_eq!(row.task(), Some(&task_id));
        assert_eq!(row.run(), Some(&run));
    }
}

/// Emits, a second apart and each at its level, the names that `daemon_log_cause_tests` provokes by their causes.
fn emit_the_names_of_the_daemon_causes(daemon: &Daemon) {
    for variant in [
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
    ] {
        daemon.clock.advance(time::Duration::seconds(1));
        match variant.level() {
            LogLevel::Error => tracing::error!(event = variant.as_str(), "error event"),
            LogLevel::Warn => tracing::warn!(event = variant.as_str(), "warning event"),
            LogLevel::Info => tracing::info!(event = variant.as_str(), "info event"),
            LogLevel::Debug => tracing::debug!(event = variant.as_str(), "debug event"),
        }
    }
}

#[tokio::test]
async fn remaining_daemon_diagnostics_are_retained_in_history_and_history_losses_stay_stderr_only() {
    let mut daemon = Daemon::new(&solo_state(), None);
    daemon.scenario(1, "done");
    daemon.start().await;
    daemon.status_is(1, agent_core::status::TaskStatus::Done).await;
    daemon.stop().await.unwrap();

    let db = HistoryDb::open(&daemon.home.history_file(), MaxTextBytes::default()).unwrap();
    let started = History::start(
        db,
        Setup {
            reporter: Arc::new(SilentReporter),
            capacity: DEFAULT_QUEUE,
        },
    )
    .unwrap();
    let stderr_lines: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let captured = Arc::clone(&stderr_lines);
    let layer = HistoryLayer::new(
        LogLevel::Debug,
        SharedClock::new(daemon.clock.clone()),
        Secrets::default(),
        Some(started.history.clone()),
    );
    layer.with_stderr_sink(Arc::new(move |line: &str| {
        captured.lock().unwrap().push(line.to_owned());
    }));
    let dispatch: tracing::Dispatch = tracing_subscriber::registry().with(layer).into();

    async {
        let (_, stream) = std::os::unix::net::UnixStream::pair().unwrap();
        let owned: std::os::fd::OwnedFd = stream.into();
        let std_listener = std::os::unix::net::UnixListener::from(owned);
        std_listener.set_nonblocking(true).unwrap();
        let bad_listener = tokio::net::UnixListener::from_std(std_listener).unwrap();
        let (inbox, _rx) = tokio::sync::mpsc::channel(1);
        let shutdown = tokio_util::sync::CancellationToken::new();
        let cancel = shutdown.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(15)).await;
            cancel.cancel();
        });
        crate::daemon::server::serve(bad_listener, inbox, shutdown).await;

        agent_runtime::history::TracingReporter.report(&Problem {
            cause: agent_runtime::history::Cause::Unavailable,
            detail: "simulated history write loss".to_owned(),
        });
    }
    .with_subscriber(dispatch)
    .await;

    started.writer.close().unwrap();

    let logs = Reader::open(&daemon.home.history_file())
        .unwrap()
        .daemon_logs()
        .unwrap();
    let no_token = logs
        .iter()
        .find(|row| row.event() == LogEvent::GithubCallFailed && row.message().contains("the home has no GitHub token"))
        .expect("expected missing GitHub token diagnostic in daemon_logs");
    assert_eq!(no_token.level(), LogLevel::Warn);
    assert_eq!(no_token.module(), "agent::daemon");

    let accept_failed = logs
        .iter()
        .find(|row| row.event() == LogEvent::RequestRefused && row.message() == "cannot accept a connection")
        .expect("expected socket accept failure diagnostic in daemon_logs");
    assert_eq!(accept_failed.level(), LogLevel::Warn);
    assert_eq!(accept_failed.module(), "agent::daemon::server");
    assert!(
        accept_failed
            .fields()
            .get("error")
            .is_some_and(|val| !val.to_string().is_empty()),
        "{accept_failed:?}"
    );

    let joined_stderr = stderr_lines.lock().unwrap().join("\n");
    assert!(
        joined_stderr.contains("the history cannot keep some records"),
        "expected history loss warning on stderr, got: {joined_stderr}"
    );
    assert!(
        !logs
            .iter()
            .any(|row| row.message().contains("the history cannot keep some records")),
        "history loss warning must stay stderr-only to avoid recursion: {logs:?}"
    );
}

/// Exercises an autopilot round with a malformed CLI line and an initial merge refusal before human merge,
/// recording `CliOutputUnreadable`, `TeamChanged`, `RoundChanged`, `ItemPhaseChanged`, `TaskStatusChanged`,
/// `RunStarted`, `RunEnded`, `DecisionTaken`, `OutsideCallCompleted`, and `GithubCallFailed`.
async fn record_autopilot_round_events(observed: &mut BTreeSet<LogEvent>) {
    let mut rig = Rig::new(
        &[
            normal("claude-1", "script:badline"),
            investigator("inv-r1", "done:VERDICT: APPROVE\\nSUMMARY: fine"),
            investigator("inv-r2", "done:VERDICT: APPROVE\\nSUMMARY: fine"),
        ],
        5,
        false,
    )
    .await;
    rig.github.answer_merges(super::fake_github::MergeAnswer::Refuse(405));
    let mut cfg = std::fs::read_to_string(rig.home.config_file()).unwrap();
    cfg.push_str("\n[log]\nlevel = \"debug\"\n");
    std::fs::write(rig.home.config_file(), cfg).unwrap();
    let scripts = rig.home.root().join("fake-scripts");
    std::fs::create_dir_all(&scripts).unwrap();
    std::fs::write(
        scripts.join("badline.sh"),
        "set -eu\nprintf 'not-valid-json-line\\n'\nprintf 'ok\\n' > work.txt\ngit add work.txt\n\
         git -c user.name=Fake -c user.email=fake@example.com -c commit.gpgsign=false commit -qm 'Item 1.1'\n",
    )
    .unwrap();
    rig.start().await;
    rig.run(&["claude-1"], ONE, MergeMode::Autopilot, None).await;
    rig.notification_with("could not be merged").await;
    rig.github.merged_by_a_human(1);
    let round_state = rig
        .item_where("1.1", "merged", |phase| matches!(phase, ItemPhase::Done { .. }))
        .await;
    rig.stop().await.unwrap();

    let rig_team = round_state.teams()[0].id().clone();
    let rig_round = round_state.teams()[0].rounds()[0].id().clone();
    let author_member = &round_state.teams()[0].crew().roster()[0].1;
    let author_task = author_member.task().clone();
    let author_run = author_member.runs()[0].id().clone();

    let rig_logs = Reader::open(&rig.home.history_file()).unwrap().daemon_logs().unwrap();
    for required in [
        LogEvent::CliOutputUnreadable,
        LogEvent::GithubCallFailed,
        LogEvent::TeamChanged,
        LogEvent::RoundChanged,
        LogEvent::ItemPhaseChanged,
        LogEvent::TaskStatusChanged,
        LogEvent::RunStarted,
        LogEvent::RunEnded,
        LogEvent::DecisionTaken,
        LogEvent::OutsideCallCompleted,
    ] {
        let row = rig_logs
            .iter()
            .find(|entry| entry.event() == required && entry.team() == Some(&rig_team))
            .unwrap_or_else(|| panic!("missing {required} with team in autopilot round logs: {rig_logs:?}"));
        assert_eq!(row.level(), required.level());
        if matches!(
            required,
            LogEvent::CliOutputUnreadable | LogEvent::RunStarted | LogEvent::RunEnded
        ) {
            assert_eq!(row.rounds(), std::slice::from_ref(&rig_round));
            assert_eq!(row.task(), Some(&author_task));
            assert_eq!(row.run(), Some(&author_run));
        }
        observed.insert(required);
    }
}

/// Resumes or unpauses `task(1)` on `owner`.
async fn send_task_control(owner: &mut crate::daemon::owner::Owner, unpause: bool) {
    let control = agent_core::protocol::ControlRequest {
        target: Target::Task(task(1)),
    };
    let command = if unpause {
        Command::Unpause(control)
    } else {
        Command::Resume(control)
    };
    let (reply, _rx) = tokio::sync::oneshot::channel();
    owner
        .handle(Message::Request {
            client: Client::Cli,
            command,
            reply,
        })
        .await
        .unwrap();
}

/// Exercises owner and runtime failure causes (`RestartCorrected`, `RunEndedWithoutStatus`, `RunQuotaExhausted`,
/// `RunRateLimited`, `RunHung`, `QuestionsFileInvalid` in both `owner.rs` and `owner/answers.rs`, `BusDown`,
/// `RequestRefused`, `ConfigurationNameMissing` for `agent` and `repo`, `StateSaveFailed`, and `GitCallFailed`).
async fn record_owner_and_runtime_failure_events(dir: &Path, base_daemon: &Daemon) {
    let (mut owner, _sinks) = base_daemon.bare_owner().await;
    base_daemon.scenario(1, "nostatus");
    owner.start_queued().await.unwrap();
    let running_state = owner.parts.state.clone();
    let joined = owner.runs.join_next_with_id().await.unwrap();
    owner.settle_joined(joined).await.unwrap();

    let recover_daemon = Daemon::new(&running_state, None);
    let (mut recovering_owner, _recover_sinks) = recover_daemon.bare_owner().await;
    recovering_owner.recover().await.unwrap();

    for (index, scenario) in ["quota", "ratelimit", "hang"].into_iter().enumerate() {
        base_daemon.scenario(1, scenario);
        send_task_control(&mut owner, index > 0).await;
        owner.start_queued().await.unwrap();
        let joined = owner.runs.join_next_with_id().await.unwrap();
        owner.settle_joined(joined).await.unwrap();
    }

    let missing_q = dir.join("missing-questions.md");
    base_daemon.scenario(1, &format!("questions {}", missing_q.display()));
    owner.start_queued().await.unwrap();
    let joined = owner.runs.join_next_with_id().await.unwrap();
    owner.settle_joined(joined).await.unwrap();

    let qfile = base_daemon.questions_file(1, 1);
    base_daemon.scenario(1, &format!("questions {}", qfile.display()));
    send_task_control(&mut owner, false).await;
    owner.start_queued().await.unwrap();
    let joined = owner.runs.join_next_with_id().await.unwrap();
    owner.settle_joined(joined).await.unwrap();
    let mut parsed_q = agent_core::questions::QuestionsFile::parse(&std::fs::read_to_string(&qfile).unwrap()).unwrap();
    parsed_q.mark_answered();
    std::fs::write(&qfile, parsed_q.render()).unwrap();
    owner
        .reported(crate::daemon::questions::watch::Report::Rescan)
        .await
        .unwrap();

    let pair_daemon = Daemon::new(&pair_state(), Some(Path::new("false")));
    let (mut pair_owner, _pair_sinks) = pair_daemon.bare_owner().await;
    pair_owner.look_at(&team_id(0));
    pair_owner.team_life().await.unwrap();

    let (reply, answer) = tokio::sync::oneshot::channel();
    pair_owner
        .handle(Message::Request {
            client: Client::Cli,
            command: Command::Status(StatusRequest {
                target: Some(Target::Task("task-20261115-090000-none".parse().unwrap())),
            }),
            reply,
        })
        .await
        .unwrap();
    answer.await.unwrap().unwrap_err();

    record_config_and_storage_failure_events(dir).await;
}

/// Exercises `ConfigurationNameMissing` (for both `agent` and `repo`), `StateSaveFailed`, and `GitCallFailed`.
async fn record_config_and_storage_failure_events(dir: &Path) {
    let missing_cfg_daemon = Daemon::new(&pair_state(), None);
    std::fs::write(
        missing_cfg_daemon.home.config_file(),
        "[agents.claude-1]\ncli = \"claude\"\n[tiers]\nnormal = [\"claude-1\"]\n\
         [repos.team-agents]\nbase_branch = \"main\"\ngate = [\"true\"]\n",
    )
    .unwrap();
    let (mut missing_agent_owner, _s1) = missing_cfg_daemon.bare_owner().await;
    missing_agent_owner.start_queued().await.unwrap();

    let missing_repo_daemon = Daemon::new(&solo_state(), None);
    missing_repo_daemon.scenario(1, "done");
    std::fs::write(
        missing_repo_daemon.home.config_file(),
        "[agents.claude-1]\ncli = \"claude\"\n[tiers]\nnormal = [\"claude-1\"]\n\
         [repos.other-repo]\nbase_branch = \"main\"\ngate = [\"true\"]\n",
    )
    .unwrap();
    let (mut missing_repo_owner, _s2) = missing_repo_daemon.bare_owner().await;
    missing_repo_owner.start_queued().await.unwrap();

    let save_daemon = Daemon::new(&solo_state(), None);
    let (save_owner, _s3) = save_daemon.bare_owner().await;
    let state_path = save_daemon.home.state_file();
    std::fs::remove_file(&state_path).unwrap();
    std::fs::create_dir(&state_path).unwrap();
    save_owner.save().await.unwrap_err();

    let git_span = tracing::error_span!("task", team = %team_id(0), task = %task(1));
    let git = agent_runtime::git::Git::new(
        dir.to_str().unwrap().parse().unwrap(),
        agent_runtime::git::CommitIdentity::new("Test".to_owned(), "test@example.com".to_owned(), None),
    );
    git.rev_parse(&agent_runtime::git::Revision::Head)
        .instrument(git_span)
        .await
        .unwrap_err();
}

/// Exercises `GateCannotStart`, `GateTimedOut`, and `GateDirtyTree` via `gate::run_recorded`.
async fn record_gate_failure_events(dir: &Path, history: History) {
    let gate_daemon = Daemon::new(&solo_state(), None);
    let gate_store = Arc::new(
        agent_runtime::store::StateStore::open(gate_daemon.home.clone())
            .await
            .unwrap(),
    );
    let (events_tx, _events_rx) = tokio::sync::broadcast::channel(16);
    let (gate_queue, gate_rx) = crate::daemon::gate::GateQueue::new();
    let gate_shutdown = tokio_util::sync::CancellationToken::new();
    let abs = |p: &Path| p.to_str().unwrap().parse::<agent_core::config::AbsolutePath>().unwrap();
    let gate_parts = crate::daemon::gate::GateParts {
        store: Arc::clone(&gate_store),
        clock: SharedClock::new(gate_daemon.clock.clone()),
        events: events_tx,
        agent_path: vec![abs(Path::new("/usr/bin")), abs(Path::new("/bin"))],
        stop_grace: Duration::from_secs(1),
    };
    let worktree = gate_daemon.home.worktree_dir(&crate::fixtures::repo(), &task(1));
    std::fs::create_dir_all(&worktree).unwrap();
    for args in [
        &["init", "-q", "-b", "main"][..],
        &["config", "user.name", "Test"][..],
        &["config", "user.email", "test@example.com"][..],
        &["-c", "commit.gpgsign=false", "commit", "--allow-empty", "-qm", "init"][..],
    ] {
        assert!(
            std::process::Command::new("git")
                .args(args)
                .current_dir(&worktree)
                .status()
                .unwrap()
                .success()
        );
    }

    let cmd = |script: &str| {
        agent_core::config::CommandLine::try_from(vec!["sh".to_owned(), "-c".to_owned(), script.to_owned()]).unwrap()
    };
    let mut replies = Vec::new();
    for (wt, script, timeout) in [
        (abs(&dir.join("nonexistent-worktree")), "true", Duration::from_secs(5)),
        (abs(&worktree), "sleep 30", Duration::from_millis(50)),
        (abs(&worktree), "printf 'dirty\\n' > dirty.txt", Duration::from_secs(5)),
    ] {
        let (reply, answer) = tokio::sync::oneshot::channel();
        gate_queue
            .enqueue(crate::daemon::gate::GateJob {
                task: task(1),
                team: team_id(0),
                worktree: wt,
                command: cmd(script),
                timeout,
                cancel: tokio_util::sync::CancellationToken::new(),
                hold: None,
                reply,
            })
            .unwrap();
        replies.push(answer);
    }
    let gate_task = tokio::spawn(
        crate::daemon::gate::run_recorded(gate_rx, gate_parts, history, gate_shutdown.clone())
            .with_subscriber(tracing::dispatcher::get_default(Clone::clone)),
    );
    replies.remove(0).await.unwrap().unwrap_err();
    replies.remove(0).await.unwrap().unwrap();
    replies.remove(0).await.unwrap().unwrap();
    gate_shutdown.cancel();
    gate_task.await.unwrap();
}

/// Verifies that `logs` contains every owner, runtime, and gate failure event with its level and scope IDs.
fn assert_owner_and_gate_failure_logs(logs: &[agent_core::log::LogEntry], observed: &mut BTreeSet<LogEvent>) {
    for run_event in [
        LogEvent::RunEndedWithoutStatus,
        LogEvent::RunQuotaExhausted,
        LogEvent::RunRateLimited,
        LogEvent::RunHung,
    ] {
        let row = logs
            .iter()
            .find(|entry| entry.event() == run_event && entry.run().is_some())
            .unwrap_or_else(|| panic!("missing {run_event} with run scope in {logs:?}"));
        assert_eq!(row.level(), LogLevel::Warn);
        assert_eq!(row.team(), Some(&team_id(0)));
        assert_eq!(row.task(), Some(&task(1)));
        observed.insert(run_event);
    }
    assert!(
        logs.iter().any(|entry| {
            entry.event() == LogEvent::RunEndedWithoutStatus && entry.message() == "a run cannot be started"
        }),
        "expected fail_to_start diagnostic in daemon_logs: {logs:?}"
    );

    for expected_module in ["agent::daemon::owner", "agent::daemon::owner::answers"] {
        assert!(
            logs.iter().any(|entry| {
                entry.event() == LogEvent::QuestionsFileInvalid
                    && entry.module() == expected_module
                    && entry.team() == Some(&team_id(0))
                    && entry.task() == Some(&task(1))
            }),
            "missing {expected_module} QuestionsFileInvalid row: {logs:?}"
        );
    }
    observed.insert(LogEvent::QuestionsFileInvalid);

    for expected_kind in ["agent", "repo"] {
        let row = logs
            .iter()
            .find(|entry| {
                entry.event() == LogEvent::ConfigurationNameMissing
                    && entry.module() == "agent::daemon::owner"
                    && entry.fields().get("kind").and_then(|v| v.as_str()) == Some(expected_kind)
            })
            .unwrap_or_else(|| panic!("missing ConfigurationNameMissing for kind={expected_kind}: {logs:?}"));
        assert_eq!(row.level(), LogLevel::Error);
        assert_eq!(row.team(), Some(&team_id(0)));
        assert!(row.task().is_some());
    }
    observed.insert(LogEvent::ConfigurationNameMissing);

    for (event, expected_team, expected_task) in [
        (LogEvent::RestartCorrected, None, None),
        (LogEvent::BusDown, Some(team_id(0)), None),
        (LogEvent::RequestRefused, None, None),
        (LogEvent::StateSaveFailed, None, None),
        (LogEvent::GitCallFailed, Some(team_id(0)), Some(task(1))),
        (LogEvent::GateCannotStart, Some(team_id(0)), Some(task(1))),
        (LogEvent::GateTimedOut, Some(team_id(0)), Some(task(1))),
        (LogEvent::GateDirtyTree, Some(team_id(0)), Some(task(1))),
    ] {
        let row = logs
            .iter()
            .find(|entry| entry.event() == event)
            .unwrap_or_else(|| panic!("missing {event} in {logs:?}"));
        assert_eq!(row.level(), event.level());
        assert_eq!(row.team(), expected_team.as_ref(), "wrong team for {event}: {row:?}");
        assert_eq!(row.task(), expected_task.as_ref(), "wrong task for {event}: {row:?}");
        observed.insert(event);
    }
}

#[tokio::test]
async fn every_production_log_event_cause_is_recorded_with_its_level_and_scope_ids() {
    let mut observed = BTreeSet::new();
    record_autopilot_round_events(&mut observed).await;

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
    let base_daemon = Daemon::new(&solo_state(), Some(Path::new("false")));
    let layer = HistoryLayer::new(
        LogLevel::Debug,
        SharedClock::new(base_daemon.clock.clone()),
        Secrets::default(),
        Some(started.history.clone()),
    );
    let dispatch: tracing::Dispatch = tracing_subscriber::registry().with(layer).into();

    async {
        record_owner_and_runtime_failure_events(dir.path(), &base_daemon).await;
        record_gate_failure_events(dir.path(), started.history.clone()).await;
    }
    .with_subscriber(dispatch)
    .await;

    started.writer.close().unwrap();
    let logs = Reader::open(&path).unwrap().daemon_logs().unwrap();
    assert_owner_and_gate_failure_logs(&logs, &mut observed);

    let panic_path = dir.path().join("panic-history.db");
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "daemon::owner::rounds::daemon_log_tests::spawned_daemon_panic_child_process",
            "--nocapture",
        ])
        .env(SPAWNED_PANIC_DB, &panic_path)
        .output()
        .unwrap();
    assert!(output.status.success());
    let panic_logs = Reader::open(&panic_path).unwrap().daemon_logs().unwrap();
    let panic_row = panic_logs
        .iter()
        .find(|entry| entry.event() == LogEvent::DaemonPanicked)
        .expect("missing DaemonPanicked row");
    assert_eq!(panic_row.level(), LogLevel::Error);
    observed.insert(LogEvent::DaemonPanicked);
    super::daemon_log_cause_tests::record_named_daemon_causes(&mut observed).await;

    let expected: BTreeSet<LogEvent> = LogEvent::ALL.iter().copied().collect();
    assert_eq!(
        observed, expected,
        "every LogEvent variant must be produced by a real production cause"
    );
}
