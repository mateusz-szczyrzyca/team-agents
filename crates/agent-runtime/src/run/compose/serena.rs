//! Whether the first prompt of a run tells it how to find code with Serena.
//!
//! The harness gives Serena to the runs it starts when `[binaries] serena` is set, and the model does what it knows:
//! without a word about it, a run greps the code where Serena would give it one body. A run that has Serena is told so
//! in its first prompt — one run of the section per session — and one that has none gets the prompt it always got:
//! `agents.toml` without `serena` renders byte for byte as before.

use std::collections::BTreeMap;

use agent_core::cli::McpServerKind;
use agent_core::config::CommandLine;

#[cfg(test)]
mod tests;
use agent_core::prompt::HarnessPrompt;

use super::PromptKind;

/// Whether a run is given Serena, the MCP server that finds code through language servers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Serena {
    /// The run has Serena among its MCP servers.
    Given,
    /// The run has no Serena.
    Without,
}

impl Serena {
    /// `Given` when a run's own MCP servers, `servers`, hold Serena; `Without` otherwise.
    #[must_use]
    pub fn of(servers: &BTreeMap<McpServerKind, CommandLine>) -> Self {
        if servers.contains_key(&McpServerKind::Serena) {
            Self::Given
        } else {
            Self::Without
        }
    }
}

/// The section on finding code with Serena for a first prompt of the given `kind`, when the run has Serena: `None` for a
/// run that resumes its session (the section is in the prompt of its first run) and a run without Serena.
///
/// Every role gets the section: the role does not change what the run may do with Serena, and a worker's delegated job
/// is about code as often as not. The text says itself that a task without code needs none of it.
#[must_use]
pub fn serena_section(kind: PromptKind, serena: Serena) -> Option<HarnessPrompt> {
    matches!((kind, serena), (PromptKind::First, Serena::Given)).then_some(HarnessPrompt::Serena)
}
