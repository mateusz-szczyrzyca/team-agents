//! The `tracing_subscriber::Layer` that turns daemon log events and their enclosing spans into rows of `history.db`
//! (or redacted lines on `stderr` before `history.db` is open or when it cannot be written), the panic hook, and the
//! `debug` helpers for pure decisions and outside tool calls.

use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};
use std::fmt;
use std::panic::PanicHookInfo;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, LazyLock, Mutex, MutexGuard, Once, OnceLock, PoisonError};
use std::time::Duration;

use agent_core::config::Secrets;
use agent_core::history::Cut;
use agent_core::ids::{RoundId, RunId, TaskId, TeamId};
use agent_core::log::{LogEvent, LogLevel, LogRecord, LogTaskScope, LogTeamScope};
use agent_core::state::Timestamp;
use serde::Serialize;
use serde_json::Value;
use tracing::field::{Field, Visit};
use tracing::span::{Attributes, Id, Record as SpanRecord};
use tracing::{Event, Level, Metadata, Subscriber};
use tracing_subscriber::Layer;
use tracing_subscriber::layer::{Context, SubscriberExt};
use tracing_subscriber::registry::LookupSpan;
use tracing_subscriber::util::SubscriberInitExt;

use super::handle::History;
use super::redact::Redactor;
use crate::clock::{SharedClock, SystemClock};

/// Callback invoked for lines written to the standard error stream fallback.
pub type StderrSink = Arc<dyn Fn(&str) + Send + Sync>;

thread_local! {
    /// Stack of currently entered `(layer_id, span_id)` pairs on the current thread, outermost first.
    static ENTERED_SPANS: RefCell<Vec<(usize, u64)>> = const { RefCell::new(Vec::new()) };
    /// Thread-local fallback subscriber guard so spans created before `.with_subscriber(...)` are still tracked even
    /// when another test thread installed a non-`HistoryLayer` global subscriber.
    static THREAD_SPAN_GUARD: RefCell<Option<tracing::subscriber::DefaultGuard>> = const { RefCell::new(None) };
}

/// Shared map of `(layer_id, span_id)` to its recorded [`SpanScope`], allowing spans created on the global
/// subscriber before `.with_subscriber(...)` to still contribute their scope when entered inside a scoped subscriber.
static SPAN_SCOPES: LazyLock<Mutex<HashMap<(usize, u64), SpanScope>>> = LazyLock::new(|| Mutex::new(HashMap::new()));

/// Global [`HistoryLayer`] installed when `agent daemon` starts.
static GLOBAL_LAYER: OnceLock<HistoryLayer> = OnceLock::new();

/// Whether [`HistoryLayer::install_global`] was called in this process (`agent daemon`), enabling dual output to
/// `stderr` alongside `history.db`.
static GLOBAL_STDERR_ENABLED: AtomicBool = AtomicBool::new(false);

/// Active [`HistoryLayer`] used by the process panic hook.
static PANIC_LAYER: Mutex<Option<HistoryLayer>> = Mutex::new(None);

/// Ensures `std::panic::set_hook` is installed at most once per process.
static PANIC_HOOK_ONCE: Once = Once::new();

/// Locks `mutex`, recovering from poison so logging never panics when another thread panicked.
fn locked<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Mutable state shared across clones of a [`HistoryLayer`].
struct LayerState {
    /// Minimum severity threshold kept by the layer.
    level: LogLevel,
    /// Clock used to timestamp events.
    clock: SharedClock,
    /// Credential redactor applied before writing to `history.db` or `stderr`.
    redactor: Redactor,
    /// Attached `history.db` handle, or `None` before `history.db` is open / after it closes.
    history: Option<History>,
    /// Optional custom `stderr` sink for deterministic testing.
    stderr_sink: Option<StderrSink>,
}

/// A `tracing_subscriber::Layer` that records structured [`LogEvent`] events into `history.db`
/// or to `stderr` before `history.db` is open or when `history.db` cannot be written.
#[derive(Clone)]
pub struct HistoryLayer {
    /// Shared configuration and target sink.
    state: Arc<Mutex<LayerState>>,
}

impl fmt::Debug for HistoryLayer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let level = locked(&self.state).level;
        f.debug_struct("HistoryLayer").field("level", &level).finish()
    }
}

impl HistoryLayer {
    /// Creates a layer with `level`, `clock`, `secrets`, and an optional open `history`.
    #[must_use]
    pub fn new(level: LogLevel, clock: SharedClock, secrets: Secrets, history: Option<History>) -> Self {
        let redactor = Redactor::new(secrets);
        if let Some(ref handle) = history {
            handle.configure_log_fallback(redactor.clone(), None);
        }
        drop(ensure_global_subscriber());
        ensure_thread_span_subscriber();
        Self {
            state: Arc::new(Mutex::new(LayerState {
                level,
                clock,
                redactor,
                history,
                stderr_sink: None,
            })),
        }
    }

    /// Creates a layer whose `history.db` handle will be attached once opened.
    #[must_use]
    pub fn deferred(level: LogLevel, clock: SharedClock, secrets: Secrets) -> Self {
        Self::new(level, clock, secrets, None)
    }

    /// Installs a global [`HistoryLayer`] and panic hook for the daemon process.
    #[must_use]
    pub fn install_global() -> Self {
        GLOBAL_STDERR_ENABLED.store(true, Ordering::Relaxed);
        let layer = ensure_global_subscriber();
        install_panic_hook(layer.clone());
        layer
    }

    /// Synchronizes the global [`HistoryLayer`], if one was installed at startup, with `self`.
    pub fn sync_global(&self) {
        let Some(global) = GLOBAL_LAYER.get() else {
            return;
        };
        if Arc::ptr_eq(&global.state, &self.state) {
            return;
        }
        let current = locked(&self.state);
        let mut target = locked(&global.state);
        target.level = current.level;
        target.clock = current.clock.clone();
        target.redactor = current.redactor.clone();
        target.history.clone_from(&current.history);
        target.stderr_sink.clone_from(&current.stderr_sink);
    }

    /// Updates the threshold level, clock, and secrets of this layer.
    pub fn configure(&self, level: LogLevel, clock: SharedClock, secrets: Secrets) {
        let redactor = Redactor::new(secrets);
        {
            let mut state = locked(&self.state);
            state.level = level;
            state.clock = clock;
            state.redactor = redactor.clone();
            let sink = state.stderr_sink.clone();
            if let Some(ref history) = state.history {
                history.configure_log_fallback(redactor, sink);
            }
        }
        self.sync_global();
    }

    /// Attaches the opened [`History`] handle so subsequent events are written to `history.db`.
    pub fn attach_history(&self, history: History) {
        {
            let mut state = locked(&self.state);
            history.configure_log_fallback(state.redactor.clone(), state.stderr_sink.clone());
            state.history = Some(history);
        }
        self.sync_global();
    }

    /// Detaches the [`History`] handle when the writer is closing so late events go to `stderr`.
    pub fn detach_history(&self) {
        locked(&self.state).history = None;
        self.sync_global();
    }

    /// Replaces the standard error fallback sink (used by tests to inspect `stderr` lines deterministically).
    pub fn with_stderr_sink(&self, sink: StderrSink) {
        {
            let mut state = locked(&self.state);
            if let Some(ref history) = state.history {
                history.configure_log_fallback(state.redactor.clone(), Some(Arc::clone(&sink)));
            }
            state.stderr_sink = Some(sink);
        }
        self.sync_global();
    }

    /// Records a [`LogEvent::DaemonPanicked`] entry from `info`.
    pub fn record_panic_info(&self, info: &PanicHookInfo<'_>) {
        let message = panic_payload_message(info);
        let location = info.location().map_or_else(
            || "unknown".to_owned(),
            |loc| format!("{}:{}:{}", loc.file(), loc.line(), loc.column()),
        );
        let thread = std::thread::current().name().unwrap_or("unnamed").to_owned();
        let mut scope = SpanScope::default();
        ENTERED_SPANS.with(|stack| {
            let stack = stack.borrow();
            if !stack.is_empty() {
                let scopes = locked(&SPAN_SCOPES);
                for raw in stack.iter() {
                    if let Some(ext) = scopes.get(raw) {
                        scope.merge_from(ext);
                    }
                }
            }
        });
        let mut fields = BTreeMap::new();
        fields.insert("location".to_owned(), Value::String(location));
        fields.insert("message".to_owned(), Value::String(message.clone()));
        fields.insert("thread".to_owned(), Value::String(thread));
        self.dispatch_record("panic", LogEvent::DaemonPanicked, scope, message, fields);
    }

    /// Builds, redacts, and routes a [`LogRecord`] to `history.db` or `stderr`.
    fn dispatch_record(
        &self,
        module: &str,
        event: LogEvent,
        scope: SpanScope,
        message: String,
        fields: BTreeMap<String, Value>,
    ) {
        let (clock, redactor, history, stderr_sink) = {
            let state = locked(&self.state);
            if !state.level.keeps(event.level()) {
                return;
            }
            (
                state.clock.clone(),
                state.redactor.clone(),
                state.history.clone(),
                state.stderr_sink.clone(),
            )
        };
        let Ok(at) = Timestamp::now(&clock) else {
            return;
        };
        let team = scope.team.map(|team_id| LogTeamScope::new(team_id, scope.rounds));
        let task = scope.task.map(|task_id| LogTaskScope::new(task_id, scope.run));
        let mut record = LogRecord {
            at,
            module: module.to_owned(),
            event,
            team,
            task,
            message,
            fields,
            cut: Cut::default(),
        };
        if !redactor.log_record(&mut record) {
            return;
        }
        if stderr_sink.is_some() || history.is_none() || GLOBAL_STDERR_ENABLED.load(Ordering::Relaxed) {
            write_stderr_line(stderr_sink.as_ref(), &format_stderr_line(&record));
        }
        if let Some(history) = history {
            history.record_log(record);
        }
    }

    /// Writes a redacted diagnostic line to `stderr` when `TracingReporter` reports lost domain history records.
    fn write_plain_stderr(&self, text: &str) {
        let (redactor, stderr_sink) = {
            let state = locked(&self.state);
            (state.redactor.clone(), state.stderr_sink.clone())
        };
        let redacted = redactor.text(text);
        write_stderr_line(stderr_sink.as_ref(), &redacted);
    }
}

/// Formats a redacted [`LogRecord`] as a single fallback line for `stderr`.
#[must_use]
fn format_stderr_line(record: &LogRecord) -> String {
    let fields = serde_json::to_string(&record.fields).unwrap_or_else(|_| "{}".to_owned());
    format!(
        "{} {} {} event={} message={:?} fields={fields}",
        record.at,
        record.level(),
        record.module,
        record.event,
        record.message
    )
}

/// Writes `line` to `sink` if present, or to `std::io::stderr()`.
fn write_stderr_line(sink: Option<&StderrSink>, line: &str) {
    if let Some(sink) = sink {
        sink(line);
    } else if std::io::Write::write_all(&mut std::io::stderr(), format!("{line}\n").as_bytes()).is_err() {
        // Best-effort stderr write.
    }
}

/// Extracts a human-readable string from a panic payload.
fn panic_payload_message(info: &PanicHookInfo<'_>) -> String {
    if let Some(text) = info.payload().downcast_ref::<&str>() {
        (*text).to_owned()
    } else if let Some(text) = info.payload().downcast_ref::<String>() {
        text.clone()
    } else {
        "panic payload is not a string".to_owned()
    }
}

/// Maps a `tracing::Level` to [`LogLevel`].
const fn level_from_tracing(level: Level) -> LogLevel {
    match level {
        Level::ERROR => LogLevel::Error,
        Level::WARN => LogLevel::Warn,
        Level::INFO => LogLevel::Info,
        Level::DEBUG | Level::TRACE => LogLevel::Debug,
    }
}

/// Scope identifiers collected from enclosing spans (`team`, `rounds`, `task`, `run`).
#[derive(Debug, Clone, Default)]
struct SpanScope {
    /// Enclosing team ID, if any.
    team: Option<TeamId>,
    /// Enclosing round IDs, if any.
    rounds: Vec<RoundId>,
    /// Enclosing task ID, if any.
    task: Option<TaskId>,
    /// Enclosing run ID, if any.
    run: Option<RunId>,
}

impl SpanScope {
    /// Merges identifiers from a child span or event into `self`.
    fn merge_from(&mut self, child: &Self) {
        if child.team.is_some() {
            self.team.clone_from(&child.team);
        }
        for round in &child.rounds {
            if !self.rounds.contains(round) {
                self.rounds.push(round.clone());
            }
        }
        if child.task.is_some() {
            self.task.clone_from(&child.task);
        }
        if child.run.is_some() {
            self.run.clone_from(&child.run);
        }
    }
}

/// Visitor that extracts `team`, `round`, `rounds`, `task`, and `run` from a span's attributes.
struct SpanVisitor<'a>(&'a mut SpanScope);

impl SpanVisitor<'_> {
    /// Parses a scope identifier if `field` is one of the scope keys. Returns `true` if `field` was a scope key.
    fn record_scope_field(&mut self, field: &Field, raw: &str) -> bool {
        let clean = raw.trim().trim_matches('"');
        match field.name() {
            "team" => {
                if let Ok(team) = clean.parse::<TeamId>() {
                    self.0.team = Some(team);
                }
                true
            }
            "round" => {
                if let Ok(round) = clean.parse::<RoundId>()
                    && !self.0.rounds.contains(&round)
                {
                    self.0.rounds.push(round);
                }
                true
            }
            "rounds" => {
                for part in clean.split(',').map(str::trim).filter(|part| !part.is_empty()) {
                    if let Ok(round) = part.parse::<RoundId>()
                        && !self.0.rounds.contains(&round)
                    {
                        self.0.rounds.push(round);
                    }
                }
                true
            }
            "task" => {
                if let Ok(task) = clean.parse::<TaskId>() {
                    self.0.task = Some(task);
                }
                true
            }
            "run" => {
                if let Ok(run) = clean.parse::<RunId>() {
                    self.0.run = Some(run);
                }
                true
            }
            _ => false,
        }
    }
}

impl Visit for SpanVisitor<'_> {
    fn record_str(&mut self, field: &Field, value: &str) {
        self.record_scope_field(field, value);
    }

    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        self.record_scope_field(field, &format!("{value:?}"));
    }
}

/// Visitor that extracts `event`, `message`, scope IDs, and structured JSON fields from a `tracing::Event`.
#[derive(Default)]
struct EventVisitor {
    /// Value of the `event` field, if present.
    event_name: Option<String>,
    /// Human-readable message of the event.
    message: String,
    /// Scope identifiers passed directly on the event.
    scope: SpanScope,
    /// Remaining structured fields of the event.
    fields: BTreeMap<String, Value>,
}

impl EventVisitor {
    /// Records a string or formatted value for `field`.
    fn record_text(&mut self, field: &Field, text: &str) {
        let clean = text.trim().trim_matches('"');
        match field.name() {
            "event" => self.event_name = Some(clean.to_owned()),
            "message" => clean.clone_into(&mut self.message),
            name if SpanVisitor(&mut self.scope).record_scope_field(field, clean) => {
                let _ = name;
            }
            "input" | "result" => {
                let parsed = serde_json::from_str::<Value>(clean).unwrap_or_else(|_| Value::String(clean.to_owned()));
                self.fields.insert(field.name().to_owned(), parsed);
            }
            name => {
                self.fields.insert(name.to_owned(), Value::String(clean.to_owned()));
            }
        }
    }
}

impl Visit for EventVisitor {
    fn record_i64(&mut self, field: &Field, value: i64) {
        self.fields.insert(field.name().to_owned(), Value::from(value));
    }

    fn record_u64(&mut self, field: &Field, value: u64) {
        self.fields.insert(field.name().to_owned(), Value::from(value));
    }

    fn record_f64(&mut self, field: &Field, value: f64) {
        self.fields.insert(field.name().to_owned(), Value::from(value));
    }

    fn record_bool(&mut self, field: &Field, value: bool) {
        self.fields.insert(field.name().to_owned(), Value::from(value));
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        self.record_text(field, value);
    }

    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        self.record_text(field, &format!("{value:?}"));
    }
}

impl<S> Layer<S> for HistoryLayer
where
    S: Subscriber + for<'a> LookupSpan<'a>,
{
    fn enabled(&self, metadata: &Metadata<'_>, _ctx: Context<'_, S>) -> bool {
        if metadata.is_span() {
            return true;
        }
        let level = level_from_tracing(*metadata.level());
        locked(&self.state).level.keeps(level)
    }

    fn on_new_span(&self, attrs: &Attributes<'_>, id: &Id, ctx: Context<'_, S>) {
        let mut scope = SpanScope::default();
        attrs.record(&mut SpanVisitor(&mut scope));
        let key = (Arc::as_ptr(&self.state) as usize, id.into_u64());
        locked(&SPAN_SCOPES).insert(key, scope.clone());
        if let Some(span) = ctx.span(id) {
            span.extensions_mut().insert(scope);
        }
    }

    fn on_record(&self, id: &Id, values: &SpanRecord<'_>, ctx: Context<'_, S>) {
        let key = (Arc::as_ptr(&self.state) as usize, id.into_u64());
        {
            let mut scopes = locked(&SPAN_SCOPES);
            let entry = scopes.entry(key).or_default();
            values.record(&mut SpanVisitor(entry));
        }
        let Some(span) = ctx.span(id) else {
            return;
        };
        let mut extensions = span.extensions_mut();
        if let Some(scope) = extensions.get_mut::<SpanScope>() {
            values.record(&mut SpanVisitor(scope));
        } else {
            let mut scope = SpanScope::default();
            values.record(&mut SpanVisitor(&mut scope));
            extensions.insert(scope);
        }
    }

    fn on_enter(&self, id: &Id, _ctx: Context<'_, S>) {
        let key = (Arc::as_ptr(&self.state) as usize, id.into_u64());
        ENTERED_SPANS.with(|stack| {
            let mut stack = stack.borrow_mut();
            if stack.last().copied() != Some(key) {
                stack.push(key);
            }
        });
    }

    fn on_exit(&self, id: &Id, _ctx: Context<'_, S>) {
        let key = (Arc::as_ptr(&self.state) as usize, id.into_u64());
        ENTERED_SPANS.with(|stack| {
            let mut stack = stack.borrow_mut();
            if let Some(pos) = stack.iter().rposition(|&item| item == key) {
                stack.remove(pos);
            }
        });
    }

    fn on_close(&self, id: Id, _ctx: Context<'_, S>) {
        let key = (Arc::as_ptr(&self.state) as usize, id.into_u64());
        locked(&SPAN_SCOPES).remove(&key);
    }

    fn on_event(&self, event: &Event<'_>, ctx: Context<'_, S>) {
        let mut scope = SpanScope::default();
        ENTERED_SPANS.with(|stack| {
            let stack = stack.borrow();
            if !stack.is_empty() {
                let scopes = locked(&SPAN_SCOPES);
                for raw in stack.iter() {
                    if let Some(ext) = scopes.get(raw) {
                        scope.merge_from(ext);
                    }
                }
            }
        });
        if let Some(span_scope) = ctx.event_scope(event) {
            for span in span_scope.from_root() {
                if let Some(ext) = span.extensions().get::<SpanScope>() {
                    scope.merge_from(ext);
                }
            }
        }
        let mut visitor = EventVisitor::default();
        event.record(&mut visitor);
        scope.merge_from(&visitor.scope);

        let Some(event_name) = visitor.event_name else {
            if visitor.message.starts_with("the history cannot keep some records") {
                let cause = visitor.fields.get("cause").and_then(Value::as_str).unwrap_or_default();
                let detail = visitor.fields.get("detail").and_then(Value::as_str).unwrap_or_default();
                self.write_plain_stderr(&format!("{} cause={cause} detail={detail}", visitor.message));
            }
            return;
        };
        let Ok(log_event) = event_name.parse::<LogEvent>() else {
            return;
        };
        let message = if visitor.message.is_empty() {
            log_event.as_str().to_owned()
        } else {
            visitor.message
        };
        self.dispatch_record(event.metadata().target(), log_event, scope, message, visitor.fields);
    }
}

/// Ensures the current thread has a default `Registry` + [`HistoryLayer`] subscriber so spans created before
/// `.with_subscriber(...)` still receive valid span IDs and record their attributes even when another test installed
/// a `fmt` global subscriber.
fn ensure_thread_span_subscriber() {
    THREAD_SPAN_GUARD.with(|slot| {
        let mut slot = slot.borrow_mut();
        if slot.is_none() {
            let silent_sink: StderrSink = Arc::new(|_| {});
            let span_layer = HistoryLayer {
                state: Arc::new(Mutex::new(LayerState {
                    level: LogLevel::Error,
                    clock: SharedClock::new(SystemClock),
                    redactor: Redactor::new(Secrets::default()),
                    history: None,
                    stderr_sink: Some(silent_sink),
                })),
            };
            let subscriber = tracing_subscriber::registry().with(span_layer);
            *slot = Some(tracing::subscriber::set_default(subscriber));
        }
    });
}

/// Ensures a global `Registry` + [`HistoryLayer`] subscriber is installed so spans created before `.with_subscriber(...)`
/// still receive valid span IDs and record their attributes.
fn ensure_global_subscriber() -> HistoryLayer {
    GLOBAL_LAYER
        .get_or_init(|| {
            let created = HistoryLayer {
                state: Arc::new(Mutex::new(LayerState {
                    level: LogLevel::Info,
                    clock: SharedClock::new(SystemClock),
                    redactor: Redactor::new(Secrets::default()),
                    history: None,
                    stderr_sink: None,
                })),
            };
            drop(tracing_subscriber::registry().with(created.clone()).try_init());
            created
        })
        .clone()
}

/// Installs the daemon panic hook that records [`LogEvent::DaemonPanicked`] with a redacted message
/// and source location through `layer`.
pub fn install_panic_hook(layer: HistoryLayer) {
    *locked(&PANIC_LAYER) = Some(layer);
    PANIC_HOOK_ONCE.call_once(|| {
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            let active = locked(&PANIC_LAYER).clone();
            if let Some(layer) = active {
                layer.record_panic_info(info);
            } else {
                previous(info);
            }
        }));
    });
}

/// Records a pure decision of `agent_core::decide` (`decision_taken`, `debug`) with its name, input, and result.
pub fn log_decision<I, R>(decision: &str, input: &I, result: &R)
where
    I: Serialize + ?Sized,
    R: Serialize + ?Sized,
{
    if !tracing::enabled!(Level::DEBUG) {
        return;
    }
    let input_json = serde_json::to_string(input).unwrap_or_else(|_| "\"<unserializable>\"".to_owned());
    let result_json = serde_json::to_string(result).unwrap_or_else(|_| "\"<unserializable>\"".to_owned());
    tracing::debug!(
        event = LogEvent::DecisionTaken.as_str(),
        decision = decision,
        input = %input_json,
        result = %result_json,
        "a decision is taken"
    );
}

/// Records a completed outside call (`outside_call_completed`, `debug`) to `git`, `agentbus`, `github`, or `gate`.
pub fn log_outside_call(tool: &str, operation: &str, duration: Duration, exit_code: Option<i32>, outcome: &str) {
    if !tracing::enabled!(Level::DEBUG) {
        return;
    }
    let duration_ms = u64::try_from(duration.as_millis()).unwrap_or(u64::MAX);
    if let Some(code) = exit_code {
        tracing::debug!(
            event = LogEvent::OutsideCallCompleted.as_str(),
            tool = tool,
            operation = operation,
            duration_ms = duration_ms,
            exit_code = i64::from(code),
            outcome = outcome,
            "an outside call completed"
        );
    } else {
        tracing::debug!(
            event = LogEvent::OutsideCallCompleted.as_str(),
            tool = tool,
            operation = operation,
            duration_ms = duration_ms,
            outcome = outcome,
            "an outside call completed"
        );
    }
}

#[cfg(test)]
mod tests;
