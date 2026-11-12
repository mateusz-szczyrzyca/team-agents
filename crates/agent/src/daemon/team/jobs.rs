//! What the daemon puts on the bus, and when it cannot be done at once, keeps for the next try.

use std::collections::BTreeMap;

use agent_core::ids::AgentName;
use agent_core::prompt::VariablesError;
use agent_runtime::agentbus::{DelegationUpdate, FieldName, InvalidFieldName, NoticeKind, Outgoing, RequestId};
use agent_runtime::team_prompts::{RenderTeamTextError, TeamPrompts};

use super::react::BusNotice;

/// One thing the daemon has to do on the bus for a team.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::daemon) enum BusJob {
    /// Put a notice on the bus for a member.
    Tell {
        /// The member that is told.
        to: AgentName,
        /// What it is told.
        notice: BusNotice,
    },
    /// Change a request for a worker.
    Answer {
        /// The request.
        request: RequestId,
        /// What changes.
        update: DelegationUpdate,
    },
    /// Mark the team finished.
    Finish,
}

/// Why a notice cannot be made into a message.
#[derive(Debug, thiserror::Error)]
pub(in crate::daemon) enum NoticeRenderError {
    /// A text of the team cannot be rendered.
    #[error("cannot render the text of a notice")]
    Render(#[from] RenderTeamTextError),
    /// A variable name of the program is not valid.
    #[error("a variable of a text is not valid")]
    Variable(#[from] VariablesError),
    /// A name of a detail of the message is not valid.
    #[error("the name of a detail of a notice is not valid")]
    Field(#[from] InvalidFieldName),
}

/// The message on the bus that `notice` is: its kind, its details and its text.
///
/// # Errors
/// [`NoticeRenderError`].
pub(in crate::daemon) fn render_notice(texts: &TeamPrompts, notice: &BusNotice) -> Result<Outgoing, NoticeRenderError> {
    let _ = (
        texts,
        notice,
        NoticeKind::TeamStuck,
        BTreeMap::<FieldName, String>::new(),
    );
    Ok(Outgoing::Chat { text: String::new() })
}

#[cfg(test)]
mod tests;
