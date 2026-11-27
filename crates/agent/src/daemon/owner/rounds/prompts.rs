//! What the autopilot adds to the prompt of a run: the request for a review that a reviewer is given, the fix pass the
//! lead is given, and the request to put the question to the human. All of it is made from the state and the kept
//! reports, so that it survives a restart.

use std::str::FromStr;

use agent_core::config::{AbsolutePath, InvalidValue, RepoName};
use agent_core::ids::{BranchName, InvalidName, ItemId};
use agent_core::kinds::Role;
use agent_core::local_time::Zone;
use agent_core::log::LogEvent;
use agent_core::prompt::{AutopilotPrompt, HarnessPrompt, Variables, VariablesError};
use agent_core::state::{
    CommitId, Findings, ItemPhase, ItemProgress, MergeConflict, PrNumber, PullRequest, QuestionState, Review,
    ReviewOutcome, TaskRef, Timestamp,
};
use agent_runtime::autopilot_prompts::{AutopilotPrompts, RenderAutopilotTextError};
use agent_runtime::git::{CommitIdentity, Git, GitError, Revision};
use agent_runtime::run::compose::{ComposeError, PromptTexts, item_sections};
use agent_runtime::store::{StoreError, load_checks_record};
use agent_runtime::text_file::{TextFileError, read_text_file, replace_text_file};

use super::super::start::{Context, next_number};
use crate::daemon::checks::{ChecksTextError, answered, relay};
use crate::daemon::merge::{Decision, decision, exhausted};
use crate::daemon::team::Why;

use crate::daemon::merge::count;

/// Why what the autopilot adds to a prompt cannot be made.
#[derive(Debug, thiserror::Error)]
pub(in crate::daemon::owner) enum SectionError {
    /// A refused answer cannot be described.
    #[error("cannot render refused answer diagnostics")]
    Response(#[from] super::responses::ResponseTextError),
    /// A findings text cannot be rendered.
    #[error("cannot render findings")]
    Findings(#[from] super::findings::TextError),
    /// The repository of the team is not configured, so its base branch is not known.
    #[error("the repository {repo} of the team is not configured")]
    Repo {
        /// The repository.
        repo: agent_core::config::RepoName,
    },
    /// The conflict request's item no longer has an open pull request.
    #[error("item {item} has no open pull request for a conflict investigation")]
    ConflictPullRequest {
        /// The requested item.
        item: ItemId,
    },
    /// A completed investigation has lost its saved answer.
    #[error("the conflict investigation answer {path:?} is missing")]
    MissingInvestigationAnswer {
        /// The answer file supplied by the bus.
        path: std::path::PathBuf,
    },
    /// A text cannot be rendered.
    #[error("cannot render a text of the autopilot")]
    Render(#[from] RenderAutopilotTextError),
    /// A text of the harness cannot be rendered.
    #[error("cannot render a text of the harness")]
    Harness(#[from] agent_runtime::harness_prompts::RenderTextError),
    /// The bus could not be queried.
    #[error("cannot read from the bus")]
    Bus(#[from] agent_runtime::agentbus::AgentBusError),
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
#[cfg(test)]
pub(in crate::daemon::owner) async fn sections_for(
    ctx: &Context<'_>,
    texts: &AutopilotPrompts,
    found: &TaskRef<'_>,
    working: &std::path::Path,
    why: Option<&Why>,
) -> Result<Option<Addition>, SectionError> {
    sections_for_in(ctx, texts, found, working, why, Zone::Utc).await
}

/// What the autopilot adds to the prompt of the run of `found` in `zone`.
///
/// # Errors
/// [`SectionError`].
pub(in crate::daemon::owner) async fn sections_for_in(
    ctx: &Context<'_>,
    texts: &AutopilotPrompts,
    found: &TaskRef<'_>,
    working: &std::path::Path,
    why: Option<&Why>,
    zone: Zone,
) -> Result<Option<Addition>, SectionError> {
    let addition = round_sections_for(ctx, texts, found, working, why, zone).await?;
    if !matches!(found.role, Role::Solo | Role::Coordinator) {
        return Ok(addition);
    }
    let file = ctx
        .home
        .task_dir(found.member.task())
        .join(crate::daemon::pr::description::LEAD_DESCRIPTION_FILE);
    let instruction = texts.render(
        AutopilotPrompt::PrLeadInstructions,
        &Variables::new().with("description_file", file.display().to_string())?,
    )?;
    Ok(Some(match addition {
        Some(Addition::Replace(text)) => Addition::Replace([text.trim_end(), instruction.trim_end()].join("\n\n")),
        Some(Addition::Append(text)) => Addition::Append([text.trim_end(), instruction.trim_end()].join("\n\n")),
        None => Addition::Append(instruction.trim_end().to_owned()),
    }))
}

/// Renders the pending operation before the shared lead instructions.
async fn round_sections_for(
    ctx: &Context<'_>,
    texts: &AutopilotPrompts,
    found: &TaskRef<'_>,
    working: &std::path::Path,
    why: Option<&Why>,
    zone: Zone,
) -> Result<Option<Addition>, SectionError> {
    if let Some((id, number, opinion)) = crate::daemon::review::pending_opinion(found.team, found.member.task()) {
        if matches!(why, Some(Why::VerdictMissing)) {
            return Ok(Some(Addition::Replace(
                texts.render(AutopilotPrompt::ReviewOpinionCorrection, &Variables::new())?,
            )));
        }
        if let Some(entry) = found
            .team
            .item(id)
            .and_then(|item| item.findings())
            .and_then(|ledger| ledger.entries().iter().find(|entry| entry.number == number))
        {
            let values = Variables::new()
                .with("number", number.to_string())?
                .with("severity", entry.finding.severity.word())?
                .with("rule", &entry.finding.rule)?
                .with("place", &entry.finding.place)?
                .with("text", &entry.finding.text)?
                .with("check", &entry.finding.check)?
                .with("reason", opinion.request.reason.as_str())?
                .with("head", opinion.request.head.as_str())?;
            return Ok(Some(Addition::Replace(
                texts.render(AutopilotPrompt::ReviewOpinionRequest, &values)?,
            )));
        }
    }
    match (found.role, why) {
        (Role::Reviewer, Some(Why::ReviewFormat(problem))) => {
            let values = Variables::new()
                .with("line", problem.problem.line.to_string())?
                .with("reason", problem.problem.kind.to_string())?;
            let mut correction = texts.render(AutopilotPrompt::ReviewFindingsInvalid, &values)?;
            let ledger = found.team.item(problem.item).and_then(|item| item.findings());
            let forms = match ledger {
                Some(ledger) => super::responses::verifier_forms(texts, ledger)?,
                None => String::new(),
            };
            correction.push_str(&super::responses::answer_diagnostic(
                texts,
                &problem.problem.detail,
                &forms,
            )?);
            Ok(Some(Addition::Replace(correction)))
        }
        (Role::Reviewer, Some(Why::VerdictMissing)) => {
            let text = texts.render(AutopilotPrompt::ReviewVerdictMissing, &Variables::new())?;
            Ok(Some(Addition::Replace(text.trim_end().to_owned())))
        }
        (Role::Reviewer, _) => Ok(review_request(ctx, texts, found, working, zone)
            .await?
            .map(Addition::Append)),
        // The gate comes before everything else of the round: a branch whose gate has not passed is not looked at further.
        (Role::Solo | Role::Coordinator, Some(Why::Gate(text) | Why::Asleep(text))) => {
            Ok(Some(Addition::Append(text.clone())))
        }
        (Role::Solo | Role::Coordinator, _) => {
            // A merge refused for a conflict comes first: the lead merges the base, or is asked to put the question.
            if let Some(text) = merge_conflict_text(ctx, texts, found).await? {
                return Ok(Some(Addition::Append(text)));
            }
            // What the checks before a review tell the lead comes first: it is the last thing that happened to its round.
            let text = match checks_notice(ctx, found).await? {
                Some(text) => Some(text),
                None => match fix_pass(ctx, texts, found, working, matches!(why, Some(Why::Woken))).await? {
                    Some(text) => Some(text),
                    None => ask_human(ctx, texts, found).await?,
                },
            };
            Ok(text.map(Addition::Append))
        }
        (Role::Member | Role::Worker, Some(Why::Gate(text))) => Ok(Some(Addition::Append(text.clone()))),
        (Role::Member | Role::Worker, _) => Ok(fix_pass(ctx, texts, found, working, matches!(why, Some(Why::Woken)))
            .await?
            .map(Addition::Append)),
        (Role::Investigator, _) => investigator_section(ctx, found).await,
    }
}

/// Uses the same bus and validated home path for investigator and lead prompts.
fn investigation_bus(ctx: &Context<'_>) -> Result<agent_runtime::agentbus::AgentBus, SectionError> {
    let home_str = ctx.home.root().to_str().ok_or_else(|| SectionError::Path {
        path: ctx.home.root().to_path_buf(),
        source: None,
    })?;
    let home = AbsolutePath::from_str(home_str).map_err(|source| SectionError::Path {
        path: ctx.home.root().to_path_buf(),
        source: Some(source),
    })?;
    Ok(agent_runtime::agentbus::AgentBus::new(
        ctx.settings.config.binaries().agentbus.clone(),
        home,
    ))
}

/// Renders the prompt section for an investigator task.
async fn investigator_section(ctx: &Context<'_>, found: &TaskRef<'_>) -> Result<Option<Addition>, SectionError> {
    let bus = investigation_bus(ctx)?;
    let open = bus.inv_open().await?;
    if let Some(inv) = open
        .into_iter()
        .find(|i| i.inv_task.as_ref() == Some(found.member.task()))
    {
        if let agent_runtime::agentbus::InvestigationKind::Split { item } = inv.kind {
            let values = Variables::new()
                .with("item", item.to_string())?
                .with("item_text", found.team.task_list().text())?
                .with("part_limit", ctx.settings.config.split().part_limit.to_string())?
                .with("question", &inv.question)?;
            let text = ctx.texts.render(HarnessPrompt::InvestigationSplit, &values)?;
            return Ok(Some(Addition::Replace(text.trim_end().to_owned())));
        }
        if let agent_runtime::agentbus::InvestigationKind::Conflict { item } = inv.kind {
            let pr = super::super::investigations::conflict_pull_request(found.team, item)
                .ok_or(SectionError::ConflictPullRequest { item })?;
            let configured = ctx
                .settings
                .config
                .repo(found.team.repo())
                .ok_or_else(|| SectionError::Repo {
                    repo: found.team.repo().clone(),
                })?;
            let values = Variables::new()
                .with("item", item.to_string())?
                .with("pr_number", pr.number.to_string())?
                .with("branch", pr.branch.as_str())?
                .with("base", configured.base_branch.as_str())?
                .with("question", &inv.question)?;
            let text = ctx.texts.render(HarnessPrompt::InvestigationConflict, &values)?;
            return Ok(Some(Addition::Replace(text.trim_end().to_owned())));
        }
        let values = Variables::new()
            .with("investigation_id", inv.id.to_string())?
            .with("from_agent", inv.from_agent.as_str())?
            .with("branch", inv.branch.as_str())?
            .with("question", &inv.question)?
            .with("context", &inv.context)?;
        let text = ctx.texts.render(HarnessPrompt::InvestigationRequest, &values)?;
        Ok(Some(Addition::Replace(text.trim_end().to_owned())))
    } else {
        Ok(None)
    }
}

/// Reads the bounded answer used by an additional conflict pass.
async fn conflict_investigator_answer(path: &std::path::Path) -> Result<String, SectionError> {
    match read_text_file(path, MAX_REPORT_BYTES)
        .await
        .map_err(|source| SectionError::Report {
            path: path.to_path_buf(),
            source,
        })? {
        Some(file) => Ok(format!(
            "An investigator worked out this resolution on the tip of your pull request. Apply it, check it, then go on from step 4:\n{}",
            file.text
        )),
        None => Err(SectionError::MissingInvestigationAnswer {
            path: path.to_path_buf(),
        }),
    }
}

/// What the lead of `found` is told about an approved pull request that GitHub refuses to merge for a conflict: the merge of
/// the base it is sent to make, or the question it is to put to the human once the passes are used up; `None` when no
/// merge is in trouble.
///
/// # Errors
/// [`SectionError`].
async fn merge_conflict_text(
    ctx: &Context<'_>,
    texts: &AutopilotPrompts,
    found: &TaskRef<'_>,
) -> Result<Option<String>, SectionError> {
    let troubled = found
        .team
        .items()
        .iter()
        .find_map(|item| match (item.phase(), item.merge_conflict()) {
            (ItemPhase::Approved { pr, .. }, Some(conflict)) => {
                Some((pr, conflict, item.merge_passes(), item.conflict_investigation()))
            }
            (ItemPhase::Approved { .. }, None)
            | (
                ItemPhase::Pending
                | ItemPhase::InProgress { .. }
                | ItemPhase::InReview { .. }
                | ItemPhase::Fixing { .. }
                | ItemPhase::Done { .. },
                _,
            ) => None,
        });
    let Some((pr, conflict, passes, investigation)) = troubled else {
        return Ok(None);
    };
    let repo = found.team.repo();
    let base = ctx
        .settings
        .config
        .repo(repo)
        .map(|configured| configured.base_branch.clone())
        .ok_or_else(|| SectionError::Repo { repo: repo.clone() })?;
    let values = Variables::new()
        .with("pr", pr.number.to_string())?
        .with("base", base.as_str())?
        .with(
            "passes",
            if investigation.is_some_and(|record| record.answered) {
                passes.to_string()
            } else {
                ctx.settings.config.merge().conflict_passes.to_string()
            },
        )?;
    let text = match conflict {
        MergeConflict::MergingBase { .. } => {
            let answer = if let Some(record) = investigation.filter(|record| record.answered) {
                let bus = investigation_bus(ctx)?;
                let request = bus.inv_get(record.id).await?;
                conflict_investigator_answer(&request.file).await?
            } else {
                String::new()
            };
            let values = values
                .with("branch", pr.branch.as_str())?
                .with("pass", passes.to_string())?
                .with("investigator_answer", answer)?;
            texts.render(AutopilotPrompt::PrMergeConflict, &values)?
        }
        MergeConflict::AskingHuman { at } => {
            if crate::daemon::merge::ran_since(found.member, *at) {
                return Ok(None);
            }
            texts.render(AutopilotPrompt::PrMergeConflictAsk, &values)?
        }
    };
    Ok(Some(text.trim_end().to_owned()))
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

/// The item that the lead `found` is fixing, with the branch and the pull request of the pass.
fn fixing_item<'a>(found: &TaskRef<'a>) -> Option<(&'a ItemProgress, &'a BranchName, &'a PullRequest)> {
    let name = found.member.agent().name();
    found.team.items().iter().find_map(|progress| match progress.phase() {
        ItemPhase::Fixing { assignee, branch, pr } if assignee.as_str() == name => Some((progress, branch, pr)),
        ItemPhase::Fixing { .. }
        | ItemPhase::Pending
        | ItemPhase::InProgress { .. }
        | ItemPhase::InReview { .. }
        | ItemPhase::Approved { .. }
        | ItemPhase::Done { .. } => None,
    })
}

/// The fix pass of the lead `found`, if an item is being fixed by it: the reports of the reviewers of the pass, or, for a
/// lead that was `woken` and was given the pass already, one line that says the pass is still open. When the worktree
/// `working` of the lead has another branch than the item's out, which the harness could not switch because the tree has
/// changes, the text that tells the lead to commit them and switch comes first; when git cannot say which branch is out, the
/// pass is given without it.
///
/// # Errors
/// [`SectionError`].
pub(super) async fn fix_pass(
    ctx: &Context<'_>,
    texts: &AutopilotPrompts,
    found: &TaskRef<'_>,
    working: &std::path::Path,
    woken: bool,
) -> Result<Option<String>, SectionError> {
    let Some(text) = fix_pass_text(ctx, texts, found, woken).await? else {
        return Ok(None);
    };
    let Some((_, branch, _)) = fixing_item(found) else {
        return Ok(Some(text));
    };
    let git = git_of(ctx, found, working)?;
    Ok(Some(match other_branch_text(texts, &git, branch).await? {
        Some(other) => format!("{}\n\n{text}", other.trim_end()),
        None => text,
    }))
}

/// The text that tells the lead its worktree, in `git`, has another branch out than its item's `branch`, which the harness
/// could not switch; `None` when the item's branch is out. A branch that git cannot read is logged and counts as no text: the
/// same policy as the checkout before a run, and the text about another branch would be a guess.
async fn other_branch_text(
    texts: &AutopilotPrompts,
    git: &Git,
    branch: &BranchName,
) -> Result<Option<String>, SectionError> {
    let checked_out = match git.current_branch().await {
        Ok(checked_out) => checked_out,
        Err(error) => {
            tracing::warn!(
                event = LogEvent::GitCallFailed.as_str(),
                to = %branch,
                error = %agent_runtime::process::error_chain(&error),
                "the branch the worktree has out cannot be read: the fix pass does not say which one it is"
            );
            return Ok(None);
        }
    };
    if checked_out.as_ref() == Some(branch) {
        return Ok(None);
    }
    let values = Variables::new()
        .with("checked_out", super::super::item_branch::named(checked_out.as_ref()))?
        .with("branch", branch.as_str())?;
    Ok(Some(texts.render(AutopilotPrompt::ReviewFixPassOtherBranch, &values)?))
}

/// The text of the fix pass of the lead `found`, without what tells it of the branch its worktree has out.
async fn fix_pass_text(
    ctx: &Context<'_>,
    texts: &AutopilotPrompts,
    found: &TaskRef<'_>,
    woken: bool,
) -> Result<Option<String>, SectionError> {
    let Some((progress, branch, pr)) = fixing_item(found) else {
        return Ok(None);
    };
    let passes = progress.fix_passes().len();
    let findings_file = ctx.home.fix_pass_findings_file_of(found.member.task(), passes);
    if let Some(correction) = progress
        .findings()
        .and_then(|l| l.disputes())
        .and_then(|d| d.correction.as_ref())
    {
        // A pass begun by an older daemon has no file: it is written from what the state holds, and then named.
        if let Some(asked) = progress.fix_passes().last() {
            ensure_findings_file(ctx, texts, progress, asked, pr.number, &findings_file).await?;
        }
        return Ok(Some(named_findings_file(texts, correction.clone(), &findings_file)?));
    }
    let Some(asked) = progress.fix_passes().last() else {
        return Ok(None);
    };
    let asked_at = asked.requested_by.iter().map(|request| request.requested_at).max();
    if woken && asked_at.is_some_and(|at| found.member.runs().iter().any(|run| run.started_at() >= at)) {
        let values = Variables::new()
            .with("pr", pr.number.to_string())?
            .with("pass", passes.to_string())?;
        let open = texts.render(AutopilotPrompt::ReviewFixPassOpen, &values)?;
        ensure_findings_file(ctx, texts, progress, asked, pr.number, &findings_file).await?;
        return Ok(Some(named_findings_file(
            texts,
            open.trim_end().to_owned(),
            &findings_file,
        )?));
    }
    let findings = findings_block(ctx, texts, progress, asked, pr.number).await?;
    // The same text goes to a file the prompts of the pass name: a lead that has lost it reads it again.
    keep_findings_file(&findings_file, &findings).await;
    let values = Variables::new()
        .with("pr", pr.number.to_string())?
        .with("pass", passes.to_string())?
        .with("passes", ctx.settings.config.review().fix_passes.to_string())?
        .with("branch", branch.as_str())?
        .with("findings", findings)?;
    let text = texts
        .render(
            if progress.findings().is_some_and(|l| l.disputes().is_some()) {
                AutopilotPrompt::ReviewFixResponse
            } else {
                AutopilotPrompt::ReviewFixPass
            },
            &values,
        )?
        .trim_end()
        .to_owned();
    let mut text = named_findings_file(texts, text, &findings_file)?;
    if matches!(found.role, Role::Member | Role::Worker) {
        let integration = texts.render(
            AutopilotPrompt::ReviewMemberFix,
            &Variables::new().with("integration", pr.branch.as_str())?,
        )?;
        text.push_str("\n\n");
        text.push_str(integration.trim_end());
    }
    Ok(Some(text))
}

/// The findings of the pass `asked` as the lead is given them: the open findings, the offer of a second opinion and the
/// reports of the reviewers.
async fn findings_block(
    ctx: &Context<'_>,
    texts: &AutopilotPrompts,
    progress: &ItemProgress,
    asked: &agent_core::state::FixPass,
    pr: PrNumber,
) -> Result<String, SectionError> {
    // The reports of the review pass that asked for this pass were kept under the number of the passes before it.
    let kept = count(progress.completed_review_passes().saturating_sub(1));
    let mut findings = vec![super::findings::open_text(texts, progress.findings())?];
    if ctx.settings.config.review().second_opinion() == agent_core::review_response::opinion::Permission::Enabled {
        findings.push(texts.render(AutopilotPrompt::ReviewOpinionOffer, &Variables::new())?);
    }
    findings.extend(reports_of(ctx, texts, &asked.requested_by, pr, kept).await?);
    Ok(findings.join("\n\n"))
}

/// Writes `findings` to `file`. The findings are in the prompt either way: a file that cannot be written is a warning, and
/// a prompt names only a file that is there.
async fn keep_findings_file(file: &std::path::Path, findings: &str) {
    if let Err(error) = replace_text_file(file, findings, 0o600).await {
        tracing::warn!(path = %file.display(), error = %agent_runtime::process::error_chain(&error), "cannot write the findings of the fix pass to a file");
    }
}

/// Writes the findings file of the pass when it is not there: a pass begun by a daemon that kept none.
async fn ensure_findings_file(
    ctx: &Context<'_>,
    texts: &AutopilotPrompts,
    progress: &ItemProgress,
    asked: &agent_core::state::FixPass,
    pr: PrNumber,
    file: &std::path::Path,
) -> Result<(), SectionError> {
    if file.is_file() {
        return Ok(());
    }
    let findings = findings_block(ctx, texts, progress, asked, pr).await?;
    keep_findings_file(file, &findings).await;
    Ok(())
}

/// `text` with the line that names `file` after it, when the file is there.
fn named_findings_file(texts: &AutopilotPrompts, text: String, file: &std::path::Path) -> Result<String, SectionError> {
    Ok(match findings_file_line(texts, file)? {
        Some(line) => format!("{}\n\n{line}", text.trim_end()),
        None => text,
    })
}

/// The line that names `file`, which holds the full text of the findings of the fix pass, when the file is there: a pass
/// begun before the file was kept, or one whose file could not be written, names nothing that is not.
pub(in crate::daemon::owner) fn findings_file_line(
    texts: &AutopilotPrompts,
    file: &std::path::Path,
) -> Result<Option<String>, super::responses::ResponseTextError> {
    let present = match std::fs::metadata(file) {
        Ok(found) => found.is_file(),
        // A pass begun before the file was kept has none: nothing is named.
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
        Err(error) => {
            tracing::warn!(path = %file.display(), error = %error, "the findings file of the fix pass cannot be looked at: it is not named");
            false
        }
    };
    if !present {
        return Ok(None);
    }
    // The home is a path of the configuration, which is text.
    let values = Variables::new().with("path", file.display().to_string())?;
    Ok(Some(
        texts
            .render(AutopilotPrompt::ReviewFixPassFile, &values)?
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
        findings.push(super::findings::open_text(texts, progress.findings())?);
        let kept = count(progress.completed_review_passes());
        findings.extend(reports_of(ctx, texts, reviews, waits.pr.number, kept).await?);
    }
    let file = ctx.home.questions_file(found.member.task(), number);
    let values = Variables::new()
        .with("pr", waits.pr.number.to_string())?
        .with("passes", waits.passes.to_string())?
        .with(
            "additional_passes",
            waits
                .items
                .first()
                .and_then(|id| team.item(*id))
                .and_then(|item| item.findings())
                .and_then(|ledger| ledger.verification())
                .map_or(1, |_| ctx.settings.config.review().fix_passes)
                .to_string(),
        )?
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

/// The kept reports of the reviews in `reviews` that are in, each as a finding with its verdict: the lead is given what
/// every reviewer of the pass said, not only the requests for changes.
pub(super) async fn reports_of(
    ctx: &Context<'_>,
    texts: &AutopilotPrompts,
    reviews: &[Review],
    pr: PrNumber,
    kept: u32,
) -> Result<Vec<String>, SectionError> {
    let mut findings = Vec::new();
    for review in reviews {
        if !matches!(review.outcome, ReviewOutcome::Submitted { .. }) {
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

/// What a review request says of the earlier passes of its pull request.
pub(super) struct Verification<'a> {
    /// The review pass the request is for, counting from one.
    pub(super) pass: usize,
    /// The findings ledger of the pull request, when it has one.
    pub(super) findings: Option<&'a Findings>,
}

/// The pass and the findings of the pull request that the `items` of a request share.
///
/// The pass is the one after the most fix passes of any item: an item that joined a fix pass has none of its own, and
/// whatever the order of the task list, the report validation demands answers for the ledger that the request shows.
pub(super) fn verification_of<'a>(items: &[&'a ItemProgress]) -> Verification<'a> {
    let passes = items
        .iter()
        .map(|item| item.completed_review_passes())
        .max()
        .unwrap_or(0);
    Verification {
        pass: passes.saturating_add(1),
        findings: items.iter().find_map(|item| item.findings()),
    }
}

/// The request for a review that the reviewer `found` has pending, with the items of the pull request.
pub(super) async fn review_request(
    ctx: &Context<'_>,
    texts: &AutopilotPrompts,
    found: &TaskRef<'_>,
    working: &std::path::Path,
    zone: Zone,
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
    let record = if let Some((_, lead)) = team.crew().roster().first() {
        agent_runtime::store::read_gate_record(&ctx.home.gate_record_file(lead.task())).await?
    } else {
        None
    };
    let verification = verification_of(&items);
    let values = Variables::new()
        .with("team", team.id().as_str())?
        .with("pr", pr.number.to_string())?
        .with("branch", pr.branch.as_str())?
        .with("base", base.as_str())?
        .with("head", head.as_str())?
        .with("pass", verification.pass.to_string())?;
    let context = verification
        .findings
        .and_then(|ledger| ledger.verification())
        .filter(|_| verification.pass > 1);
    let mut sections = if let Some(context) = context {
        let changes = verification_changes(
            &git_of(ctx, found, working)?,
            texts,
            context.reviewed_head.as_ref(),
            base.as_str(),
        )
        .await?;
        let values = values
            .with("changes", changes)?
            .with("lead_message", context.lead_message.as_deref().unwrap_or_default())?;
        vec![texts.render(AutopilotPrompt::ReviewVerification, &values)?]
    } else {
        vec![texts.render(AutopilotPrompt::ReviewRequest, &values)?]
    };
    sections.push(review_gate_in(texts, &head, record.as_ref(), zone)?);
    if crate::daemon::review::independent_first_review(team, task) {
        sections.push(texts.render(AutopilotPrompt::ReviewIndependent, &Variables::new())?);
    }
    if verification.pass > 1 && verification.findings.is_some() {
        let values = Variables::new().with(
            "findings",
            super::findings::verification_text(texts, verification.findings)?,
        )?;
        sections.push(texts.render(
            if verification.findings.is_some_and(|l| l.disputes().is_some()) {
                AutopilotPrompt::ReviewDisputes
            } else {
                AutopilotPrompt::ReviewOpenFindings
            },
            &values,
        )?);
    }
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
    Ok(git_of(ctx, found, working)?.rev_parse(&Revision::Head).await?)
}

/// Opens git in the reviewer's worktree with its configured identity.
fn git_of(ctx: &Context<'_>, found: &TaskRef<'_>, working: &std::path::Path) -> Result<Git, SectionError> {
    let text = working.to_str().ok_or_else(|| SectionError::Path {
        path: working.to_path_buf(),
        source: None,
    })?;
    let worktree = AbsolutePath::from_str(text).map_err(|source| SectionError::Path {
        path: working.to_path_buf(),
        source: Some(source),
    })?;
    Ok(Git::new(
        worktree,
        CommitIdentity::for_agent(ctx.settings.config.git(), &found.member.agent().name().parse()?),
    ))
}

/// Uses the reviewed commit when it remains an ancestor, otherwise explains the base diff.
pub(super) async fn verification_changes(
    git: &Git,
    texts: &AutopilotPrompts,
    previous: Option<&CommitId>,
    base: &str,
) -> Result<String, SectionError> {
    if let Some(previous) = previous
        && git
            .is_ancestor(&Revision::Commit(previous.clone()), &Revision::Head)
            .await?
    {
        return Ok(texts.render(
            AutopilotPrompt::ReviewVerificationChanges,
            &Variables::new().with("previous", previous.as_str())?,
        )?);
    }
    Ok(texts.render(
        AutopilotPrompt::ReviewVerificationRewritten,
        &Variables::new().with("base", base)?,
    )?)
}

/// Supplies only a gate recorded at the reviewed head; otherwise requests the normal gate.
#[cfg(test)]
fn review_gate(
    texts: &AutopilotPrompts,
    head: &CommitId,
    record: Option<&agent_core::state::GateRecord>,
) -> Result<String, SectionError> {
    review_gate_in(texts, head, record, Zone::Utc)
}

/// Supplies only a gate recorded at the reviewed head, formatted in `zone`; otherwise requests the normal gate.
fn review_gate_in(
    texts: &AutopilotPrompts,
    head: &CommitId,
    record: Option<&agent_core::state::GateRecord>,
    zone: Zone,
) -> Result<String, SectionError> {
    let values = Variables::new().with("head", head.as_str())?;
    let (prompt, values) = if let Some(record) = record.filter(|record| record.result.head == *head) {
        (
            AutopilotPrompt::ReviewGate,
            values
                .with("finished", zone.stamp(record.result.finished_at))?
                .with("result", record.summary())?
                .with("log", record.result.log.as_path().display().to_string())?,
        )
    } else {
        (AutopilotPrompt::ReviewGateMissing, values)
    };
    Ok(texts.render(prompt, &values)?.trim_end().to_owned())
}

#[cfg(test)]
mod tests;
