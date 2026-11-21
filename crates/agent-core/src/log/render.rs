//! A record's line in the daemon log: the event in words, the task, the agent, the run and the fields.
//!
//! The line is a pure function of a stored row and of its run; nothing here reads a clock, a file or the state.

pub mod steps;
#[cfg(test)]
mod tests;

use crate::history::RunStart;
use crate::local_time::Zone;
use crate::log::{LogEntry, LogEvent};

/// Longest value a line shows, in characters; a longer one is cut and ends with [`CUT_MARK`].
const MAX_VALUE_CHARS: usize = 120;

/// The mark that ends a value the line cut.
const CUT_MARK: &str = "…";

/// The line of `entry` that `agent log` prints as text, with `subject`, the start of the entry's run, when known: its
/// time in the RFC 3339 form of the files, as `agent log --utc` prints it.
#[must_use]
pub fn record_line(entry: &LogEntry, subject: Option<&RunStart>) -> String {
    record_line_in(entry, subject, &Zone::Utc)
}

/// The line of `entry` as [`record_line`] writes it, with its time and the times in its words in `zone`.
#[must_use]
pub fn record_line_in(entry: &LogEntry, subject: Option<&RunStart>, zone: &Zone) -> String {
    let mut line = format!(
        "{} {} {} {}",
        zone.stamp(entry.at()),
        entry.level(),
        entry.event(),
        scope(entry, subject)
    );
    line.push_str(": ");
    line.push_str(&words(entry, subject, zone));
    let fields = fields(entry);
    if !fields.is_empty() {
        line.push(' ');
        line.push_str(&fields);
    }
    line
}

/// Who the entry is about and where: the task, the agent's name, the team and rounds, and the run.
fn scope(entry: &LogEntry, subject: Option<&RunStart>) -> String {
    let task = entry.task().map(ToString::to_string);
    let agent = agent(entry, subject);
    let mut parts: Vec<String> = Vec::new();
    match (task, agent) {
        (Some(task), Some(agent)) => parts.push(format!("{task} ({agent})")),
        (Some(task), None) => parts.push(task),
        (None, Some(agent)) => parts.push(agent),
        (None, None) => {}
    }
    if let Some(team) = entry.team() {
        let rounds: Vec<&str> = entry.rounds().iter().map(crate::ids::RoundId::as_str).collect();
        if rounds.is_empty() {
            parts.push(format!("team {team}"));
        } else {
            parts.push(format!("team {team} rounds {}", rounds.join(",")));
        }
    }
    if let Some(run) = entry.run().or_else(|| subject.map(|run| &run.run)) {
        parts.push(format!("run {run}"));
    }
    parts.join(" ")
}

/// The agent's name: its own field of the event, else the agent of the run the entry belongs to.
fn agent(entry: &LogEntry, subject: Option<&RunStart>) -> Option<String> {
    match text(entry.fields().get("agent")) {
        Some(own) => Some(own),
        None => subject.map(|run| run.agent.name.clone()),
    }
}

/// The fields of `entry` as `key=value`, except the ones the words of its event already use.
fn fields(entry: &LogEntry) -> String {
    entry
        .fields()
        .iter()
        .filter(|(key, _)| !in_words(entry.event(), key))
        .map(|(key, field)| format!("{key}={}", field_value(field)))
        .collect::<Vec<_>>()
        .join(" ")
}

/// Whether the words of `event` already carry the field `key`, so that its `key=value` would repeat it.
fn in_words(event: LogEvent, key: &str) -> bool {
    let used: &[&str] = match event {
        LogEvent::TaskStatusChanged => &["agent", "from", "to"],
        LogEvent::ItemPhaseChanged => &["agent", "from", "item", "phase", "pr"],
        LogEvent::RunStarted => &["agent", "from", "why"],
        LogEvent::RunEnded => &["agent", "end"],
        _ => &["agent"],
    };
    used.contains(&key)
}

/// A field's value in one short line: a string as it is, another value as JSON.
fn field_value(field: &serde_json::Value) -> String {
    match field {
        serde_json::Value::String(text) => value(text),
        other => value(&other.to_string()),
    }
}

/// `field` as its text, when it is a string.
fn text(field: Option<&serde_json::Value>) -> Option<String> {
    field.and_then(serde_json::Value::as_str).map(ToOwned::to_owned)
}

/// What happened, in words for the common events and in the event's own message otherwise.
fn words(entry: &LogEntry, subject: Option<&RunStart>, zone: &Zone) -> String {
    match entry.event() {
        LogEvent::TaskStatusChanged => match (field(entry, "from"), field(entry, "to")) {
            (Some(from), Some(to)) => format!("{to} ← {from}"),
            _ => value(entry.message()),
        },
        LogEvent::ItemPhaseChanged => words_item(entry),
        LogEvent::RunStarted => words_run_started(entry, subject),
        LogEvent::RunEnded => match entry.fields().get("end") {
            Some(end) => format!("ended {}", words_end(end, zone)),
            None => value(entry.message()),
        },
        _ => value(entry.message()),
    }
}

/// The text of the field `key` of `entry`, when it is a string.
fn field<'a>(entry: &'a LogEntry, key: &str) -> Option<&'a str> {
    entry.fields().get(key).and_then(|value| value.as_str())
}

/// An item's phase change: what happened to which item, the pull request, and the phase it left.
fn words_item(entry: &LogEntry) -> String {
    let (Some(item), Some(phase)) = (field(entry, "item"), field(entry, "phase")) else {
        return value(entry.message());
    };
    let mut text = format!("item {} {phase}", value(item));
    if let Some(from) = field(entry, "from") {
        text.push_str(" ← ");
        text.push_str(&value(from));
    }
    text.push(' ');
    text.push_str(&reason(entry.message()));
    match field(entry, "pr") {
        Some(pr) => format!("{text}, pr={}", value(pr)),
        None => text,
    }
}

/// A run's start and who started it: the queue, or the bus's message from an agent.
fn words_run_started(entry: &LogEntry, subject: Option<&RunStart>) -> String {
    if let Some("wake_up") = field(entry, "why") {
        return match field(entry, "from") {
            Some(from) => format!("the bus woke the agent for a message from {}", value(from)),
            None => "the bus woke the agent for a message".to_owned(),
        };
    }
    let agent = agent(entry, subject).unwrap_or_default();
    format!("run started by {agent}")
}

/// How a run ended, from the `Debug` form of its end that the daemon records.
fn words_end(end: &serde_json::Value, zone: &Zone) -> String {
    let Some(text) = end.as_str() else {
        return value(&end.to_string());
    };
    match text.split([' ', '{']).next().unwrap_or_default() {
        "Reported" => format!("reported {}", keyword(&text_after(text, "outcome: "))),
        "NoStatus" => match text.split_once("exit_code: Some(") {
            Some((_, code)) => format!("no STATUS line (exit code {})", text_before(code)),
            None => "no STATUS line (ended by a signal)".to_owned(),
        },
        "QuotaExhausted" => match text.split_once("resets_at: Some(") {
            Some((_, at)) => format!("quota exhausted (back at {})", back_at(at, zone)),
            None => "quota exhausted".to_owned(),
        },
        "RateLimited" => "rate limited".to_owned(),
        "Hung" => "hung: stopped by the watchdog".to_owned(),
        "TurnLimit" => "turn limit reached".to_owned(),
        "Stopped" => "stopped by the human".to_owned(),
        _ => value(text),
    }
}

/// The time a quota is back at, from the text after `resets_at: Some(`, in `zone` when it is a time of the files or its
/// `Debug` form, else as it was written.
fn back_at(rest: &str, zone: &Zone) -> String {
    let inner = rest.trim_start().strip_prefix("Timestamp(").unwrap_or(rest);
    let raw = text_before(inner);
    let cleaned = raw
        .strip_suffix(" +00:00:00")
        .or_else(|| raw.strip_suffix(" +00:00"))
        .or_else(|| raw.strip_suffix(" +00"))
        .or_else(|| raw.strip_suffix(" UTC"))
        .unwrap_or(&raw);
    match Zone::Utc.parse_typed(cleaned) {
        Ok(at) => zone.stamp(at),
        Err(_) => raw,
    }
}

/// The text after `marker` in `text`, up to the first `,` or `}`.
fn text_after(text: &str, marker: &str) -> String {
    let rest = text.split_once(marker).map_or(text, |(_, rest)| rest);
    text_before(rest)
}

/// The text of `text` before its first `,` or `}`, trimmed.
fn text_before(text: &str) -> String {
    text.split([',', ')', '}']).next().unwrap_or_default().trim().to_owned()
}

/// `text` of a `Debug` form of an enum variant as its `Display` writes it: `Done` is `DONE` on a `STATUS:` line.
fn keyword(text: &str) -> String {
    match text {
        "Done" => "DONE".to_owned(),
        "Questions" => "QUESTIONS".to_owned(),
        "Failed" => "FAILED".to_owned(),
        "Sleeping" => "SLEEPING".to_owned(),
        "Excluded" => "EXCLUDED".to_owned(),
        other => other.to_owned(),
    }
}

/// The reason of a phase change from the daemon's message: what the subject line says of the item, without its
/// subject: "after a review verdict" for "an item changed phase after a review verdict".
fn reason(message: &str) -> String {
    let first = message.split('\n').next().unwrap_or_default();
    ["an item changed phase ", "the item ", "an item "]
        .iter()
        .find_map(|subject| first.strip_prefix(subject))
        .unwrap_or(first)
        .to_owned()
}

/// `text` on one line, cut after [`MAX_VALUE_CHARS`] characters with [`CUT_MARK`] when it was longer or had more lines.
fn value(text: &str) -> String {
    let (first, rest) = match text.split_once('\n') {
        Some((first, rest)) => (first, rest),
        None => (text, ""),
    };
    let first = first.strip_suffix('\r').unwrap_or(first);
    let mut shown: String = first.chars().take(MAX_VALUE_CHARS).collect();
    if !rest.is_empty() || first.chars().count() > MAX_VALUE_CHARS {
        shown.push_str(CUT_MARK);
    }
    shown
}
