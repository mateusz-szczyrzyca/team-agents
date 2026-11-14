//! Known secret values are removed before the writer can persist them.

use std::sync::{Arc, Mutex};

use agent_core::cli::ActivityEvent;
use agent_core::config::Secrets;
use agent_core::history::{Record, RoundSubject};
use agent_runtime::history::{DEFAULT_QUEUE, History, Problem, Reporter, Setup, Store, WriteError};

use super::support::world;

/// What crossed the persistence boundary.
#[derive(Debug, Default, Clone)]
struct Memory(Arc<Mutex<Vec<Record>>>);
impl Store for Memory {
    fn write(&mut self, records: &[Record]) -> Result<(), WriteError> {
        self.0.lock().unwrap().extend_from_slice(records);
        Ok(())
    }
}

/// Diagnostics without a process-wide subscriber.
#[derive(Debug, Default)]
struct Reports(Mutex<Vec<Problem>>);
impl Reporter for Reports {
    fn report(&self, problem: &Problem) {
        self.0.lock().unwrap().push(problem.clone());
    }
}

#[test]
fn seeded_free_text_is_redacted_before_any_store_sees_it() {
    for seed in 0..32 {
        let mut w = world(seed);
        let marker = format!("private-{seed}-é😀");
        let raw = format!("before {marker} after");
        let safe = "before [REDACTED] after";
        w.team.task_list.clone_from(&raw);
        for round in &mut w.rounds {
            round.subject = RoundSubject::Brief(raw.clone());
        }
        for run in &mut w.runs {
            run.start.prompt.clone_from(&raw);
            if let Some(launch) = &mut run.launch {
                launch.args = vec![raw.clone(), "ordinary".to_owned()];
            }
            for step in &mut run.steps {
                step.event = ActivityEvent::Text(raw.clone());
            }
            if let Some(end) = &mut run.end {
                end.final_message = Some(raw.clone());
            }
            if let Some(changes) = &mut run.changes {
                for commit in &mut changes.commits {
                    commit.subject.clone_from(&raw);
                }
            }
        }
        let secrets = Secrets::parse(&format!("github_token = {marker:?}"), 0o600).unwrap();
        let store = Memory::default();
        let stored = Arc::clone(&store.0);
        let started = History::start_with_secrets(
            store,
            Setup {
                capacity: DEFAULT_QUEUE,
                reporter: Arc::new(Reports::default()),
            },
            secrets,
        )
        .unwrap();
        let originals = w.records();
        for record in &originals {
            started.history.record(record.clone());
        }
        started.writer.close().unwrap();
        assert_eq!(started.history.health().lost, 0, "seed {seed}");
        let got = stored.lock().unwrap();
        assert_eq!(got.len(), originals.len(), "seed {seed}");
        for (before, after) in originals.iter().zip(got.iter()) {
            // All injected free text must change, while every other field stays identical.
            assert_eq!(
                format!("{after:?}"),
                format!("{before:?}").replace(&raw, safe),
                "seed {seed}"
            );
        }
    }
}

#[test]
fn secret_in_a_structured_identity_is_omitted_counted_and_reported_once() {
    let w = world(42);
    let secrets = Secrets::parse(&format!("github_token = {:?}", w.team.team.as_str()), 0o600).unwrap();
    let reports = Arc::new(Reports::default());
    let store = Memory::default();
    let stored = Arc::clone(&store.0);
    let started = History::start_with_secrets(
        store,
        Setup {
            capacity: DEFAULT_QUEUE,
            reporter: Arc::clone(&reports) as Arc<dyn Reporter>,
        },
        secrets,
    )
    .unwrap();
    for _ in 0..2 {
        started.history.record(Record::Team(Box::new(w.team.clone())));
    }
    started.writer.close().unwrap();
    assert_eq!(started.history.health().lost, 2);
    assert_eq!(*stored.lock().unwrap(), Vec::new());
    let problems = reports.0.lock().unwrap();
    assert_eq!(problems.len(), 1);
    assert!(!format!("{problems:?}").contains(w.team.team.as_str()));
}

#[test]
fn all_activity_text_fields_and_overlapping_unicode_values_are_redacted() {
    let w = super::support::complete(3);
    let raw = "left αsecret😀suffix right";
    let safe = "left [REDACTED] right";
    let events = [
        ActivityEvent::Text(raw.to_owned()),
        ActivityEvent::Command(raw.to_owned()),
        ActivityEvent::Error(raw.to_owned()),
        ActivityEvent::Info(raw.to_owned()),
        ActivityEvent::FileEdit { path: raw.to_owned() },
        ActivityEvent::BusCall {
            tool: raw.to_owned(),
            arguments: raw.to_owned(),
        },
        ActivityEvent::ToolCall {
            name: raw.to_owned(),
            summary: raw.to_owned(),
        },
    ];
    let secrets = Secrets::parse(
        "github_token = 'αsecret😀'\n[notification_env]\nCREDENTIAL = '😀suffix'",
        0o600,
    )
    .unwrap();
    let store = Memory::default();
    let stored = Arc::clone(&store.0);
    let started = History::start_with_secrets(
        store,
        Setup {
            capacity: DEFAULT_QUEUE,
            reporter: Arc::new(Reports::default()),
        },
        secrets,
    )
    .unwrap();
    for (i, event) in events.iter().enumerate() {
        started.history.record(Record::Step(Box::new(agent_core::history::Step {
            run: w.runs[0].start.run.clone(),
            number: u32::try_from(i + 1).unwrap().try_into().unwrap(),
            at: w.team.at,
            event: event.clone(),
        })));
    }
    started.writer.close().unwrap();
    let got = stored.lock().unwrap();
    assert_eq!(got.len(), events.len());
    for (record, event) in got.iter().zip(events) {
        let Record::Step(step) = record else {
            panic!("{record:?}")
        };
        assert_eq!(format!("{:?}", step.event), format!("{event:?}").replace(raw, safe));
    }
}

#[test]
fn same_prefix_and_nested_secrets_redact_the_longest_match() {
    let w = super::support::complete(4);
    let secrets = Secrets::parse(
        "github_token = 'abc'\n[agent_env]\nINNER = 'bcd'\nUNICODE_PREFIX = 'αsecret'\n[notification_env]\nFULL = 'abcdef'\nUNICODE_FULL = 'αsecret😀tail'",
        0o600,
    )
    .unwrap();
    let store = Memory::default();
    let stored = Arc::clone(&store.0);
    let started = History::start_with_secrets(
        store,
        Setup {
            capacity: DEFAULT_QUEUE,
            reporter: Arc::new(Reports::default()),
        },
        secrets,
    )
    .unwrap();
    started.history.record(Record::Step(Box::new(agent_core::history::Step {
        run: w.runs[0].start.run.clone(),
        number: 1.try_into().unwrap(),
        at: w.team.at,
        event: ActivityEvent::Text("before abcdef middle abc and αsecret😀tail after".to_owned()),
    })));
    started.writer.close().unwrap();
    let got = stored.lock().unwrap();
    assert_eq!(got.len(), 1);
    let Record::Step(step) = &got[0] else {
        panic!("{:?}", got[0])
    };
    assert_eq!(
        step.event,
        ActivityEvent::Text("before [REDACTED] middle [REDACTED] and [REDACTED] after".to_owned())
    );
}
