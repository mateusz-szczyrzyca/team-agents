//! `[session]`: the size of a session's context above which the harness no longer resumes it, and an agent's own
//! bound.

use serde::Deserialize;

use super::ConfigError;

/// The bound a session's context has when `[session]` says nothing, in tokens. It is taken from the measurements of
/// 22.11.2026 (sessions that grew to 438 000 and 580 000 tokens, and runs that ended `STATUS: FAILED` above 360 000
/// tokens), not from any model's documentation.
pub const DEFAULT_MAX_CONTEXT: u64 = 150_000;

/// `[session]`: the bound on a session's context for an agent that has none of its own.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct Session {
    /// The bound in tokens of a session's context; `0` switches the bound off.
    pub max_context: u64,
}

impl Default for Session {
    fn default() -> Self {
        Self {
            max_context: DEFAULT_MAX_CONTEXT,
        }
    }
}

/// The bound in tokens of an agent's session: its own `max_context` when it has one, else the section's. `None` when
/// the bound is `0`, which switches it off.
#[must_use]
pub fn context_bound(agent: Option<u64>, section: u64) -> Option<u64> {
    let bound = agent.unwrap_or(section);
    (bound != 0).then_some(bound)
}

/// The section of `agents.toml` whose `context` an agent sets: `[agents]` or `[investigators]`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContextKey {
    /// The key `[agents.NAME] context`: a normal agent's or a worker's window.
    Agent,
    /// The key `[investigators.NAME] context`: an investigator's window.
    Investigator,
}

impl ContextKey {
    /// The name of the section, as the error message names it.
    #[must_use]
    pub const fn section(self) -> &'static str {
        match self {
            Self::Agent => "agents",
            Self::Investigator => "investigators",
        }
    }
}

/// Checks the `context` window `context` of the agent or investigator `name` of `key`. `0` is refused: it would tell the
/// CLI that the model reads nothing, and no message fits a window of none. Any other window is accepted, also one below
/// the bound of the session: that is a window at which the CLI compacts a session before the harness replaces it.
///
/// # Errors
/// [`ConfigError::AgentContextZero`] for `0`, naming the section, the agent and the key.
pub(super) fn check_context(key: ContextKey, name: &str, context: Option<u64>) -> Result<(), ConfigError> {
    match context {
        Some(0) => Err(ConfigError::AgentContextZero {
            key,
            name: name.to_owned(),
        }),
        Some(_) | None => Ok(()),
    }
}

/// Checks the `output` limit `output` of the agent or investigator `name` of `key` against its `context`. `0` is refused
/// for the reason `context` of `0` is; an `output` above the window is refused, because a model writes inside its
/// window. An `output` without a `context` is accepted: nothing is written for it, and the other key may come later.
///
/// # Errors
/// [`ConfigError::AgentOutputZero`] for `0`, [`ConfigError::AgentOutputAboveContext`] above the window, each naming the
/// section, the agent and the keys.
pub(super) fn check_output(
    key: ContextKey,
    name: &str,
    context: Option<u64>,
    output: Option<u64>,
) -> Result<(), ConfigError> {
    match (context, output) {
        (_, Some(0)) => Err(ConfigError::AgentOutputZero {
            key,
            name: name.to_owned(),
        }),
        (Some(context), Some(output)) if output > context => Err(ConfigError::AgentOutputAboveContext {
            key,
            name: name.to_owned(),
            output,
            context,
        }),
        (_, Some(_) | None) => Ok(()),
    }
}

#[cfg(test)]
mod tests;
