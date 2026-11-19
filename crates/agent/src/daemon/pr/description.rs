//! Harness-owned description sections and the lead's editable contribution.

use super::PullTextError;
use agent_core::prompt::{AutopilotPrompt, Variables};
use agent_core::state::GateRecord;
use agent_runtime::autopilot_prompts::AutopilotPrompts;

/// Removes harness-owned sections from the lead's contribution.
pub(crate) fn lead_sections(text: &str) -> String {
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
                if title.eq_ignore_ascii_case("Size") || title.eq_ignore_ascii_case("Gate") {
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
