//! Which handle of the history is off: only the one that `[history] enabled = false` makes. A history that cannot be
//! opened is not off — its records are lost, and counted.

use std::sync::Arc;

use agent_runtime::history::{History, OpenError, Reporter, Setup, TracingReporter};

use super::support::database;

#[test]
fn only_a_history_that_is_switched_off_is_disabled() {
    let (_dir, _path, db) = database();
    let reporter: Arc<dyn Reporter> = Arc::new(TracingReporter);
    let started = History::start(
        db,
        Setup {
            reporter: Arc::clone(&reporter),
            capacity: std::num::NonZeroUsize::new(4).unwrap(),
        },
    )
    .unwrap();
    let error = OpenError::MigrationOrder { found: 2, expected: 1 };
    let unavailable = History::unavailable(&error, reporter);

    assert!(History::disabled().is_disabled());
    assert!(!started.history.is_disabled());
    assert!(!unavailable.is_disabled());
    started.writer.close().unwrap();
}
