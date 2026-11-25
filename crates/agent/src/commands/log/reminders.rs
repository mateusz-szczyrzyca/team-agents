//! `agent log --reminders`: how the reminders of the harness did, agent by agent, from the log of the history.

use std::io::Write;
use std::path::Path;

use agent_core::decide::reminders::{ReminderLine, reminder_table};
use agent_core::state::Timestamp;
use agent_runtime::history::Reader;

use super::LogError;

/// The columns of the table, left to right.
const HEADER: [&str; 7] = ["agent", "model", "commit", "context", "followed", "median", "cuts"];

/// What the table prints for a missing model or median.
const NONE: &str = "-";

/// The gap between two columns.
const GAP: &str = "  ";

/// Prints the table of the reminders of the last `days` days before `now`, read from the history in `history`.
///
/// # Errors
/// [`LogError::Read`] when the history cannot be read, [`LogError::Output`] when `out` refuses the text.
pub(crate) fn print_reminders(history: &Path, days: u32, now: Timestamp, out: &mut impl Write) -> Result<(), LogError> {
    let rows = Reader::open(history)?.reminder_rows()?;
    let text = render(&reminder_table(&rows, now, days), days);
    out.write_all(text.as_bytes()).map_err(LogError::Output)?;
    out.flush().map_err(LogError::Output)
}

/// The text of the table `lines` of the last `days` days: a header and a line for each agent and model, or one
/// sentence when there is none. Every column is as wide as its widest cell; the last one is not padded.
pub(crate) fn render(lines: &[ReminderLine], days: u32) -> String {
    if lines.is_empty() {
        let unit = if days == 1 { "day" } else { "days" };
        return format!("no reminders in the last {days} {unit}\n");
    }
    let mut cells: Vec<[String; 7]> = vec![HEADER.map(str::to_owned)];
    cells.extend(lines.iter().map(cells_of));
    let widths: [usize; 7] = std::array::from_fn(|column| {
        cells
            .iter()
            .filter_map(|row| row.get(column))
            .map(|cell| cell.chars().count())
            .max()
            .unwrap_or(0)
    });
    let mut text = String::new();
    for row in &cells {
        let last = row.len().saturating_sub(1);
        for (column, (cell, width)) in row.iter().zip(widths).enumerate() {
            text.push_str(cell);
            if column != last {
                text.extend(std::iter::repeat_n(' ', width.saturating_sub(cell.chars().count())));
                text.push_str(GAP);
            }
        }
        text.push('\n');
    }
    text
}

/// The cells of the line `line`, in the order of [`HEADER`].
fn cells_of(line: &ReminderLine) -> [String; 7] {
    [
        line.agent.to_string(),
        line.model.clone().unwrap_or_else(|| NONE.to_owned()),
        line.commit.to_string(),
        line.context.to_string(),
        line.followed.to_string(),
        line.median_minutes
            .map_or_else(|| NONE.to_owned(), |minutes| format!("{minutes} min")),
        line.cuts.to_string(),
    ]
}

#[cfg(test)]
mod tests;
