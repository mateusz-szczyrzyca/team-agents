//! Records the spans enclosing named events without replacing production instrumentation.

use std::sync::{Arc, Mutex, PoisonError};

use tracing::{Event, Subscriber, field};
use tracing_subscriber::Layer;
use tracing_subscriber::layer::Context;
use tracing_subscriber::registry::LookupSpan;

/// One event and the names of its enclosing spans, outermost first.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ObservedEvent {
    /// The structured `event` field, or the tracing metadata name when it has none.
    pub(crate) name: String,
    /// The enclosing operation, request, team, task, and run span names.
    pub(crate) spans: Vec<String>,
}

/// A layer that shares every observed event and its scope with its test.
#[derive(Debug, Clone, Default)]
pub(crate) struct ScopeRecorder {
    events: Arc<Mutex<Vec<ObservedEvent>>>,
}

impl ScopeRecorder {
    /// Returns a snapshot of the records observed so far.
    pub(crate) fn events(&self) -> Vec<ObservedEvent> {
        self.events.lock().unwrap_or_else(PoisonError::into_inner).clone()
    }
}

/// Extracts the stable event name used by the daemon log.
#[derive(Default)]
struct EventName(String);

impl field::Visit for EventName {
    fn record_debug(&mut self, _field: &field::Field, _value: &dyn std::fmt::Debug) {}

    fn record_str(&mut self, field: &field::Field, value: &str) {
        if field.name() == "event" {
            self.0 = value.to_owned();
        }
    }
}

impl<S> Layer<S> for ScopeRecorder
where
    S: Subscriber + for<'lookup> LookupSpan<'lookup>,
{
    fn on_event(&self, event: &Event<'_>, context: Context<'_, S>) {
        let mut name = EventName::default();
        event.record(&mut name);
        let name = if name.0.is_empty() {
            event.metadata().name().to_owned()
        } else {
            name.0
        };
        let spans = context
            .event_scope(event)
            .into_iter()
            .flatten()
            .map(|span| span.name().to_owned())
            .collect();
        self.events
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(ObservedEvent { name, spans });
    }
}
