//! Unit tests for [`HistoryLayer`], [`install_panic_hook`], [`log_decision`], and [`log_outside_call`].

use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use agent_core::config::Secrets;
use agent_core::history::MaxTextBytes;
use agent_core::ids::{Clock, RoundId, RunId, TaskId, TeamId};
use agent_core::log::{LogEvent, LogLevel};
use serde_json::json;
use tracing_subscriber::layer::SubscriberExt;

use super::{HistoryLayer, install_panic_hook, log_decision, log_outside_call};
use crate::clock::SharedClock;
use crate::history::{DEFAULT_QUEUE, History, HistoryDb, Problem, Reader, Reporter, Setup};

/// Monotonically advancing clock for deterministic log timestamps.
struct StepClock(AtomicI64);

impl StepClock {
    /// Creates a clock starting at `start` seconds since the Unix epoch.
    const fn new(start: i64) -> Self {
        Self(AtomicI64::new(start))
    }
}

impl Clock for StepClock {
    fn now(&self) -> time::UtcDateTime {
        let secs = self.0.fetch_add(1, Ordering::Relaxed);
        time::UtcDateTime::from_unix_timestamp(secs).unwrap()
    }
}

/// Silent reporter for unit tests that inspect `stderr` via [`HistoryLayer::with_stderr_sink`].
#[derive(Debug)]
struct SilentReporter;

impl Reporter for SilentReporter {
    fn report(&self, _problem: &Problem) {}
}

#[test]
fn every_log_threshold_keeps_exactly_its_levels_and_preserves_span_ids() {
    let team: TeamId = "team-20261115-090000-log1".parse().unwrap();
    let round_a: RoundId = "round-20261115-090000-rnda".parse().unwrap();
    let round_b: RoundId = "round-20261115-090000-rndb".parse().unwrap();
    let task: TaskId = "task-20261115-090000-cld1".parse().unwrap();
    let run: RunId = "run-20261115-090000-run1".parse().unwrap();

    for (threshold, expected) in [
        (LogLevel::Error, vec![LogEvent::StateSaveFailed]),
        (
            LogLevel::Warn,
            vec![LogEvent::StateSaveFailed, LogEvent::CliOutputUnreadable],
        ),
        (
            LogLevel::Info,
            vec![
                LogEvent::StateSaveFailed,
                LogEvent::CliOutputUnreadable,
                LogEvent::RunStarted,
            ],
        ),
        (
            LogLevel::Debug,
            vec![
                LogEvent::StateSaveFailed,
                LogEvent::CliOutputUnreadable,
                LogEvent::RunStarted,
                LogEvent::DecisionTaken,
            ],
        ),
    ] {
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
        let clock = SharedClock::new(StepClock::new(1_763_200_000));
        let layer = HistoryLayer::new(threshold, clock, Secrets::default(), Some(started.history.clone()));
        let subscriber = tracing_subscriber::registry().with(layer);

        tracing::subscriber::with_default(subscriber, || {
            let team_span = tracing::error_span!("team", team = %team);
            let _team_enter = team_span.enter();
            let rounds_csv = format!("{round_a},{round_b}");
            let round_span = tracing::error_span!("round", rounds = %rounds_csv);
            let _round_enter = round_span.enter();
            let task_span = tracing::error_span!("task", task = %task);
            let _task_enter = task_span.enter();
            let run_span = tracing::error_span!("run", run = %run);
            let _run_enter = run_span.enter();

            tracing::error!(
                event = LogEvent::StateSaveFailed.as_str(),
                reason = "disk full",
                "cannot save state"
            );
            tracing::warn!(
                event = LogEvent::CliOutputUnreadable.as_str(),
                detail = "not json",
                "unreadable cli output"
            );
            tracing::info!(event = LogEvent::RunStarted.as_str(), role = "solo", "a run is started");
            tracing::debug!(
                event = LogEvent::DecisionTaken.as_str(),
                decision = "assignable",
                "a decision is taken"
            );
        });

        started.writer.close().unwrap();
        let rows = Reader::open(&path).unwrap().daemon_logs().unwrap();
        let events: Vec<LogEvent> = rows.iter().map(agent_core::log::LogEntry::event).collect();
        assert_eq!(events, expected, "unexpected events for threshold {threshold}");

        for row in &rows {
            assert_eq!(row.team(), Some(&team));
            assert_eq!(row.rounds(), [round_a.clone(), round_b.clone()]);
            assert_eq!(row.task(), Some(&task));
            assert_eq!(row.run(), Some(&run));
        }
    }
}

#[test]
fn log_decision_and_log_outside_call_record_structured_debug_fields() {
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
    let clock = SharedClock::new(StepClock::new(1_763_200_100));
    let layer = HistoryLayer::new(
        LogLevel::Debug,
        clock,
        Secrets::default(),
        Some(started.history.clone()),
    );
    let subscriber = tracing_subscriber::registry().with(layer);
    let team: TeamId = "team-20261115-090000-dbg1".parse().unwrap();

    tracing::subscriber::with_default(subscriber, || {
        let span = tracing::error_span!("team", team = %team);
        let _enter = span.enter();

        log_decision(
            "assignable",
            &json!({ "team": team.as_str(), "items": ["1.1"] }),
            &json!({ "assigned": [{ "item": "1.1", "agent": "claude-1" }] }),
        );
        log_outside_call("git", "rev-parse", Duration::from_millis(42), Some(0), "ok");
    });

    started.writer.close().unwrap();
    let rows = Reader::open(&path).unwrap().daemon_logs().unwrap();
    assert_eq!(rows.len(), 2);

    assert_eq!(rows[0].event(), LogEvent::DecisionTaken);
    assert_eq!(rows[0].level(), LogLevel::Debug);
    assert_eq!(rows[0].team(), Some(&team));
    assert_eq!(rows[0].fields()["decision"], json!("assignable"));
    assert_eq!(rows[0].fields()["input"]["items"], json!(["1.1"]));
    assert_eq!(rows[0].fields()["result"]["assigned"][0]["agent"], json!("claude-1"));

    assert_eq!(rows[1].event(), LogEvent::OutsideCallCompleted);
    assert_eq!(rows[1].level(), LogLevel::Debug);
    assert_eq!(rows[1].team(), Some(&team));
    assert_eq!(rows[1].fields()["tool"], json!("git"));
    assert_eq!(rows[1].fields()["operation"], json!("rev-parse"));
    assert_eq!(rows[1].fields()["duration_ms"], json!(42));
    assert_eq!(rows[1].fields()["exit_code"], json!(0));
    assert_eq!(rows[1].fields()["outcome"], json!("ok"));
}

#[test]
fn panic_hook_records_daemon_panicked_with_redacted_message_and_location() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("history.db");
    let secrets = Secrets::parse("github_token = \"ghp_panic_secret_987654\"\n", 0o600).unwrap();
    let db = HistoryDb::open(&path, MaxTextBytes::default()).unwrap();
    let started = History::start_with_secrets(
        db,
        Setup {
            reporter: Arc::new(SilentReporter),
            capacity: DEFAULT_QUEUE,
        },
        secrets.clone(),
    )
    .unwrap();
    let clock = SharedClock::new(StepClock::new(1_763_200_200));
    let layer = HistoryLayer::new(LogLevel::Info, clock, secrets, Some(started.history.clone()));
    install_panic_hook(layer);

    let caught = std::panic::catch_unwind(|| {
        panic!("fatal crash with token ghp_panic_secret_987654");
    });
    assert!(caught.is_err());

    started.writer.close().unwrap();
    let rows = Reader::open(&path).unwrap().daemon_logs().unwrap();
    let panic_row = rows
        .iter()
        .find(|row| row.event() == LogEvent::DaemonPanicked)
        .expect("expected a daemon_panicked row");
    assert_eq!(panic_row.level(), LogLevel::Error);
    let message = panic_row.fields()["message"].as_str().unwrap_or_default();
    assert!(message.contains("[REDACTED]"), "{message}");
    assert!(!message.contains("ghp_panic_secret_987654"), "{message}");
    let location = panic_row.fields()["location"].as_str().unwrap_or_default();
    assert!(location.contains("tests.rs:"), "{location}");
}

#[test]
fn secret_markers_never_reach_stderr_or_history_db_or_wal_bytes() {
    const MARKERS: [&str; 3] = [
        "ghp_secret_log_alpha_111",
        "sk-ant-secret-beta-222",
        "smtp-secret-gamma-333",
    ];
    let secrets = Secrets::parse(
        "github_token = \"ghp_secret_log_alpha_111\"\n\
         [agent_env]\n\
         ANTHROPIC_API_KEY = \"sk-ant-secret-beta-222\"\n\
         [notification_env]\n\
         SMTP_PASS = \"smtp-secret-gamma-333\"\n",
        0o600,
    )
    .unwrap();

    let stderr_lines: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let captured = Arc::clone(&stderr_lines);
    let clock = SharedClock::new(StepClock::new(1_763_200_300));
    let layer = HistoryLayer::deferred(LogLevel::Debug, clock, secrets.clone());
    layer.with_stderr_sink(Arc::new(move |line: &str| {
        captured.lock().unwrap().push(line.to_owned());
    }));
    let dispatch: tracing::Dispatch = tracing_subscriber::registry().with(layer.clone()).into();

    // 1. Before history.db is open: events go to stderr, redacted.
    tracing::dispatcher::with_default(&dispatch, || {
        tracing::error!(
            event = LogEvent::StateSaveFailed.as_str(),
            detail = "failed with ghp_secret_log_alpha_111 and sk-ant-secret-beta-222 and smtp-secret-gamma-333",
            "cannot save state"
        );
    });

    // 2. While history.db is open: events go to history.db (and WAL), redacted.
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("history.db");
    let db = HistoryDb::open(&path, MaxTextBytes::default()).unwrap();
    let started = History::start_with_secrets(
        db,
        Setup {
            reporter: Arc::new(SilentReporter),
            capacity: DEFAULT_QUEUE,
        },
        secrets.clone(),
    )
    .unwrap();
    layer.attach_history(started.history.clone());

    tracing::dispatcher::with_default(&dispatch, || {
        tracing::warn!(
            event = LogEvent::CliOutputUnreadable.as_str(),
            line = "leaked ghp_secret_log_alpha_111 sk-ant-secret-beta-222 smtp-secret-gamma-333",
            "unreadable line"
        );
        log_decision(
            "assignable",
            &json!({ "token": "ghp_secret_log_alpha_111", "key": "sk-ant-secret-beta-222" }),
            &json!({ "smtp": "smtp-secret-gamma-333" }),
        );
        log_outside_call(
            "github",
            "create_pull ghp_secret_log_alpha_111",
            Duration::from_millis(12),
            Some(500),
            "failed sk-ant-secret-beta-222 smtp-secret-gamma-333",
        );
    });

    started.writer.close().unwrap();

    // 3. When history.db cannot be written: events and the rate-limited notice go to stderr, redacted.
    let broken_err = std::io::Error::new(
        std::io::ErrorKind::PermissionDenied,
        "denied with ghp_secret_log_alpha_111 and sk-ant-secret-beta-222 and smtp-secret-gamma-333",
    );
    let broken = History::unavailable(&broken_err, Arc::new(SilentReporter));
    layer.attach_history(broken);

    tracing::dispatcher::with_default(&dispatch, || {
        tracing::warn!(
            event = LogEvent::BusDown.as_str(),
            error = "bus failed with ghp_secret_log_alpha_111 sk-ant-secret-beta-222 smtp-secret-gamma-333",
            "agentbus call failed"
        );
    });

    let stderr_joined = stderr_lines.lock().unwrap().join("\n");
    assert_redacted_stderr(&stderr_joined, &MARKERS);

    let rows = Reader::open(&path).unwrap().daemon_logs().unwrap();
    assert_eq!(rows.len(), 3);
    assert_no_markers_on_disk(dir.path(), &MARKERS);
}

fn assert_redacted_stderr(stderr_joined: &str, markers: &[&str]) {
    assert!(
        stderr_joined.contains("state_save_failed"),
        "expected pre-open event on stderr: {stderr_joined}"
    );
    assert!(
        stderr_joined.contains("the history cannot keep a log entry"),
        "expected unwritable-history notice on stderr: {stderr_joined}"
    );
    assert!(
        stderr_joined.contains("bus_down"),
        "expected fallback event on stderr: {stderr_joined}"
    );
    assert!(stderr_joined.contains("[REDACTED]"), "{stderr_joined}");
    for marker in markers {
        assert!(
            !stderr_joined.contains(marker),
            "secret marker {marker} leaked to stderr: {stderr_joined}"
        );
    }
}

fn assert_no_markers_on_disk(dir: &std::path::Path, markers: &[&str]) {
    for file in ["history.db", "history.db-wal", "history.db-shm"] {
        let candidate = dir.join(file);
        if candidate.exists() {
            let bytes = std::fs::read(&candidate).unwrap();
            let text = String::from_utf8_lossy(&bytes);
            for marker in markers {
                assert!(!text.contains(marker), "secret marker {marker} leaked into {file}");
            }
        }
    }
}

#[test]
fn constructing_an_unused_layer_does_not_replace_the_active_subscriber() {
    let stderr_lines: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let captured = Arc::clone(&stderr_lines);
    let layer = HistoryLayer::deferred(
        LogLevel::Info,
        SharedClock::new(StepClock::new(1_763_200_400)),
        Secrets::default(),
    );
    layer.with_stderr_sink(Arc::new(move |line: &str| {
        captured.lock().unwrap().push(line.to_owned());
    }));
    let subscriber = tracing_subscriber::registry().with(layer);
    std::thread::spawn(move || {
        tracing::subscriber::with_default(subscriber, || {
            let _unused = HistoryLayer::new(
                LogLevel::Debug,
                SharedClock::new(StepClock::new(1_763_200_500)),
                Secrets::default(),
                None,
            );
            tracing::error!(
                event = LogEvent::StateSaveFailed.as_str(),
                reason = "disk full",
                "the active subscriber must survive"
            );
        });
    })
    .join()
    .unwrap();
    let joined = stderr_lines.lock().unwrap().join("\n");
    assert!(joined.contains("the active subscriber must survive"), "{joined}");
}

#[test]
fn unnamed_and_unknown_events_keep_a_redacted_stderr_diagnostic() {
    let secrets = Secrets::parse("github_token = \"ghp_unknown_diagnostic_secret\"\n", 0o600).unwrap();
    let stderr_lines: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let captured = Arc::clone(&stderr_lines);
    let layer = HistoryLayer::deferred(
        LogLevel::Debug,
        SharedClock::new(StepClock::new(1_763_200_600)),
        secrets,
    );
    layer.with_stderr_sink(Arc::new(move |line: &str| {
        captured.lock().unwrap().push(line.to_owned());
    }));
    let subscriber = tracing_subscriber::registry().with(layer);
    tracing::subscriber::with_default(subscriber, || {
        tracing::error!(detail = "ghp_unknown_diagnostic_secret", "ordinary daemon failure");
        tracing::warn!(
            event = "future_event",
            detail = "keep this too",
            "unknown daemon warning"
        );
    });
    let joined = stderr_lines.lock().unwrap().join("\n");
    assert!(joined.contains("ordinary daemon failure"), "{joined}");
    assert!(joined.contains("unknown daemon warning"), "{joined}");
    assert!(joined.contains("keep this too"), "{joined}");
    assert!(!joined.contains("ghp_unknown_diagnostic_secret"), "{joined}");
    assert!(joined.contains("[REDACTED]"), "{joined}");
}

#[test]
fn quoted_and_whitespace_secrets_are_redacted_before_field_normalization() {
    const QUOTED: &str = "\"private-marker\"";
    const WHITESPACE: &str = "  whitespace-marker  ";
    let secrets = Secrets::parse(
        "github_token = '\"private-marker\"'\n\
         [agent_env]\n\
         SPACED = '  whitespace-marker  '\n",
        0o600,
    )
    .unwrap();
    let stderr_lines: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let captured = Arc::clone(&stderr_lines);
    let layer = HistoryLayer::deferred(
        LogLevel::Debug,
        SharedClock::new(StepClock::new(1_763_200_700)),
        secrets,
    );
    layer.with_stderr_sink(Arc::new(move |line: &str| {
        captured.lock().unwrap().push(line.to_owned());
    }));
    let subscriber = tracing_subscriber::registry().with(layer);
    tracing::subscriber::with_default(subscriber, || {
        tracing::warn!(
            event = LogEvent::CliOutputUnreadable.as_str(),
            detail = QUOTED,
            "quoted secret"
        );
        tracing::warn!(
            event = LogEvent::BusDown.as_str(),
            detail = WHITESPACE,
            "whitespace secret"
        );
    });
    let joined = stderr_lines.lock().unwrap().join("\n");
    assert!(!joined.contains(QUOTED), "{joined}");
    assert!(!joined.contains(WHITESPACE), "{joined}");
    assert_eq!(joined.matches("[REDACTED]").count(), 2, "{joined}");
}

#[test]
fn stderr_fallback_contains_all_available_scope_ids() {
    let team: TeamId = "team-20261115-090000-err1".parse().unwrap();
    let round_a: RoundId = "round-20261115-090000-er1a".parse().unwrap();
    let round_b: RoundId = "round-20261115-090000-er1b".parse().unwrap();
    let task: TaskId = "task-20261115-090000-err1".parse().unwrap();
    let run: RunId = "run-20261115-090000-err1".parse().unwrap();
    let stderr_lines: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let captured = Arc::clone(&stderr_lines);
    let layer = HistoryLayer::deferred(
        LogLevel::Debug,
        SharedClock::new(StepClock::new(1_763_200_800)),
        Secrets::default(),
    );
    layer.with_stderr_sink(Arc::new(move |line: &str| {
        captured.lock().unwrap().push(line.to_owned());
    }));
    let subscriber = tracing_subscriber::registry().with(layer);
    tracing::subscriber::with_default(subscriber, || {
        let span = tracing::error_span!(
            "run",
            team = %team,
            rounds = %format!("{round_a},{round_b}"),
            task = %task,
            run = %run,
        );
        let _enter = span.enter();
        tracing::warn!(event = LogEvent::BusDown.as_str(), reason = "offline", "bus failed");
    });
    let lines = stderr_lines.lock().unwrap();
    assert_eq!(lines.len(), 1, "{lines:?}");
    let normalized = lines[0].split_once(' ').map_or(lines[0].as_str(), |(_, rest)| rest);
    assert_eq!(
        normalized,
        format!(
            "warn agent_runtime::history::log::tests event=bus_down team={team} \
             rounds={round_a},{round_b} task={task} run={run} message=\"bus failed\" \
             fields={{\"reason\":\"offline\"}}"
        )
    );
}

/// A clock in the year -1: no [`agent_core::state::Timestamp`] can hold its time.
struct BeforeYearZero;

impl Clock for BeforeYearZero {
    fn now(&self) -> time::UtcDateTime {
        time::UtcDateTime::new(
            time::Date::from_calendar_date(-1, time::Month::January, 1).unwrap(),
            time::Time::MIDNIGHT,
        )
    }
}

/// A started history in `dir`, and its path.
fn started_history(dir: &std::path::Path) -> (crate::history::Started, std::path::PathBuf) {
    let path = dir.join("history.db");
    let started = History::start(
        HistoryDb::open(&path, MaxTextBytes::default()).unwrap(),
        Setup {
            reporter: Arc::new(SilentReporter),
            capacity: DEFAULT_QUEUE,
        },
    )
    .unwrap();
    (started, path)
}

/// A sink that keeps the lines of `stderr`, and the lines.
fn stderr_capture(layer: &HistoryLayer) -> Arc<Mutex<Vec<String>>> {
    let lines: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let captured = Arc::clone(&lines);
    layer.with_stderr_sink(Arc::new(move |line: &str| {
        captured.lock().unwrap().push(line.to_owned());
    }));
    lines
}

#[test]
fn an_event_whose_time_the_clock_cannot_give_goes_to_stderr_and_not_to_the_history() {
    let dir = tempfile::tempdir().unwrap();
    let (started, path) = started_history(dir.path());
    let layer = HistoryLayer::new(
        LogLevel::Info,
        SharedClock::new(BeforeYearZero),
        Secrets::default(),
        Some(started.history.clone()),
    );
    let stderr = stderr_capture(&layer);
    let subscriber = tracing_subscriber::registry().with(layer);

    tracing::subscriber::with_default(subscriber, || {
        tracing::error!(
            event = LogEvent::StateSaveFailed.as_str(),
            path = "/srv/agents/state.json",
            "the state cannot be saved"
        );
    });
    started.writer.close().unwrap();

    assert_eq!(Reader::open(&path).unwrap().daemon_logs().unwrap(), []);
    let lines = stderr.lock().unwrap();
    let line = lines
        .iter()
        .find(|line| line.contains("the state cannot be saved"))
        .unwrap_or_else(|| panic!("the event is not on stderr: {lines:?}"));
    assert!(line.contains("event=state_save_failed"), "{line}");
    assert!(line.contains("/srv/agents/state.json"), "{line}");
}

#[test]
fn a_scoped_future_dropped_under_another_subscriber_closes_its_spans_in_its_own() {
    let layer = HistoryLayer::deferred(
        LogLevel::Debug,
        SharedClock::new(StepClock::new(1_763_201_200)),
        Secrets::default(),
    );
    let daemon = tracing::Dispatch::new(tracing_subscriber::registry().with(layer));
    let other = tracing::Dispatch::new(tracing_subscriber::registry());
    let team: TeamId = "team-20261115-090000-scp1".parse().unwrap();
    let task: TaskId = "task-20261115-090000-scp1".parse().unwrap();
    let future = tracing::dispatcher::with_default(&daemon, || {
        let parent = tracing::info_span!("team", team = %team);
        let child = tracing::info_span!(parent: &parent, "task", task = %task);
        tracing::Instrument::instrument(
            async move {
                let _child = child;
                std::future::pending::<()>().await;
            },
            parent,
        )
    });
    let mut scoped = Box::pin(super::Scoped::new(future, daemon));
    let waker = std::task::Waker::noop();
    let mut context = std::task::Context::from_waker(waker);
    assert!(scoped.as_mut().poll(&mut context).is_pending());

    // An aborted task is dropped by the runtime, under whatever subscriber the thread has then.
    let dropped = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        tracing::dispatcher::with_default(&other, || drop(scoped));
    }));

    assert!(dropped.is_ok(), "the spans were closed in the other registry");
}

#[test]
fn installed_panic_hook_preserves_another_threads_message_and_location() {
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "history::log::tests::previous_panic_hook_child_process",
            "--nocapture",
        ])
        .env("AGENT_TEST_PREVIOUS_PANIC_HOOK", "1")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "child stdout: {}\nchild stderr: {}",
        String::from_utf8(output.stdout).unwrap(),
        String::from_utf8(output.stderr).unwrap()
    );
}

/// Process isolation keeps the observer hook from intercepting concurrent tests' panics.
#[test]
fn previous_panic_hook_child_process() {
    if std::env::var_os("AGENT_TEST_PREVIOUS_PANIC_HOOK").is_none() {
        return;
    }
    let observed = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&observed);
    let original = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let location = info.location().unwrap();
        sink.lock().unwrap().push((
            info.payload().downcast_ref::<&str>().unwrap().to_string(),
            location.file().to_owned(),
            location.line(),
        ));
    }));
    let layer = HistoryLayer::deferred(
        LogLevel::Info,
        SharedClock::new(StepClock::new(1_763_200_200)),
        Secrets::default(),
    );
    layer.with_stderr_sink(Arc::new(|_| {}));
    install_panic_hook(layer);
    let line = line!() + 2;
    let thread = std::thread::spawn(|| {
        panic!("another test failed: λ");
    });
    let panicked = thread.join().is_err();
    std::panic::set_hook(original);
    assert!(panicked);
    assert_eq!(
        *observed.lock().unwrap(),
        vec![("another test failed: λ".to_owned(), file!().to_owned(), line)]
    );
}
