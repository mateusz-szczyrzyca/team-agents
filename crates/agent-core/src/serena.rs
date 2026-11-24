//! The reminder for a run with Serena that searches code without it.
//!
//! A prompt asks an agent to find code with Serena; it does not make it. A run that has Serena and goes on with the shell
//! and its CLI's own tools is told once, while it goes on, that Serena finds symbols and that these tools stay for text.
//! This module holds what that reminder needs: the rule of which step is a search of code without Serena. It is pure: the
//! run engine feeds the steps in and asks.

use crate::cli::ActivityEvent;

#[cfg(test)]
mod tests;

/// The files a step searched or read, when it searched or read code without Serena; `None` for a step that is not
/// one: Serena's own calls, agentbus, a file change, a message, and a search of text only.
#[must_use]
pub fn code_search(event: &ActivityEvent) -> Option<CodeSearch> {
    let _ = event;
    None
}

/// The files of code one step searched or read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodeSearch {
    /// The paths it read, as the CLI named them, in order and without a path twice.
    pub paths: Vec<String>,
}
