//! Harness-owned description sections and the lead's editable contribution.

use super::PullTextError;
use agent_core::prompt::{AutopilotPrompt, Variables};
use agent_core::state::GateRecord;
use agent_runtime::autopilot_prompts::AutopilotPrompts;

/// The lead contribution lives in the task directory, outside the worktree.
pub(crate) const LEAD_DESCRIPTION_FILE: &str = "description.md";

/// Maximum UTF-8 contribution size in bytes, bounding daemon reads.
pub(crate) const MAX_DESCRIPTION_BYTES: usize = 1 << 20;

/// Removes harness-owned sections from the lead's contribution.
pub(crate) fn lead_sections(text: &str) -> String {
    without_sections(text, &["Size", "Gate", "Allowed beforehand", "Unpushed base"])
}

/// Excludes task permissions from marker justifications supplied by the lead or the remote description.
pub(crate) fn checks_description(text: &str) -> String {
    without_sections(text, &["Allowed beforehand"])
}

/// Removes only the named headings and their children, respecting fenced examples.
fn without_sections(text: &str, names: &[&str]) -> String {
    let mut kept = Vec::new();
    let mut omitted = None;
    let mut fence = None;
    for line in text.lines() {
        let trimmed = line.trim_start();
        if let Some((marker, length)) = fence {
            if trimmed.chars().take_while(|c| *c == marker).count() >= length {
                fence = None;
            }
        } else if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            if let Some(marker) = trimmed.chars().next() {
                fence = Some((marker, trimmed.chars().take_while(|c| *c == marker).count()));
            }
        } else {
            let level = trimmed.chars().take_while(|c| *c == '#').count();
            if (1..=6).contains(&level)
                && let Some(title) = trimmed.get(level..).and_then(|s| s.strip_prefix(' '))
            {
                if omitted.is_some_and(|depth| level <= depth) {
                    omitted = None;
                }
                let title = title.trim().trim_end_matches('#').trim();
                if names.iter().any(|name| title.eq_ignore_ascii_case(name)) {
                    omitted = Some(level);
                }
            }
        }
        if omitted.is_none() {
            kept.push(line);
        }
    }
    kept.join("\n").trim().to_owned()
}

/// Renders the measured size and recorded gate, including a preceding failure at this head.
pub(crate) fn measured_sections(
    texts: &AutopilotPrompts,
    size: usize,
    gate: &GateRecord,
    previous: Option<&GateRecord>,
) -> Result<String, PullTextError> {
    let size = texts.render(
        AutopilotPrompt::PrSize,
        &Variables::new().with("size", size.to_string())?,
    )?;
    let values = Variables::new()
        .with("head", gate.result.head.as_str())?
        .with("finished", gate.result.finished_at.to_string())?
        .with("result", gate.summary())?
        .with("log", gate.result.log.as_path().display().to_string())?;
    let mut sections = vec![size, texts.render(AutopilotPrompt::PrGate, &values)?];
    if gate.counts_as_pass() && previous.is_some_and(|old| old.result.head == gate.result.head && !old.counts_as_pass())
    {
        sections.push(texts.render(AutopilotPrompt::PrGateRetry, &Variables::new())?);
    }
    Ok(sections.iter().map(|s| s.trim_end()).collect::<Vec<_>>().join("\n\n"))
}

#[cfg(test)]
mod tests;

/// Lists changed paths covered by the task's permissions, using a harness-owned template.
///
/// # Errors
/// [`PullTextError`] if template rendering or variable construction fails.
pub(crate) fn allowed_section(
    texts: &AutopilotPrompts,
    files: &[agent_core::decide::checks::FileChange],
    allowances: &[agent_core::task_allowance::TaskAllowance],
) -> Result<Option<String>, PullTextError> {
    use agent_core::decide::checks::Change;
    let mut covered = std::collections::BTreeMap::new();
    for file in files {
        let paths = std::iter::once(&file.path).chain(match &file.change {
            Change::Renamed { from } => Some(from),
            Change::Added | Change::Modified | Change::Deleted => None,
        });
        for path in paths {
            if let Some(allowance) = allowances.iter().find(|line| line.matches(path)) {
                covered.insert(path, allowance);
            }
        }
    }
    if covered.is_empty() {
        return Ok(None);
    }
    let rows = covered
        .into_iter()
        .map(|(path, allowance)| format!("- `{path}` — `Allow: {}`", allowance.as_str()))
        .collect::<Vec<_>>()
        .join("\n");
    Ok(Some(texts.render(
        AutopilotPrompt::PrAllowedBeforehand,
        &Variables::new().with("paths", rows)?,
    )?))
}
