//! Harness-owned description sections and the lead's editable contribution.

use super::PullTextError;
use agent_core::state::GateRecord;
use agent_runtime::autopilot_prompts::AutopilotPrompts;

/// Removes harness-owned sections from the lead's contribution.
#[cfg_attr(
    not(test),
    expect(dead_code, reason = "wired into publication after the RED specification")
)]
pub(crate) fn lead_sections(_text: &str) -> String {
    String::new()
}

/// Renders the measured size and recorded gate, including a preceding failure at this head.
#[cfg_attr(
    not(test),
    expect(dead_code, reason = "wired into publication after the RED specification")
)]
pub(crate) fn measured_sections(
    _texts: &AutopilotPrompts,
    _size: usize,
    _gate: &GateRecord,
    _previous: Option<&GateRecord>,
) -> Result<String, PullTextError> {
    Ok(String::new())
}

#[cfg(test)]
mod tests;
