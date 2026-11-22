//! What the daemon puts on the bus, and when it cannot be done at once, keeps for the next try.

use std::collections::BTreeMap;

use agent_core::ids::{AgentName, BranchName, ItemId};
use agent_core::prompt::{TeamPrompt, Variables, VariablesError};
use agent_core::state::Team;
use agent_runtime::agentbus::{DelegationUpdate, FieldName, InvalidFieldName, NoticeKind, Outgoing, RequestId};
use agent_runtime::notify::{InvalidNotification, Notification};
use agent_runtime::team_prompts::{RenderTeamTextError, TeamPrompts};

use super::prompts::SECTION_SEPARATOR;
use super::react::{BusNotice, LeadWork, WorkerEnd};

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
pub(crate) enum NoticeRenderError {
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

/// The kind, the details and the text of a message on the bus, before the details are checked.
type Rendered = (NoticeKind, Vec<(&'static str, String)>, String);

/// The message on the bus that `notice` is: its kind, its details and its text.
///
/// # Errors
/// [`NoticeRenderError`].
pub(in crate::daemon) fn render_notice(texts: &TeamPrompts, notice: &BusNotice) -> Result<Outgoing, NoticeRenderError> {
    let (kind, details, text) = match notice {
        BusNotice::SplitRequested { member, item, task } => (
            NoticeKind::MemberQuestions,
            vec![("member", member.to_string()), ("item", item.to_string())],
            texts.render(
                TeamPrompt::SplitRequested,
                &Variables::new()
                    .with("member", member.as_str())?
                    .with("item", item.to_string())?
                    .with("task", task.as_str())?,
            )?,
        ),
        BusNotice::MemberFinished { member, item, branch } => member_finished(texts, member, item.as_ref(), branch)?,
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
        BusNotice::MemberFailedItem {
            member,
            message,
            item,
            branch,
        } => (
            NoticeKind::MemberFailed,
            vec![
                ("branch", branch.to_string()),
                ("item", item.to_string()),
                ("member", member.to_string()),
            ],
            texts.render(
                TeamPrompt::MemberFailedItem,
                &Variables::new()
                    .with("member", member.as_str())?
                    .with("item", item.to_string())?
                    .with("branch", branch.as_str())?
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
        BusNotice::MemberNoStatusItem { member, item, branch } => no_status_item(texts, member, *item, branch)?,
        BusNotice::LeadTakesWork(work) => lead_takes_work(texts, work)?,
        BusNotice::HelperResult {
            request,
            worker,
            summary,
        } => helper_result(
            texts,
            *request,
            worker,
            HELPER_DONE,
            (TeamPrompt::HelperResult, "summary", summary),
        )?,
        BusNotice::HelperGivenAgain {
            request,
            worker,
            next,
            reason,
        } => given_again(texts, *request, (worker, next), reason)?,
        BusNotice::HelperFailed {
            request,
            worker,
            reason,
        } => helper_result(
            texts,
            *request,
            worker,
            HELPER_FAILED,
            (TeamPrompt::HelperFailed, "reason", reason),
        )?,
    };
    let fields = details
        .into_iter()
        .map(|(name, value)| Ok((name.parse::<FieldName>()?, value)))
        .collect::<Result<BTreeMap<_, _>, InvalidFieldName>>()?;
    Ok(Outgoing::Notice { kind, fields, text })
}

/// The kind, the fields and the text of the message that a member's work is its lead's now, with the briefing of what
/// the member's runs left after it when there is one.
fn lead_takes_work(texts: &TeamPrompts, work: &LeadWork) -> Result<Rendered, NoticeRenderError> {
    let text = texts.render(
        TeamPrompt::LeadTakesWork,
        &Variables::new()
            .with("member", work.member.as_str())?
            .with("task", work.task.as_str())?
            .with("branch", work.branch.as_str())?
            .with("worktree", &work.worktree)?,
    )?;
    let text = match &work.briefing {
        Some(briefing) => format!("{}\n\n{}", text.trim_end(), briefing.trim_end()),
        None => text,
    };
    Ok((
        NoticeKind::MemberFailed,
        vec![
            ("branch", work.branch.to_string()),
            ("member", work.member.to_string()),
            ("task", work.task.to_string()),
        ],
        text,
    ))
}

/// The notice that `member` ended its turn without a status line while working on `item` of `branch`.
fn no_status_item(
    texts: &TeamPrompts,
    member: &AgentName,
    item: ItemId,
    branch: &BranchName,
) -> Result<Rendered, NoticeRenderError> {
    Ok((
        NoticeKind::MemberNoStatus,
        vec![
            ("branch", branch.to_string()),
            ("item", item.to_string()),
            ("member", member.to_string()),
        ],
        texts.render(
            TeamPrompt::MemberNoStatusItem,
            &Variables::new()
                .with("member", member.as_str())?
                .with("item", item.to_string())?
                .with("branch", branch.as_str())?,
        )?,
    ))
}

/// The notice that `member` is done: with its item when the list has items, else with its branch alone.
fn member_finished(
    texts: &TeamPrompts,
    member: &AgentName,
    item: Option<&ItemId>,
    branch: &BranchName,
) -> Result<Rendered, NoticeRenderError> {
    let common = Variables::new()
        .with("member", member.as_str())?
        .with("branch", branch.as_str())?;
    let mut details = vec![("member", member.to_string()), ("branch", branch.to_string())];
    let text = match item {
        Some(item) => {
            details.push(("item", item.to_string()));
            texts.render(TeamPrompt::MemberFinished, &common.with("item", item.to_string())?)?
        }
        None => texts.render(TeamPrompt::MemberFinishedPlain, &common)?,
    };
    Ok((NoticeKind::MemberFinished, details, text))
}

/// The notice that the worker finished (or could not do) the request `request`; `text` is the text, the name of its
/// variable for the outcome, and the outcome.
fn helper_result(
    texts: &TeamPrompts,
    request: RequestId,
    worker: &AgentName,
    status: &str,
    (prompt, variable, outcome): (TeamPrompt, &str, &String),
) -> Result<Rendered, NoticeRenderError> {
    let values = Variables::new()
        .with("worker", worker.as_str())?
        .with("request_id", request.number().to_string())?
        .with(variable, outcome)?;
    Ok((
        NoticeKind::HelperResult,
        vec![
            ("request", request.to_string()),
            ("status", status.to_owned()),
            ("worker", worker.to_string()),
        ],
        texts.render(prompt, &values)?,
    ))
}

/// Why a worker could not do a request, for its requester: how the worker's run ended (`how`), then the end of what it
/// said (`said`), if it said anything.
///
/// # Errors
/// [`NoticeRenderError`].
pub(in crate::daemon) fn failure_reason(
    texts: &TeamPrompts,
    how: WorkerEnd,
    said: Option<&str>,
) -> Result<String, NoticeRenderError> {
    let prompt = match how {
        WorkerEnd::Failed => TeamPrompt::WorkerFailed,
        WorkerEnd::NoResult => TeamPrompt::WorkerNoResult,
        WorkerEnd::NoStatus => TeamPrompt::WorkerNoStatus,
        WorkerEnd::Stopped => TeamPrompt::WorkerStopped,
        WorkerEnd::Hung => TeamPrompt::WorkerHung,
    };
    let sentence = texts.render(prompt, &Variables::new())?;
    let sentence = sentence.trim_end();
    Ok(match said {
        Some(said) => format!("{sentence}{SECTION_SEPARATOR}{said}"),
        None => sentence.to_owned(),
    })
}

/// The text that tells a requester its worker's commit is on its branch: the step, the commit, the files and the end of
/// the output of the check that passed.
///
/// # Errors
/// [`NoticeRenderError`].
pub(in crate::daemon) fn worker_result_brought(
    texts: &TeamPrompts,
    worker: &AgentName,
    step: &str,
    commit: &str,
    files: &str,
    tail: &str,
) -> Result<String, NoticeRenderError> {
    let values = Variables::new()
        .with("worker", worker.as_str())?
        .with("step", step)?
        .with("commit", commit)?
        .with("files", files)?
        .with("tail", tail)?;
    Ok(texts.render(TeamPrompt::WorkerResultBrought, &values)?)
}

/// The notice that the result of `worker` did not serve the request `request`, which goes once more to `next`.
fn given_again(
    texts: &TeamPrompts,
    request: RequestId,
    (worker, next): (&AgentName, &AgentName),
    reason: &str,
) -> Result<Rendered, NoticeRenderError> {
    let values = Variables::new()
        .with("worker", worker.as_str())?
        .with("request_id", request.number().to_string())?
        .with("reason", reason)?
        .with("next", next.as_str())?;
    Ok((
        NoticeKind::HelperResult,
        vec![
            ("request", request.to_string()),
            ("status", "again".to_owned()),
            ("worker", worker.to_string()),
        ],
        texts.render(TeamPrompt::WorkerGivenAgain, &values)?,
    ))
}

/// The text that tells the worker of a second try who served the request before it and why that result did not serve.
///
/// # Errors
/// [`NoticeRenderError`].
pub(in crate::daemon) fn delegation_again(
    texts: &TeamPrompts,
    previous_worker: &AgentName,
    reason: &str,
) -> Result<String, NoticeRenderError> {
    let values = Variables::new()
        .with("previous_worker", previous_worker.as_str())?
        .with("reason", reason)?;
    Ok(texts.render(TeamPrompt::DelegationAgain, &values)?)
}

/// The text that tells a requester the worker's commit does not apply to its branch: the files in conflict.
///
/// # Errors
/// [`NoticeRenderError`].
pub(in crate::daemon) fn worker_result_conflict(
    texts: &TeamPrompts,
    worker: &AgentName,
    step: &str,
    commit: &str,
    files: &str,
) -> Result<String, NoticeRenderError> {
    let values = Variables::new()
        .with("worker", worker.as_str())?
        .with("step", step)?
        .with("commit", commit)?
        .with("files", files)?;
    Ok(texts.render(TeamPrompt::WorkerResultConflict, &values)?)
}

/// The text that gives a step back to the requester: every reason a result was wrong, one on a line.
///
/// # Errors
/// [`NoticeRenderError`].
pub(in crate::daemon) fn worker_step_returned(
    texts: &TeamPrompts,
    request: RequestId,
    step: &str,
    reasons: &[String],
) -> Result<String, NoticeRenderError> {
    let listed: Vec<String> = reasons.iter().map(|reason| format!("- {reason}")).collect();
    let values = Variables::new()
        .with("request_id", request.number().to_string())?
        .with("step", step)?
        .with("reasons", listed.join("\n"))?;
    Ok(texts.render(TeamPrompt::WorkerStepReturned, &values)?)
}

/// Why the notification that a team is finished cannot be made.
#[derive(Debug, thiserror::Error)]
pub(crate) enum DoneNoticeError {
    /// The text cannot be rendered.
    #[error("cannot render the notification that a team is finished")]
    Render(#[from] RenderTeamTextError),
    /// A variable name of the program is not valid.
    #[error("a variable of a text is not valid")]
    Variable(#[from] VariablesError),
    /// The rendered text is not a notification.
    #[error("the text that a team is finished is not a notification")]
    Invalid(#[from] InvalidNotification),
}

use crate::daemon::notice::{NoticeContext, NoticeKind as HumanNoticeKind, layout_notification};

/// The notification for the human that `team` is finished.
///
/// # Errors
/// [`DoneNoticeError`].
pub(in crate::daemon) fn team_done_notification(
    texts: &TeamPrompts,
    team: &Team,
    context: &NoticeContext<'_>,
) -> Result<Notification, DoneNoticeError> {
    let values = Variables::new()
        .with("team", team.id().as_str())?
        .with("repo", team.repo().as_str())?
        .with("items", team.items().len().to_string())?;
    let rendered = texts.render(TeamPrompt::NoticeTeamDone, &values)?;
    Ok(layout_notification(HumanNoticeKind::TeamDone, context, &rendered)?)
}

/// The `status` detail of the notice of a request a worker finished, as the bus words it.
const HELPER_DONE: &str = "done";

/// The `status` detail of the notice of a request a worker could not do.
const HELPER_FAILED: &str = "failed";

#[cfg(test)]
mod tests;
