//! Harness-owned description sections and the lead's editable contribution.

use super::PullTextError;
use agent_core::ids::AgentName;
use agent_core::prompt::{AutopilotPrompt, Variables};
use agent_core::state::GateRecord;
use agent_runtime::agentbus::{DelegationStatus, TaskRequest};
use agent_runtime::autopilot_prompts::AutopilotPrompts;

/// The lead contribution lives in the task directory, outside the worktree.
pub(crate) const LEAD_DESCRIPTION_FILE: &str = "description.md";

/// Maximum UTF-8 contribution size in bytes, bounding daemon reads.
pub(crate) const MAX_DESCRIPTION_BYTES: usize = 1 << 20;

/// Removes harness-owned sections from the lead's contribution.
pub(crate) fn lead_sections(text: &str) -> String {
    without_sections(
        text,
        &[
            "Size",
            "Gate",
            "Allowed beforehand",
            "Unpushed base",
            "Split",
            "Steps done by a worker",
            "Steps returned to their requesters",
        ],
    )
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
#[cfg(test)]
pub(crate) fn measured_sections(
    texts: &AutopilotPrompts,
    size: usize,
    gate: &GateRecord,
    previous: Option<&GateRecord>,
) -> Result<String, PullTextError> {
    measured_sections_in(texts, size, gate, previous, agent_core::local_time::Zone::Utc)
}

/// Renders the measured size and recorded gate in `zone`, including a preceding failure at this head.
pub(crate) fn measured_sections_in(
    texts: &AutopilotPrompts,
    size: usize,
    gate: &GateRecord,
    previous: Option<&GateRecord>,
    zone: agent_core::local_time::Zone,
) -> Result<String, PullTextError> {
    let size = texts.render(
        AutopilotPrompt::PrSize,
        &Variables::new().with("size", size.to_string())?,
    )?;
    let values = Variables::new()
        .with("head", gate.result.head.as_str())?
        .with("finished", zone.stamp(gate.result.finished_at))?
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

/// The section of the steps of the plans that workers did: each request of `requests` (a requester and the requests
/// of its task, as the bus lists them) that is done and names a step and a worker. `None` when there is none.
///
/// # Errors
/// [`PullTextError`] if template rendering or variable construction fails.
pub(crate) fn worker_steps_section(
    texts: &AutopilotPrompts,
    requests: &[(AgentName, Vec<TaskRequest>)],
) -> Result<Option<String>, PullTextError> {
    let rows: Vec<String> = requests
        .iter()
        .flat_map(|(requester, of_task)| of_task.iter().map(move |request| (requester, request)))
        .filter_map(|(requester, request)| match request.status {
            DelegationStatus::Done => request.step.zip(request.worker.as_ref()).map(|(step, worker)| {
                format!(
                    "- step {step} of {requester}'s plan — done by {worker} ({})",
                    request.id
                )
            }),
            DelegationStatus::Queued
            | DelegationStatus::Starting
            | DelegationStatus::Running
            | DelegationStatus::Failed
            | DelegationStatus::Unclear => None,
        })
        .collect();
    if rows.is_empty() {
        return Ok(None);
    }
    Ok(Some(texts.render(
        AutopilotPrompt::PrWorkerSteps,
        &Variables::new().with("steps", rows.join("\n"))?,
    )?))
}

/// The longest reason a row of the section of the returned steps shows, in characters.
pub(crate) const RETURNED_REASON_CHARS: usize = 120;

/// What a row shows for a request whose summary is empty.
pub(crate) const NO_REASON_GIVEN: &str = "no reason given";

/// A request of a task as the bus lists it, with its summary as `helper_get` gives it; the summary is empty for a
/// request the caller did not read.
pub(crate) type RequestWithSummary = (TaskRequest, String);

/// The section of the steps whose requests came back to their requester: each request of `requests` that failed and
/// names a step, with the first line of its summary as the reason. `None` when there is none.
///
/// # Errors
/// [`PullTextError`] if template rendering or variable construction fails.
pub(crate) fn returned_steps_section(
    texts: &AutopilotPrompts,
    requests: &[(AgentName, Vec<RequestWithSummary>)],
) -> Result<Option<String>, PullTextError> {
    let rows: Vec<String> = requests
        .iter()
        .flat_map(|(requester, of_task)| of_task.iter().map(move |entry| (requester, entry)))
        .filter_map(|(requester, (request, summary))| match request.status {
            DelegationStatus::Failed => request.step.map(|step| {
                format!(
                    "- step {step} of {requester}'s plan — returned to {requester} ({}): {}",
                    request.id,
                    reason_of(summary)
                )
            }),
            DelegationStatus::Queued
            | DelegationStatus::Starting
            | DelegationStatus::Running
            | DelegationStatus::Done
            | DelegationStatus::Unclear => None,
        })
        .collect();
    if rows.is_empty() {
        return Ok(None);
    }
    Ok(Some(texts.render(
        AutopilotPrompt::PrReturnedSteps,
        &Variables::new().with("steps", rows.join("\n"))?,
    )?))
}

/// The reason a returned step shows: the first line of `summary`, cut to [`RETURNED_REASON_CHARS`] characters, or
/// [`NO_REASON_GIVEN`] when that line is empty.
fn reason_of(summary: &str) -> String {
    let first = summary.lines().next().unwrap_or_default().trim();
    if first.is_empty() {
        return NO_REASON_GIVEN.to_owned();
    }
    first.chars().take(RETURNED_REASON_CHARS).collect()
}
