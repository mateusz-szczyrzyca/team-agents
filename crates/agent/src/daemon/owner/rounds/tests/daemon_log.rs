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
use serde_json::json;
use tracing::Instrument;
use tracing::instrument::WithSubscriber;
use tracing_subscriber::Layer;
use tracing_subscriber::layer::{Context, SubscriberExt};
use tracing_subscriber::registry::LookupSpan;

use super::rig::{ONE, Rig, investigator, normal};
use crate::daemon::owner::Message;
use crate::daemon::owner::fixture::{Daemon, pair_state};
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
