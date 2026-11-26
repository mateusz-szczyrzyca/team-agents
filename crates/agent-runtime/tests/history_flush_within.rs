//! A wait for the history's writer that gives up: the owner of the daemon waits for its checkpoints to be written, and a
//! writer that is stuck must not stop a round. The stuck store stands in for a database another process holds.

#[cfg(test)]
mod history_flush_within {
    use std::num::{NonZeroU32, NonZeroUsize};
    use std::sync::mpsc::{Receiver, Sender, channel};
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use agent_core::cli::ActivityEvent;
    use agent_core::history::Record;
    use agent_core::ids::RunId;
    use agent_core::state::Timestamp;
    use agent_runtime::history::{Flush, History, Problem, Reporter, Setup, Store, WriteError};

    /// A reporter that says nothing: these tests look at the flush, not at the reports.
    #[derive(Debug)]
    struct Silent;

    impl Reporter for Silent {
        fn report(&self, _problem: &Problem) {}
    }

    /// A store whose first write waits until the test lets it go, after it said it was in the write.
    struct Stuck {
        /// What was written.
        written: Arc<Mutex<Vec<Record>>>,
        /// Tells the test that the first write has started.
        entered: Sender<()>,
        /// Waits for the test; `None` once the gate has been passed.
        release: Option<Receiver<()>>,
    }

    impl Store for Stuck {
        fn write(&mut self, records: &[Record]) -> Result<(), WriteError> {
            if let Some(release) = self.release.take() {
                self.entered.send(()).expect("the test listens");
                release.recv().expect("the test releases the store");
            }
            self.written.lock().expect("a lock").extend_from_slice(records);
            Ok(())
        }
    }

    /// A step record of the run `run`, number `n`.
    fn step(run: &RunId, n: u32) -> Record {
        Record::Step(Box::new(agent_core::history::Step {
            run: run.clone(),
            number: NonZeroU32::new(n).expect("a number"),
            at: Timestamp::from_utc(
                time::UtcDateTime::from_unix_timestamp(1_795_000_000 + i64::from(n)).expect("a time"),
            )
            .expect("a timestamp"),
            event: ActivityEvent::Info(format!("step {n}")),
        }))
    }

    /// A run id.
    fn run() -> RunId {
        "run-20261126-100000-r1r1".parse().expect("a run id")
    }

    /// A history over a [`Stuck`] store, and what the test needs to drive it.
    struct Fixture {
        /// The history and its writer.
        started: agent_runtime::history::Started,
        /// Receives a message when the first write has started.
        entered: Receiver<()>,
        /// Lets the first write go.
        release: Sender<()>,
        /// What the store was given.
        written: Arc<Mutex<Vec<Record>>>,
    }

    fn start(capacity: usize) -> Fixture {
        let (entered, entered_rx) = channel();
        let (release_tx, release) = channel();
        let written = Arc::new(Mutex::new(Vec::new()));
        let store = Stuck {
            written: Arc::clone(&written),
            entered,
            release: Some(release),
        };
        let setup = Setup {
            reporter: Arc::new(Silent),
            capacity: NonZeroUsize::new(capacity).expect("a capacity"),
        };
        Fixture {
            started: History::start(store, setup).expect("the history starts"),
            entered: entered_rx,
            release: release_tx,
            written,
        }
    }

    #[test]
    fn a_flush_within_a_limit_times_out_while_the_writer_is_stuck() {
        let Fixture {
            started,
            entered,
            release,
            written,
        } = start(64);
        started.history.record(step(&run(), 1));
        entered.recv().expect("the writer is in its first write");

        let outcome = started.history.flush_within(Duration::from_millis(50));

        assert_eq!(outcome, Flush::TimedOut);
        release.send(()).expect("the store is released");
        started.writer.close().expect("the writer closes");
        assert_eq!(written.lock().expect("a lock").len(), 1);
    }

    #[test]
    fn a_flush_within_a_limit_is_done_when_the_writer_has_written_everything() {
        let Fixture {
            started,
            entered,
            release,
            written,
        } = start(64);
        started.history.record(step(&run(), 1));
        entered.recv().expect("the writer is in its first write");
        release.send(()).expect("the store is released");

        let outcome = started.history.flush_within(Duration::from_secs(30));

        assert_eq!(outcome, Flush::Done);
        assert_eq!(written.lock().expect("a lock").len(), 1);
        started.writer.close().expect("the writer closes");
    }

    #[test]
    fn a_flush_within_a_limit_times_out_when_the_queue_has_no_room_for_the_question() {
        let Fixture {
            started,
            entered,
            release,
            written: _written,
        } = start(2);
        started.history.record(step(&run(), 1));
        entered.recv().expect("the writer is in its first write");
        started.history.record(step(&run(), 2));
        started.history.record(step(&run(), 3));

        let outcome = started.history.flush_within(Duration::from_secs(30));

        assert_eq!(outcome, Flush::TimedOut);
        release.send(()).expect("the store is released");
        started.writer.close().expect("the writer closes");
    }
}
