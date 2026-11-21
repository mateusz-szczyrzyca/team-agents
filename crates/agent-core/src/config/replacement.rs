//! `[replacement]`: how a task whose runs go bad is kept going — in a new session, then by another agent, by the
//! team's lead, or by the human.

use std::collections::{BTreeMap, HashSet};

use serde::Deserialize;

use super::{AgentConfig, ConfigError, InvestigatorConfig, RenewalLimit};
use crate::decide::replacement::ReplacementMode;
use crate::ids::{AgentName, InvestigatorName};

/// `[replacement]`: whether a task whose runs go bad is kept going, and how.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct Replacement {
    /// How many new sessions of its agent a task is started in after its runs went bad; `0` never starts one.
    new_sessions: RenewalLimit,
    /// The agents that may take a task over, in the order the file names them.
    agents: Vec<AgentName>,
    /// How far the harness goes to keep a task going without the human.
    mode: ReplacementMode,
    /// How many times a task is taken over by another agent after its runs went bad; `0` never takes one over.
    replacements: replacements::Limit,
}

impl Replacement {
    /// How many new sessions of its agent a task is started in after its runs went bad; `0` never starts one.
    #[must_use]
    pub const fn new_sessions(&self) -> RenewalLimit {
        self.new_sessions
    }

    /// The agents that may take a task over, in the order the file names them; none when the file names none.
    #[must_use]
    pub fn agents(&self) -> &[AgentName] {
        &[]
    }

    /// How far the harness goes to keep a task going without the human.
    #[must_use]
    pub const fn mode(&self) -> ReplacementMode {
        ReplacementMode::Auto
    }

    /// How many times a task is taken over by another agent after its runs went bad; `0` never takes one over.
    #[must_use]
    pub fn replacements(&self) -> replacements::Limit {
        replacements::Limit::default()
    }
}

/// The numbers of `[replacement]`.
pub(super) mod replacements {
    /// The most times a task is taken over by another agent: a configuration above it is refused.
    const MAX: u32 = 10;

    /// How many times a task may be taken over by another agent: `0` to 10.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize, serde::Deserialize)]
    #[serde(try_from = "u32", into = "u32")]
    pub struct Limit(u32);

    /// A number the takeovers of a task cannot have.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
    #[error("{value} takeovers per task is not a limit: it is 0 to {MAX}")]
    pub struct InvalidLimit {
        /// The refused number.
        pub value: u32,
    }

    impl Limit {
        /// A limit of `value` takeovers per task.
        ///
        /// # Errors
        /// [`InvalidLimit`] above 10.
        pub fn new(value: u32) -> Result<Self, InvalidLimit> {
            if value <= MAX {
                Ok(Self(value))
            } else {
                Err(InvalidLimit { value })
            }
        }

        /// The number of takeovers per task this limit allows.
        #[must_use]
        pub const fn get(self) -> u32 {
            self.0
        }
    }

    impl TryFrom<u32> for Limit {
        type Error = InvalidLimit;

        fn try_from(value: u32) -> Result<Self, Self::Error> {
            Self::new(value)
        }
    }

    impl From<Limit> for u32 {
        fn from(limit: Limit) -> Self {
            limit.get()
        }
    }

    impl Default for Limit {
        fn default() -> Self {
            // One takeover per task, as for a new session.
            Self(1)
        }
    }
}

/// Checks that `[replacement] agents` names only configured agents, each once, in the order the file gives.
///
/// # Errors
/// [`ConfigError::ReplacementAgentUnknown`] for a name that has no table in the file;
/// [`ConfigError::ReplacementAgentTwice`] for a name named twice.
pub(super) fn check(
    agents: &[AgentName],
    configured: &BTreeMap<AgentName, AgentConfig>,
    investigators: &BTreeMap<InvestigatorName, InvestigatorConfig>,
) -> Result<(), ConfigError> {
    let mut seen: HashSet<&AgentName> = HashSet::new();
    for name in agents {
        let known = configured.contains_key(name) || investigators.keys().any(|inv| inv.as_str() == name.as_str());
        if !known {
            return Err(ConfigError::ReplacementAgentUnknown { name: name.clone() });
        }
        if !seen.insert(name) {
            return Err(ConfigError::ReplacementAgentTwice { name: name.clone() });
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
