//! The writer and the plans of tasks: a plan is kept for a task whose run the history has, so the records handed in
//! before a plan are written before it, even when they wait in the same batch.

use std::num::{NonZeroU32, NonZeroUsize};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};

use agent_core::cli::ActivityEvent;
use agent_core::history::Record;
use agent_runtime::history::{History, PlanRead, PlanRecord, Problem, Reporter, Setup, Store, WriteError};

use super::support::{at, world};

/// A reporter that keeps nothing: these tests look at the order of the calls only.
#[derive(Debug)]
struct Quiet;

impl Reporter for Quiet {
    fn report(&self, _problem: &Problem) {}
}

/// One call to the store, as the test sees it.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Call {
    /// Records were written, this many.
    Records(usize),
    /// Plans were written, this many.
    Plans(usize),
}

/// A store whose first call of records waits until the test lets it go, and which keeps the order of its calls.
struct Ordered {
    /// The calls, in order.
    calls: Arc<Mutex<Vec<Call>>>,
    /// Tells the test that the first call has started.
    entered: Sender<()>,
    /// Waits for the test; `None` once passed.
    release: Option<Receiver<()>>,
}

impl Store for Ordered {
    fn write(&mut self, records: &[Record]) -> Result<(), WriteError> {
        if let Some(release) = self.release.take() {
            self.entered.send(()).unwrap();
            release.recv().unwrap();
        }
        self.calls.lock().unwrap().push(Call::Records(records.len()));
        Ok(())
    }

    fn write_plans(&mut self, records: &[PlanRecord]) -> Result<(), WriteError> {
        self.calls.lock().unwrap().push(Call::Plans(records.len()));
        Ok(())
    }
}

#[test]
fn records_handed_in_before_a_plan_are_written_before_it() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let (entered, entered_rx) = channel();
    let (release_tx, release) = channel();
    let store = Ordered {
        calls: Arc::clone(&calls),
        entered,
        release: Some(release),
    };
    let setup = Setup {
        reporter: Arc::new(Quiet) as Arc<dyn Reporter>,
        capacity: NonZeroUsize::new(64).unwrap(),
    };
    let started = History::start(store, setup).unwrap();
    let start = world(3).runs[0].start.clone();
    let step = |n: u32| {
        Record::Step(Box::new(agent_core::history::Step {
            run: start.run.clone(),
            number: NonZeroU32::new(n).unwrap(),
            at: at(i64::from(n), 0),
            event: ActivityEvent::Info(format!("step {n} — zażółć")),
        }))
    };
    started.history.record(step(1));
    entered_rx.recv().unwrap();

    started.history.record(step(2));
    started.history.write_plans(vec![PlanRecord {
        task: start.task.clone(),
        team: start.team.clone(),
        run: Some(start.run.clone()),
        at: at(10, 0),
        read: PlanRead::Missing,
    }]);
    release_tx.send(()).unwrap();
    started.history.flush();

    assert_eq!(
        *calls.lock().unwrap(),
        [Call::Records(1), Call::Records(1), Call::Plans(1)]
    );
    started.writer.close().unwrap();
}
