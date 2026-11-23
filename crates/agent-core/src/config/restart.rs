//! `[restart]`: what a restart of the daemon does with the session of a run it cut.

use serde::Deserialize;

/// Which session a task whose run a restart of the daemon cut goes on in.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RestartSession {
    /// A new session of the same agent, whose first prompt is a briefing of what the cut run left.
    New,
    /// The session the run was cut in, resumed with a short note that the harness restarted.
    #[default]
    Same,
}

/// `[restart]`: what a restart of the daemon does with the runs it cut.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct Restart {
    /// Which session a cut run goes on in.
    pub session: RestartSession,
}

#[cfg(test)]
mod tests;
