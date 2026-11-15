//! The `tracing_subscriber::Layer` that turns daemon log events and their enclosing spans into rows of `history.db`
//! (or redacted lines on `stderr` before `history.db` is open or when it cannot be written), the panic hook, and the
//! `debug` helpers for pure decisions and outside tool calls.

use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};
use std::fmt;
use std::panic::PanicHookInfo;
use std::sync::{Arc, Mutex, MutexGuard, Once, PoisonError};
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

use super::handle::History;
use super::redact::Redactor;
use crate::clock::{SharedClock, SystemClock};

/// Callback invoked for lines written to the standard error stream fallback.
pub type StderrSink = Arc<dyn Fn(&str) + Send + Sync>;

thread_local! {
    /// Stack of currently entered `(layer_id, span_id)` pairs on the current thread, outermost first. It is used only
    /// by the panic hook, which runs outside the subscriber's event callback and therefore has no span context.
    static ENTERED_SPANS: RefCell<Vec<(usize, u64)>> = const { RefCell::new(Vec::new()) };
}

/// Active [`HistoryLayer`] used by the process panic hook.
static PANIC_LAYER: Mutex<Option<HistoryLayer>> = Mutex::new(None);

/// Ensures `std::panic::set_hook` is installed at most once per process.
static PANIC_HOOK_ONCE: Once = Once::new();

/// Subscriber registered in `tracing_core`'s global dispatcher list so that `tracing_core` never takes its
/// single-dispatcher fast path (`Rebuilder::JustOne`) and caches `Interest::never()` from a thread running without a
/// subscriber while another thread uses `WithSubscriber`.
struct ScopedCallsiteGuard;

impl Subscriber for ScopedCallsiteGuard {
    fn register_callsite(&self, _metadata: &'static Metadata<'static>) -> tracing::subscriber::Interest {
        tracing::subscriber::Interest::sometimes()
    }

    fn enabled(&self, _metadata: &Metadata<'_>) -> bool {
        false
    }

    fn max_level_hint(&self) -> Option<tracing::level_filters::LevelFilter> {
        Some(tracing::level_filters::LevelFilter::TRACE)
    }

    fn new_span(&self, _span: &Attributes<'_>) -> Id {
        Id::from_u64(1)
    }

    fn record(&self, _span: &Id, _values: &SpanRecord<'_>) {}

    fn record_follows_from(&self, _span: &Id, _follows: &Id) {}

    fn event(&self, _event: &Event<'_>) {}

    fn enter(&self, _span: &Id) {}

    fn exit(&self, _span: &Id) {}
}

/// Holds two live `Dispatch` instances so `tracing_core`'s `LOCKED_DISPATCHERS` always has more than one entry and
/// combines callsite interest to `Interest::sometimes()`.
static SCOPED_CALLSITE_GUARDS: std::sync::LazyLock<(tracing::Dispatch, tracing::Dispatch)> =
    std::sync::LazyLock::new(|| {
        let first = tracing::Dispatch::new(ScopedCallsiteGuard);
        let second = tracing::Dispatch::new(ScopedCallsiteGuard);
        tracing::callsite::rebuild_interest_cache();
        (first, second)
    });

/// Ensures `tracing` macro callsites evaluate against the current thread's scoped subscriber instead of caching
/// `Interest::never()` from a concurrent thread with `NoSubscriber`.
pub fn ensure_scoped_callsites() {
    let _ = &*SCOPED_CALLSITE_GUARDS;
}

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
    /// Whether the daemon writes every retained record to `stderr` as well as to its open history database.
    dual_stderr: bool,
    /// Span scopes owned by this layer's subscriber, keyed by the subscriber-local span ID.
    spans: HashMap<u64, SpanScope>,
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
        ensure_scoped_callsites();
        let redactor = Redactor::new(secrets);
        if let Some(ref handle) = history {
            handle.configure_log_fallback(redactor.clone(), None);
        }
        Self {
            state: Arc::new(Mutex::new(LayerState {
                level,
                clock,
                redactor,
                history,
                stderr_sink: None,
                dual_stderr: false,
                spans: HashMap::new(),
            })),
        }
    }

    /// Creates a layer whose `history.db` handle will be attached once opened.
    #[must_use]
    pub fn deferred(level: LogLevel, clock: SharedClock, secrets: Secrets) -> Self {
        Self::new(level, clock, secrets, None)
    }

    /// Explicitly installs a global subscriber and panic hook for the daemon process.
    ///
    /// Construction itself has no subscriber side effects. The caller handles an already-installed global subscriber.
    ///
    /// # Errors
    /// [`tracing::subscriber::SetGlobalDefaultError`]: another global subscriber is already installed.
    pub fn install_global() -> Result<Self, tracing::subscriber::SetGlobalDefaultError> {
        let layer = Self::deferred(LogLevel::Info, SharedClock::new(SystemClock), Secrets::default());
        locked(&layer.state).dual_stderr = true;
        tracing::subscriber::set_global_default(tracing_subscriber::registry().with(layer.clone()))?;
        install_panic_hook(layer.clone());
        Ok(layer)
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
    }

    /// Attaches the opened [`History`] handle so subsequent events are written to `history.db`.
    pub fn attach_history(&self, history: History) {
        {
            let mut state = locked(&self.state);
            history.configure_log_fallback(state.redactor.clone(), state.stderr_sink.clone());
            state.history = Some(history);
        }
    }

    /// Detaches the [`History`] handle when the writer is closing so late events go to `stderr`.
    pub fn detach_history(&self) {
        locked(&self.state).history = None;
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
            let layer_id = Arc::as_ptr(&self.state) as usize;
            let state = locked(&self.state);
            for (entered_layer, span) in stack.borrow().iter() {
                if *entered_layer == layer_id
                    && let Some(ext) = state.spans.get(span)
                {
                    scope.merge_from(ext);
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
        let (clock, redactor, history, stderr_sink, dual_stderr) = {
            let state = locked(&self.state);
            if !state.level.keeps(event.level()) {
                return;
            }
            (
                state.clock.clone(),
                state.redactor.clone(),
                state.history.clone(),
                state.stderr_sink.clone(),
                state.dual_stderr,
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
        if stderr_sink.is_some() || history.is_none() || dual_stderr {
            write_stderr_line(stderr_sink.as_ref(), &format_stderr_line(&record));
        }
        if let Some(history) = history {
            history.record_log(record);
        }
    }

    /// Keeps diagnostics outside the fixed event vocabulary visible on `stderr` without inventing history rows.
    fn write_unrecognized_event(&self, metadata: &Metadata<'_>, name: Option<&str>, visitor: &EventVisitor) {
        let fields = serde_json::to_string(&visitor.fields).unwrap_or_else(|_| "{}".to_owned());
        let name = name.map_or_else(String::new, |name| format!(" event={name}"));
        self.write_plain_stderr(&format!(
            "{} {}{name} message={:?} fields={fields}",
            level_from_tracing(*metadata.level()),
            metadata.target(),
            visitor.message
        ));
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
    let mut scope = Vec::new();
    if let Some(team) = &record.team {
        scope.push(format!("team={}", team.team()));
        if !team.rounds().is_empty() {
            scope.push(format!(
                "rounds={}",
                team.rounds().iter().map(RoundId::as_str).collect::<Vec<_>>().join(",")
            ));
        }
    }
    if let Some(task) = &record.task {
        scope.push(format!("task={}", task.task()));
        if let Some(run) = task.run() {
            scope.push(format!("run={run}"));
        }
    }
    let scope = if scope.is_empty() {
        String::new()
    } else {
        format!(" {}", scope.join(" "))
    };
    format!(
        "{} {} {} event={}{scope} message={:?} fields={fields}",
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
            "message" => text.clone_into(&mut self.message),
            name if SpanVisitor(&mut self.scope).record_scope_field(field, clean) => {
                let _ = name;
            }
            "input" | "result" => {
                let parsed = serde_json::from_str::<Value>(clean).unwrap_or_else(|_| Value::String(text.to_owned()));
                self.fields.insert(field.name().to_owned(), parsed);
            }
            name => {
                // Keep the original string intact: trimming it before `Redactor` sees it can turn a configured secret
                // with quotes or surrounding whitespace into a different, unredacted value.
                self.fields.insert(name.to_owned(), Value::String(text.to_owned()));
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
        let state = locked(&self.state);
        let level = level_from_tracing(*metadata.level());
        state.level.keeps(level)
    }

    fn on_new_span(&self, attrs: &Attributes<'_>, id: &Id, ctx: Context<'_, S>) {
        let mut scope = SpanScope::default();
        attrs.record(&mut SpanVisitor(&mut scope));
        locked(&self.state).spans.insert(id.into_u64(), scope.clone());
        if let Some(span) = ctx.span(id) {
            span.extensions_mut().insert(scope);
        }
    }

    fn on_record(&self, id: &Id, values: &SpanRecord<'_>, ctx: Context<'_, S>) {
        {
            let mut state = locked(&self.state);
            values.record(&mut SpanVisitor(state.spans.entry(id.into_u64()).or_default()));
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
        locked(&self.state).spans.remove(&id.into_u64());
    }

    fn on_event(&self, event: &Event<'_>, ctx: Context<'_, S>) {
        let level = level_from_tracing(*event.metadata().level());
        if !locked(&self.state).level.keeps(level) {
            return;
        }
        let mut scope = SpanScope::default();
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

        let Some(event_name) = visitor.event_name.clone() else {
            self.write_unrecognized_event(event.metadata(), None, &visitor);
            return;
        };
        let Ok(log_event) = event_name.parse::<LogEvent>() else {
            self.write_unrecognized_event(event.metadata(), Some(&event_name), &visitor);
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

/// The callsite for directly dispatched outside-call events.
static OUTSIDE_CALL_CALLSITE: tracing::callsite::DefaultCallsite =
    tracing::callsite::DefaultCallsite::new(&OUTSIDE_CALL_METADATA);

/// The stable metadata and public fields of an outside-call event.
static OUTSIDE_CALL_METADATA: Metadata<'static> = Metadata::new(
    "outside_call_completed",
    module_path!(),
    Level::DEBUG,
    Some(file!()),
    Some(line!()),
    Some(module_path!()),
    tracing::field::FieldSet::new(
        &[
            "event",
            "tool",
            "operation",
            "duration_ms",
            "exit_code",
            "outcome",
            "message",
        ],
        tracing::callsite::Identifier(&OUTSIDE_CALL_CALLSITE),
    ),
    tracing::metadata::Kind::EVENT,
);

/// Records a completed outside call (`outside_call_completed`, `debug`) to `git`, `agentbus`, `github`, or `gate`.
pub fn log_outside_call(tool: &str, operation: &str, duration: Duration, exit_code: Option<i32>, outcome: &str) {
    let duration_ms = u64::try_from(duration.as_millis()).unwrap_or(u64::MAX);
    let exit_code = exit_code.map(i64::from);
    let event = LogEvent::OutsideCallCompleted.as_str();
    let message = "an outside call completed";
    let fields = OUTSIDE_CALL_METADATA.fields();
    let field_list = fields.iter().collect::<Vec<_>>();
    let [
        event_field,
        tool_field,
        operation_field,
        duration_field,
        exit_field,
        outcome_field,
        message_field,
    ] = field_list.as_slice()
    else {
        // The static metadata and this destructuring must change together.
        return;
    };
    let entries = [
        (event_field, Some(&event as &dyn tracing::field::Value)),
        (tool_field, Some(&tool as &dyn tracing::field::Value)),
        (operation_field, Some(&operation as &dyn tracing::field::Value)),
        (duration_field, Some(&duration_ms as &dyn tracing::field::Value)),
        (exit_field, Some(&exit_code as &dyn tracing::field::Value)),
        (outcome_field, Some(&outcome as &dyn tracing::field::Value)),
        (message_field, Some(&message as &dyn tracing::field::Value)),
    ];
    let values = fields.value_set(&entries);
    // Direct dispatch still reaches the active subscriber, whose layer filters by level, but does not let another
    // scoped dispatcher's cached macro interest suppress this completion record.
    tracing::Event::dispatch(&OUTSIDE_CALL_METADATA, &values);
}

#[cfg(test)]
mod tests;
