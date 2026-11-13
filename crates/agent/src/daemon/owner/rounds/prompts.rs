//! What the autopilot adds to the prompt of a run: the request for a review that a reviewer is given, the fix pass the
//! lead is given, and the request to put the question to the human. All of it is made from the state and the kept
//! reports, so that it survives a restart.

use std::str::FromStr;

use agent_core::config::{AbsolutePath, InvalidValue, RepoName};
use agent_core::ids::InvalidName;
use agent_core::kinds::Role;
use agent_core::prompt::{AutopilotPrompt, Variables, VariablesError};
use agent_core::state::{CommitId, ItemPhase, PrNumber, QuestionState, Review, ReviewOutcome, TaskRef, Timestamp};
use agent_core::status::ReviewVerdict;
use agent_runtime::autopilot_prompts::{AutopilotPrompts, RenderAutopilotTextError};
use agent_runtime::git::{CommitIdentity, Git, GitError, Revision};
use agent_runtime::run::compose::{ComposeError, PromptTexts, item_sections};
use agent_runtime::store::{StoreError, load_checks_record};
use agent_runtime::text_file::{TextFileError, read_text_file};

use super::super::start::{Context, next_number};
use crate::daemon::checks::{ChecksTextError, answered, relay};
use crate::daemon::merge::{Decision, decision, exhausted};
use crate::daemon::team::Why;

use crate::daemon::merge::count;

/// Why what the autopilot adds to a prompt cannot be made.
#[derive(Debug, thiserror::Error)]
pub(in crate::daemon::owner) enum SectionError {
    /// A text cannot be rendered.
    #[error("cannot render a text of the autopilot")]
    Render(#[from] RenderAutopilotTextError),
    /// A variable name of the program is not valid.
    #[error("a variable of a text is not valid")]
    Variable(#[from] VariablesError),
    /// An item cannot be rendered.
    #[error("cannot render an item")]
    Item(#[from] ComposeError),
    /// Git cannot say which commit the worktree has out.
    #[error("git cannot say which commit the worktree has out")]
    Head(#[from] GitError),
    /// The name of the agent is not an agent name.
    #[error("the name of the agent is not an agent name")]
    Name(#[from] InvalidName),
    /// The worktree is not a path git can be run in.
    #[error("the worktree {path:?} cannot be used")]
    Path {
        /// The path.
        path: std::path::PathBuf,
        /// What is wrong with it, if the text was refused.
        source: Option<InvalidValue>,
    },
    /// The repository of the team is not in the configuration.
    #[error("the repository {repo} is not in the configuration")]
    RepoGone {
        /// The repository.
        repo: RepoName,
    },
    /// A text of the checks before a review cannot be made.
    #[error("cannot make a text of the checks before a review")]
    Checks(#[from] ChecksTextError),
    /// The record of the checks cannot be read back.
    #[error("cannot read the record of the checks")]
    Record(#[from] StoreError),
    /// The report of a review cannot be read back.
    #[error("cannot read the report {path:?}")]
    Report {
        /// The file.
        path: std::path::PathBuf,
        /// What is wrong with it.
        source: TextFileError,
    },
}

/// What the autopilot does to the prompt of a run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::daemon::owner) enum Addition {
    /// This follows the prompt.
    Append(String),
    /// This is the prompt: the session that is resumed holds the rest.
    Replace(String),
}

/// What the autopilot adds to the prompt of the run of the task `found` that works in `working`, started for `why`: for a
/// reviewer, the pull request it is asked for a review of and the items it is for, or the demand for a verdict when its
/// report had none; `None` when the task has nothing of the autopilot to be told.
///
/// # Errors
/// [`SectionError`].
pub(in crate::daemon::owner) async fn sections_for(
    ctx: &Context<'_>,
    texts: &AutopilotPrompts,
    found: &TaskRef<'_>,
    working: &std::path::Path,
    why: Option<&Why>,
) -> Result<Option<Addition>, SectionError> {
    match (found.role, why) {
        (Role::Reviewer, Some(Why::VerdictMissing)) => {
            let text = texts.render(AutopilotPrompt::ReviewVerdictMissing, &Variables::new())?;
            Ok(Some(Addition::Replace(text.trim_end().to_owned())))
        }
        (Role::Reviewer, _) => Ok(review_request(ctx, texts, found, working).await?.map(Addition::Append)),
        (Role::Solo | Role::Coordinator, _) => {
            // What the checks before a review tell the lead comes first: it is the last thing that happened to its round.
            let text = match checks_notice(ctx, found).await? {
                Some(text) => Some(text),
                None => match fix_pass(ctx, texts, found).await? {
                    Some(text) => Some(text),
                    None => ask_human(ctx, texts, found).await?,
                },
            };
            Ok(text.map(Addition::Append))
        }
        (Role::Member | Role::Worker, _) => Ok(None),
    }
}

/// What the checks before a review tell the lead `found` that it has not been told yet: the reasons the round goes back to
/// it with, the questions it is to put to the human, or what to do now that the human answered them.
async fn checks_notice(ctx: &Context<'_>, found: &TaskRef<'_>) -> Result<Option<String>, SectionError> {
    let Some(texts) = ctx.checks else {
        return Ok(None);
    };
    let (task, member) = (found.member.task(), found.member);
    let Some(record) = load_checks_record(ctx.home, task).await? else {
        return Ok(None);
    };
    // What was written for the lead is told in the run that starts after it was written; a run that has started since
    // has been told.
    let ran_since = |time: Timestamp| member.runs().last().is_some_and(|run| run.started_at() >= time);
    if let Some(sent) = record.sent.as_ref().filter(|sent| !ran_since(sent.at)) {
        return Ok(Some(sent.text.clone()));
    }
    let Some(asked) = &record.asked else {
        return Ok(None);
    };
    match member
        .questions()
        .iter()
        .find(|question| question.number() == asked.number)
    {
        None if !ran_since(asked.at) => {
            let file = ctx.home.questions_file(task, asked.number);
            Ok(Some(relay(texts, task, asked.number, &file)?))
        }
        Some(question) => match question.state() {
            QuestionState::Answered { at, .. } if !ran_since(*at) => Ok(Some(answered(texts)?)),
            QuestionState::Answered { .. } | QuestionState::Open => Ok(None),
        },
        None => Ok(None),
    }
}

/// The fix pass of the lead `found`, if an item is being fixed by it: the reports of the reviewers that asked for changes.
pub(super) async fn fix_pass(
    ctx: &Context<'_>,
    texts: &AutopilotPrompts,
    found: &TaskRef<'_>,
) -> Result<Option<String>, SectionError> {
    let team = found.team;
    let name = found.member.agent().name();
    let fixing = team.items().iter().find_map(|progress| match progress.phase() {
        ItemPhase::Fixing { assignee, branch, pr } if assignee.as_str() == name => Some((progress, branch, pr)),
        ItemPhase::Fixing { .. }
        | ItemPhase::Pending
        | ItemPhase::InProgress { .. }
        | ItemPhase::InReview { .. }
        | ItemPhase::Approved { .. }
        | ItemPhase::Done { .. } => None,
    });
    let Some((progress, branch, pr)) = fixing else {
        return Ok(None);
    };
    let Some(asked) = progress.fix_passes().last() else {
        return Ok(None);
    };
    let passes = progress.fix_passes().len();
    // The reports of the review pass that asked for this pass were kept under the number of the passes before it.
    let kept = count(passes.saturating_sub(1));
    let findings = reports_of(ctx, texts, &asked.requested_by, pr.number, kept).await?;
    let values = Variables::new()
        .with("pr", pr.number.to_string())?
        .with("pass", passes.to_string())?
        .with("passes", ctx.settings.config.review().fix_passes.to_string())?
        .with("branch", branch.as_str())?
        .with("findings", findings.join("\n\n"))?;
    Ok(Some(
        texts
            .render(AutopilotPrompt::ReviewFixPass, &values)?
            .trim_end()
            .to_owned(),
    ))
}

/// The request to the lead `found` to put the question to the human, when a pull request of the team waits for it: the
/// reports of the reviewers that asked for changes, and the questions file to write.
pub(super) async fn ask_human(
    ctx: &Context<'_>,
    texts: &AutopilotPrompts,
    found: &TaskRef<'_>,
) -> Result<Option<String>, SectionError> {
    let team = found.team;
    let Some(waits) = exhausted(team)
        .into_iter()
        .find(|waits| decision(found.member, waits.requested_at) == Decision::Ask)
    else {
        return Ok(None);
    };
    let Some(number) = next_number(found.member) else {
        return Ok(None);
    };
    let mut findings = Vec::new();
    for id in waits.items.iter().take(1) {
        let Some(progress) = team.item(*id) else {
            continue;
        };
        let ItemPhase::InReview { reviews, .. } = progress.phase() else {
            continue;
        };
        let kept = count(progress.fix_passes().len());
        findings.extend(reports_of(ctx, texts, reviews, waits.pr.number, kept).await?);
    }
    let file = ctx.home.questions_file(found.member.task(), number);
    let values = Variables::new()
        .with("pr", waits.pr.number.to_string())?
        .with("passes", waits.passes.to_string())?
        .with("findings", findings.join("\n\n"))?
        .with("questions_file", file.display().to_string())?
        .with("number", number.to_string())?
        .with("task", found.member.task().as_str())?;
    Ok(Some(
        texts
            .render(AutopilotPrompt::ReviewAskHuman, &values)?
            .trim_end()
            .to_owned(),
    ))
}

/// The kept reports of the reviews in `reviews` that asked for changes, each as a finding.
pub(super) async fn reports_of(
    ctx: &Context<'_>,
    texts: &AutopilotPrompts,
    reviews: &[Review],
    pr: PrNumber,
    kept: u32,
) -> Result<Vec<String>, SectionError> {
    let mut findings = Vec::new();
    for review in reviews {
        if !matches!(
            review.outcome,
            ReviewOutcome::Submitted {
                verdict: ReviewVerdict::RequestChanges,
                ..
            }
        ) {
            continue;
        }
        let path = ctx.home.review_report_file(&review.task, pr, kept);
        if let Some(file) = read_text_file(&path, MAX_REPORT_BYTES)
            .await
            .map_err(|source| SectionError::Report { path, source })?
        {
            let values = Variables::new()
                .with("reviewer", review.reviewer.as_str())?
                .with("report", file.text.trim())?;
            findings.push(
                texts
                    .render(AutopilotPrompt::ReviewFinding, &values)?
                    .trim_end()
                    .to_owned(),
            );
        }
    }
    Ok(findings)
}

/// The most bytes of a report that are read back: far above [`MAX_REPORT_CHARS`](crate::daemon::review::MAX_REPORT_CHARS).
pub(super) const MAX_REPORT_BYTES: usize = 1 << 20;

/// The request for a review that the reviewer `found` has pending, with the items of the pull request.
pub(super) async fn review_request(
    ctx: &Context<'_>,
    texts: &AutopilotPrompts,
    found: &TaskRef<'_>,
    working: &std::path::Path,
) -> Result<Option<String>, SectionError> {
    let team = found.team;
    let task = found.member.task();
    let mut pending = team.items().iter().filter_map(|progress| match progress.phase() {
        ItemPhase::InReview { pr, reviews }
            if reviews
                .iter()
                .any(|review| review.task == *task && matches!(review.outcome, ReviewOutcome::Pending)) =>
        {
            Some((progress, pr))
        }
        ItemPhase::Pending
        | ItemPhase::InProgress { .. }
        | ItemPhase::InReview { .. }
        | ItemPhase::Fixing { .. }
        | ItemPhase::Approved { .. }
        | ItemPhase::Done { .. } => None,
    });
    let Some((first, pr)) = pending.next() else {
        return Ok(None);
    };
    let items: Vec<_> = std::iter::once(first)
        .chain(pending.map(|(progress, _)| progress))
        .collect();
    let base = ctx
        .settings
        .config
        .repo(team.repo())
        .map(|repo| repo.base_branch.clone())
        .ok_or_else(|| SectionError::RepoGone {
            repo: team.repo().clone(),
        })?;
    let head = head_of(ctx, found, working).await?;
    let values = Variables::new()
        .with("team", team.id().as_str())?
        .with("pr", pr.number.to_string())?
        .with("branch", pr.branch.as_str())?
        .with("base", base.as_str())?
        .with("head", head.as_str())?
        .with("pass", first.fix_passes().len().saturating_add(1).to_string())?;
    let mut sections = vec![texts.render(AutopilotPrompt::ReviewRequest, &values)?];
    let harness = PromptTexts {
        harness: ctx.texts,
        rules: ctx.rules,
    };
    for progress in items {
        if let Some(item) = team.task_list().list().item(progress.id()) {
            sections.extend(item_sections(&harness, item)?);
        }
    }
    Ok(Some(
        sections
            .iter()
            .map(|section| section.trim_end())
            .collect::<Vec<_>>()
            .join("\n\n"),
    ))
}

/// The commit the worktree of the task `found` has out, which is the one a reviewer reviews.
async fn head_of(ctx: &Context<'_>, found: &TaskRef<'_>, working: &std::path::Path) -> Result<CommitId, SectionError> {
    let text = working.to_str().ok_or_else(|| SectionError::Path {
        path: working.to_path_buf(),
        source: None,
    })?;
    let worktree = AbsolutePath::from_str(text).map_err(|source| SectionError::Path {
        path: working.to_path_buf(),
        source: Some(source),
    })?;
    let git = Git::new(
        worktree,
        CommitIdentity::for_agent(ctx.settings.config.git(), &found.member.agent().name().parse()?),
    );
    Ok(git.rev_parse(&Revision::Head).await?)
}
