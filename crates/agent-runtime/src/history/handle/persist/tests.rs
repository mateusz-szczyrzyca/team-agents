//! Required writes acknowledge database success and propagate failures.
use super::*;
use crate::history::{DEFAULT_QUEUE, HistoryDb, Reader, RecordKind, Setup, Store, TracingReporter, WriteError};
use agent_core::history::{Cut, MaxTextBytes, Record};
use agent_core::log::{LogEvent, LogTeamScope};
use std::collections::BTreeMap;
use std::sync::Arc;

fn entry() -> LogRecord {
    LogRecord {
        at: "2026-11-23T10:00:00Z".parse().unwrap(),
        module: "agent::idle".into(),
        event: LogEvent::DecisionTaken,
        team: None,
        task: None,
        message: "idle removal".into(),
        fields: BTreeMap::new(),
        cut: Cut::default(),
    }
}
fn setup() -> Setup {
    Setup {
        capacity: DEFAULT_QUEUE,
        reporter: Arc::new(TracingReporter),
    }
}
#[tokio::test]
async fn acknowledged_log_is_readable_before_return_and_redacts_secrets() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("history.db");
    let db = HistoryDb::open(&path, MaxTextBytes::default()).unwrap();
    let secrets = agent_core::config::Secrets::parse("github_token = 'secret-value-123'", 0o600).unwrap();
    let started = History::start_with_secrets(db, setup(), secrets).unwrap();
    let mut record = entry();
    record.message = "idle secret-value-123".into();
    started.history.persist_log(record).await.unwrap();
    let entries = Reader::open(&path).unwrap().daemon_logs().unwrap();
    assert_eq!(entries.len(), 1);
    assert!(!entries[0].message().contains("secret-value-123"));
    started.writer.close().unwrap();
}
/// A database boundary that rejects every log transaction.
struct RefusingStore;
impl Store for RefusingStore {
    fn write(&mut self, _records: &[Record]) -> Result<(), WriteError> {
        Ok(())
    }
    fn write_logs(&mut self, _records: &[LogRecord]) -> Result<(), WriteError> {
        Err(WriteError::Rule {
            record: RecordKind::Event,
            rule: "refused log transaction",
        })
    }
}
#[tokio::test]
async fn acknowledged_log_propagates_database_failure_and_missing_writer() {
    let started = History::start(RefusingStore, setup()).unwrap();
    assert!(matches!(
        started.history.persist_log(entry()).await,
        Err(PersistLogError::Write(WriteError::Rule {
            record: RecordKind::Event,
            rule: "refused log transaction"
        }))
    ));
    started.writer.close().unwrap();
    assert!(matches!(
        started.history.persist_log(entry()).await,
        Err(PersistLogError::Unavailable)
    ));
    assert!(matches!(
        History::disabled().persist_log(entry()).await,
        Err(PersistLogError::Unavailable)
    ));
}
#[tokio::test]
async fn acknowledged_log_refuses_credentials_in_identity() {
    let secrets = agent_core::config::Secrets::parse("github_token = 'team-20261123-100000-abcd'", 0o600).unwrap();
    let started = History::start_with_secrets(RefusingStore, setup(), secrets).unwrap();
    let mut record = entry();
    record.team = Some(LogTeamScope::new("team-20261123-100000-abcd".parse().unwrap(), []));
    assert!(matches!(
        started.history.persist_log(record).await,
        Err(PersistLogError::SecretIdentity)
    ));
    started.writer.close().unwrap();
}
