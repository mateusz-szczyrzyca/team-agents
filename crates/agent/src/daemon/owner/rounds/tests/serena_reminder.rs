//! The log event of a Serena reminder, produced as the run engine produces it: a run with Serena that searched code
//! without it past the bound is told once, and the reminder is logged at info with the task, the run and the count.

use std::collections::BTreeSet;
use std::str::FromStr;
use std::sync::Arc;

use agent_core::history::MaxTextBytes;
use agent_core::ids::RunId;
use agent_core::kinds::CliKind;
use agent_core::log::LogEvent;
use agent_runtime::clock::SharedClock;
use agent_runtime::history::{DEFAULT_QUEUE, History, HistoryDb, HistoryLayer, Reader, Setup};
use tracing::instrument::WithSubscriber;
use tracing_subscriber::layer::SubscriberExt;

use crate::daemon::owner::fixture::Daemon as DaemonFixture;
use crate::fixtures::{solo_state, task};

/// Records the reminder of a run in the daemon's log as the engine writes it.
pub(super) async fn record_serena_reminded(observed: &mut BTreeSet<LogEvent>) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("history.db");
    let db = HistoryDb::open(&path, MaxTextBytes::default()).unwrap();
    let started = History::start(
        db,
        Setup {
            reporter: Arc::new(agent_runtime::history::TracingReporter),
            capacity: DEFAULT_QUEUE,
        },
    )
    .unwrap();
    let daemon: DaemonFixture = DaemonFixture::new(&solo_state(), None);
    let layer = HistoryLayer::new(
        agent_core::log::LogLevel::Debug,
        SharedClock::new(daemon.clock.clone()),
        agent_core::config::Secrets::default(),
        Some(started.history.clone()),
    );
    let dispatch: tracing::Dispatch = tracing_subscriber::registry().with(layer).into();

    let task = task(1);
    let run = RunId::from_str("run-20261124-100001-r1r1").expect("a run");
    async {
        tracing::info!(
            event = LogEvent::SerenaReminded.as_str(),
            task = %task,
            run = %run,
            step = 40,
            searches = 40,
            cli = CliKind::Claude.as_str(),
            "the run was told once to find code with Serena"
        );
    }
    .with_subscriber(dispatch)
    .await;
    started.writer.close().unwrap();

    let rows = Reader::open(&path).unwrap().daemon_logs().unwrap();
    let row = rows
        .iter()
        .find(|entry| entry.event() == LogEvent::SerenaReminded)
        .expect("the reminder is recorded in the daemon log");
    assert_eq!(row.level(), LogEvent::SerenaReminded.level());
    assert_eq!(row.task(), Some(&task));
    assert_eq!(row.run(), Some(&run));
    observed.insert(LogEvent::SerenaReminded);
}
