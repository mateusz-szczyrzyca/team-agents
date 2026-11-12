//! What the daemon puts on the bus, and when it cannot be done at once, keeps for the next try.

use std::collections::BTreeMap;

use agent_core::ids::AgentName;
use agent_core::prompt::{TeamPrompt, Variables, VariablesError};
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
    let (kind, details, text) = match notice {
        BusNotice::MemberFinished {
            member,
            item: Some(item),
            branch,
        } => (
            NoticeKind::MemberFinished,
            vec![
                ("member", member.to_string()),
                ("item", item.to_string()),
                ("branch", branch.to_string()),
            ],
            texts.render(
                TeamPrompt::MemberFinished,
                &Variables::new()
                    .with("member", member.as_str())?
                    .with("item", item.to_string())?
                    .with("branch", branch.as_str())?,
            )?,
        ),
        BusNotice::MemberFinished {
            member,
            item: None,
            branch,
        } => (
            NoticeKind::MemberFinished,
            vec![("member", member.to_string()), ("branch", branch.to_string())],
            texts.render(
                TeamPrompt::MemberFinishedPlain,
                &Variables::new()
                    .with("member", member.as_str())?
                    .with("branch", branch.as_str())?,
            )?,
        ),
        BusNotice::MemberFailed { member, message } => (
            NoticeKind::MemberFailed,
            vec![("member", member.to_string())],
            texts.render(
                TeamPrompt::MemberFailed,
                &Variables::new()
                    .with("member", member.as_str())?
                    .with("message", message)?,
            )?,
        ),
        BusNotice::MemberNoStatus { member } => (
            NoticeKind::MemberNoStatus,
            vec![("member", member.to_string())],
            texts.render(
                TeamPrompt::MemberNoStatus,
                &Variables::new().with("member", member.as_str())?,
            )?,
        ),
        BusNotice::HelperResult {
            request,
            worker,
            summary,
        } => (
            NoticeKind::HelperResult,
            vec![
                ("request", request.to_string()),
                ("status", HELPER_DONE.to_owned()),
                ("worker", worker.to_string()),
            ],
            texts.render(
                TeamPrompt::HelperResult,
                &Variables::new()
                    .with("worker", worker.as_str())?
                    .with("request_id", request.number().to_string())?
                    .with("summary", summary)?,
            )?,
        ),
        BusNotice::HelperFailed {
            request,
            worker,
            reason,
        } => (
            NoticeKind::HelperResult,
            vec![
                ("request", request.to_string()),
                ("status", HELPER_FAILED.to_owned()),
                ("worker", worker.to_string()),
            ],
            texts.render(
                TeamPrompt::HelperFailed,
                &Variables::new()
                    .with("worker", worker.as_str())?
                    .with("request_id", request.number().to_string())?
                    .with("reason", reason)?,
            )?,
        ),
    };
    let fields = details
        .into_iter()
        .map(|(name, value)| Ok((name.parse::<FieldName>()?, value)))
        .collect::<Result<BTreeMap<_, _>, InvalidFieldName>>()?;
    Ok(Outgoing::Notice { kind, fields, text })
}

/// The `status` detail of the notice of a request a worker finished, as the bus words it.
const HELPER_DONE: &str = "done";

/// The `status` detail of the notice of a request a worker could not do.
const HELPER_FAILED: &str = "failed";

#[cfg(test)]
mod tests;
