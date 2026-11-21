//! A step's line in the daemon log: what the agent did, and who ran it.

#[cfg(test)]
mod tests;

use crate::cli::ActivityEvent;
use crate::history::{RunStart, Step};
use crate::ids::AgentName;
use crate::protocol::Client;

/// Longest text a step's line shows, in characters; a longer one is cut and ends with [`CUT_MARK`].
const MAX_TEXT_CHARS: usize = 120;

/// The mark that ends a text the line cut.
const CUT_MARK: &str = "…";

/// The line of `step` that `agent log` prints as text: the time, the step as `kind: what` in its first line, cut,
/// and the name of its agent when more than one is followed. `run` names the step's agent and task when the history
/// knows its start.
#[must_use]
pub fn step_line(step: &Step, run: Option<&RunStart>, followed: &[AgentName]) -> String {
    let mut line = step.at.to_string();
    line.push(' ');
    if followed.len() > 1
        && let Some(name) = run.map(|run| run.agent.name.as_str())
    {
        line.push_str(&value(name));
        line.push(' ');
    }
    line.push_str(&text(&step.event));
    line
}

/// A step as `kind: what`; a message is its text alone, since that is what its kind means here.
fn text(event: &ActivityEvent) -> String {
    let (kind, text) = match event {
        ActivityEvent::Text(text) => ("", text.clone()),
        ActivityEvent::Command(text) => ("command: ", text.clone()),
        ActivityEvent::FileEdit { path } => ("file edit: ", path.clone()),
        ActivityEvent::BusCall { tool, arguments } => ("bus call: ", format!("{tool} {arguments}")),
        ActivityEvent::ToolCall { name, summary } => ("tool call: ", format!("{name} {summary}")),
        ActivityEvent::Error(text) => ("error: ", text.clone()),
        ActivityEvent::Info(text) => ("info: ", text.clone()),
        ActivityEvent::Control { action, by } => ("control: ", format!("{action} {}", client(by.clone()))),
    };
    format!("{kind}{}", value(&text))
}

/// Who used a control, in words.
fn client(client: Client) -> String {
    match client {
        Client::Cli => "from the terminal".to_owned(),
        Client::Dashboard { login } => format!("through the dashboard as {}", value(login.as_str())),
    }
}

/// `text` on one line, cut after [`MAX_TEXT_CHARS`] characters with [`CUT_MARK`] when it was longer or had more lines.
fn value(text: &str) -> String {
    let (first, rest) = match text.split_once('\n') {
        Some((first, rest)) => (first, rest),
        None => (text, ""),
    };
    let first = first.strip_suffix('\r').unwrap_or(first);
    let mut shown: String = first.chars().take(MAX_TEXT_CHARS).collect();
    if !rest.is_empty() || first.chars().count() > MAX_TEXT_CHARS {
        shown.push_str(CUT_MARK);
    }
    shown
}
