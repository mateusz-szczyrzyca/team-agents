//! The owner's side of the autopilot: publishing the branch of a team as a pull request, asking investigators from
//! outside the team to review it, taking their verdicts, giving the team its fix passes, asking the human when they are
//! used up, and merging (or, with `--no-merge`, telling the human).
//!
//! What is decided is in `daemon/pr`, `daemon/review` and `daemon/merge`; the state of each round is the state of the
//! items, so a daemon that restarts finds the same things to do at its first look. Without the token of a GitHub account
//! in `secrets.toml` the autopilot is off: pull requests are not opened and the items stay where they are.

use std::collections::{HashMap, HashSet};
use std::str::FromStr;
use std::sync::Arc;

use agent_core::config::{AbsolutePath, GithubLogin, InvalidValue, RepoName, Secret};
use agent_core::decide::round::{RoundError, RoundEvent, RoundPolicy, RoundState, Stage};
use agent_core::ids::{AgentName, BranchName, GenerateIdError, InvalidBranchName, InvalidName, ItemId, TaskId, TeamId};
use agent_core::kinds::Role;
use agent_core::prompt::{AutopilotPrompt, Variables, VariablesError};
use agent_core::protocol::EventKind;
use agent_core::state::{
    CommitId, Completion, GateRecord, ItemPhase, Member, Participant, PrNumber, PullRequest, Review, ReviewOutcome,
    TaskRef, Timestamp,
};
use agent_core::status::ReviewVerdict;
use agent_runtime::autopilot_prompts::{AutopilotPrompts, RenderAutopilotTextError};
use agent_runtime::git::{CommitIdentity, Git, GitError, Revision, WorktreeBranch};
use agent_runtime::github::{
    ApiBase, CommentReview, GitHub, GitHubError, GitHubSettings, InvalidApiBase, InvalidRepoSlug, MergeMethod,
    MergeRequest, NewPullRequest, PullRequestUpdate, PullState, Readiness, Rejection, RepoSlug,
};
use agent_runtime::notify::{InvalidNotification, Notification};
use agent_runtime::process::error_chain;
use agent_runtime::run::compose::{ComposeError, PromptTexts, item_sections};
use agent_runtime::settings::Settings;
use agent_runtime::store::StoreError;
use agent_runtime::text_file::{TextFileError, read_text_file};

use super::start::{Context, next_round};
use super::teams::Round;
use super::{Located, Notice, Owner, OwnerError};
use crate::daemon::merge::{
    Asking, Decision, Exhausted, Next, VerdictError, after_answer, after_verdict, asking, decision, exhausted,
    fix_delivery, review_deliveries,
};
use crate::daemon::pr::{Joining, Publication, Publish, PullTextError, plan_publications, pull_text};
use crate::daemon::review::{Slot, VerdictProblem, choose_reviewers, free_reviewers, parse_verdict, report_text};
use crate::daemon::team::Why;
use crate::daemon::team::{agent_name, task_branch};

/// Why the autopilot cannot be set up from the settings.
#[derive(Debug, thiserror::Error)]
pub(crate) enum PullsError {
    /// The address of the GitHub API in `agents.toml` cannot be used.
    #[error("the address of the GitHub API in agents.toml cannot be used")]
    ApiBase(#[source] InvalidApiBase),
}

/// What the autopilot needs besides the state: its texts and the way to GitHub.
#[derive(Debug, Clone)]
pub(crate) struct Pulls {
    /// The texts of the pull requests and the reviews.
    texts: Arc<AutopilotPrompts>,
    /// Where GitHub is and how a call is made.
    settings: GitHubSettings,
    /// The token of the account the harness acts as.
    token: Secret,
    /// The accounts whose words are taken as instructions.
    trusted: Vec<GithubLogin>,
}

impl Pulls {
    /// The autopilot of `settings`; `None` when `secrets.toml` has no token of a GitHub account.
    ///
    /// # Errors
    /// [`PullsError`] when the address of the API is not one that is safe for the token.
    pub(crate) fn new(texts: Arc<AutopilotPrompts>, settings: &Settings) -> Result<Option<Self>, PullsError> {
        let Some(token) = settings.secrets.github_token() else {
            return Ok(None);
        };
        let github = settings.config.github();
        let base = match &github.api_base {
            Some(url) => url.as_str().parse::<ApiBase>().map_err(PullsError::ApiBase)?,
            None => ApiBase::public(),
        };
        Ok(Some(Self {
            texts,
            settings: GitHubSettings {
                base,
                ..GitHubSettings::default()
            },
            token: token.clone(),
            trusted: github.trusted_accounts.clone(),
        }))
    }

    /// The texts of the pull requests and the reviews.
    pub(super) fn texts(&self) -> &AutopilotPrompts {
        &self.texts
    }

    /// A client of the repository `slug`.
    ///
    /// # Errors
    /// [`GitHubError::Client`] when the HTTP client cannot be built.
    pub(super) fn client(&self, slug: RepoSlug) -> Result<GitHub, GitHubError> {
        GitHub::new(self.settings.clone(), slug, &self.token, &self.trusted)
    }
}

/// Why what the autopilot adds to a prompt cannot be made.
#[derive(Debug, thiserror::Error)]
pub(super) enum SectionError {
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
pub(super) enum Addition {
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
pub(super) async fn sections_for(
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
            let text = match fix_pass(ctx, texts, found).await? {
                Some(text) => Some(text),
                None => ask_human(ctx, texts, found).await?,
            };
            Ok(text.map(Addition::Append))
        }
        (Role::Member | Role::Worker, _) => Ok(None),
    }
}

/// The fix pass of the lead `found`, if an item is being fixed by it: the reports of the reviewers that asked for changes.
async fn fix_pass(
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
    // The reports of the round that asked for this pass were kept under the number of the passes before it.
    let kept = u32::try_from(passes.saturating_sub(1)).unwrap_or(u32::MAX);
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
async fn ask_human(
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
    let Some(round) = next_round(found.member) else {
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
        let kept = u32::try_from(progress.fix_passes().len()).unwrap_or(u32::MAX);
        findings.extend(reports_of(ctx, texts, reviews, waits.pr.number, kept).await?);
    }
    let file = ctx.home.questions_file(found.member.task(), round);
    let values = Variables::new()
        .with("pr", waits.pr.number.to_string())?
        .with("passes", waits.passes.to_string())?
        .with("findings", findings.join("\n\n"))?
        .with("questions_file", file.display().to_string())?
        .with("round", round.to_string())?
        .with("task", found.member.task().as_str())?;
    Ok(Some(
        texts
            .render(AutopilotPrompt::ReviewAskHuman, &values)?
            .trim_end()
            .to_owned(),
    ))
}

/// The kept reports of the reviews in `reviews` that asked for changes, each as a finding.
async fn reports_of(
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
const MAX_REPORT_BYTES: usize = 1 << 20;

/// The request for a review that the reviewer `found` has pending, with the items of the pull request.
async fn review_request(
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
    let head = git.rev_parse(&Revision::Head).await?;
    let round = first.fix_passes().len().saturating_add(1);
    let values = Variables::new()
        .with("team", team.id().as_str())?
        .with("pr", pr.number.to_string())?
        .with("branch", pr.branch.as_str())?
        .with("base", base.as_str())?
        .with("head", head.as_str())?
        .with("round", round.to_string())?;
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

/// How long a branch that could not be published is left alone before the next attempt.
const PUBLISH_BACKOFF: time::Duration = time::Duration::seconds(60);

/// What the autopilot remembers between two looks and the state does not keep: when to try again, and what the human has
/// been told.
#[derive(Debug, Default)]
pub(super) struct PullWatch {
    /// The branches whose publication failed, and the time before which they are not tried again.
    retry_at: HashMap<(TeamId, BranchName), Timestamp>,
    /// What the human was told once and is not told again.
    announced: HashSet<Announcement>,
    /// How many times each reviewer's report was sent back for lacking a verdict.
    bounces: HashMap<TaskId, u32>,
    /// When each approved pull request was last looked at, so that one that waits is not asked about at every look.
    polled: HashMap<(TeamId, PrNumber), Timestamp>,
    /// The pull requests whose merge GitHub refused: they are not merged again, only looked at for the human's merge.
    refused: HashSet<(TeamId, PrNumber)>,
}

/// How many times a report without a verdict is sent back before the human is told.
const MAX_VERDICT_BOUNCES: u32 = 2;

/// How long an approved pull request that waits (for the human, or for a merge that failed) is left alone between two
/// looks at it.
const POLL_EVERY: time::Duration = time::Duration::seconds(30);

/// Something the human is told once.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(super) enum Announcement {
    /// The pull request is approved and the team does not merge by itself.
    Approved {
        /// The team.
        team: TeamId,
        /// The pull request.
        pr: PrNumber,
    },
    /// GitHub refused to merge the pull request.
    MergeFailed {
        /// The team.
        team: TeamId,
        /// The pull request.
        pr: PrNumber,
    },
    /// The pull request was closed without being merged.
    Closed {
        /// The team.
        team: TeamId,
        /// The pull request.
        pr: PrNumber,
    },
    /// The reviewers still ask for changes, the fix passes are used up and the lead put no question.
    Decision {
        /// The team.
        team: TeamId,
        /// The pull request.
        pr: PrNumber,
        /// When the last request for changes came.
        requested_at: Timestamp,
    },
    /// A reviewer ended its review again and again without a verdict.
    NoVerdict {
        /// The reviewer's task.
        task: TaskId,
    },
    /// A branch could not be published.
    PublishFailed {
        /// The team.
        team: TeamId,
        /// The branch.
        branch: BranchName,
    },
}

/// Why a branch could not be published: what the human is told, and what is logged.
#[derive(Debug, thiserror::Error)]
enum PublishError {
    /// A path of the home is not a path git can be run in.
    #[error("the worktree {path:?} cannot be used")]
    Path {
        /// The path.
        path: std::path::PathBuf,
        /// What is wrong with it, if the text was refused.
        source: Option<InvalidValue>,
    },
    /// Git refused.
    #[error("git failed")]
    Git(#[from] GitError),
    /// The origin of the repository is not a repository on GitHub.
    #[error("the origin of the repository is not a repository on GitHub")]
    Origin(#[from] InvalidRepoSlug),
    /// GitHub refused or did not answer.
    #[error("GitHub failed")]
    GitHub(#[from] GitHubError),
    /// A file of the home cannot be read.
    #[error("cannot read what the gate recorded")]
    Store(#[from] StoreError),
    /// The text of the pull request cannot be made.
    #[error("cannot write the pull request")]
    Text(#[from] PullTextError),
    /// A text of the harness cannot be rendered.
    #[error("cannot render a text")]
    Render(#[from] RenderAutopilotTextError),
    /// A variable name of the program is not valid.
    #[error("a variable of a text is not valid")]
    Variable(#[from] VariablesError),
    /// The pull request carries another commit than the gated one.
    #[error("the pull request does not carry the commit the gate passed on")]
    Round(#[from] agent_core::decide::round::RoundError),
    /// The repository of the team is not in the configuration.
    #[error("the repository {repo} of the team is not in the configuration")]
    RepoGone {
        /// The repository.
        repo: RepoName,
    },
    /// An ID cannot be made.
    #[error("cannot make an ID")]
    Id(#[from] GenerateIdError),
    /// A name is not valid.
    #[error("a name is not valid")]
    Name(#[from] InvalidName),
    /// A branch name is not valid.
    #[error("a branch name is not valid")]
    Branch(#[from] InvalidBranchName),
    /// The notification cannot be made.
    #[error("the notification is not valid")]
    Notification(#[from] InvalidNotification),
    /// A reviewer cannot be added to the team.
    #[error("a reviewer cannot be added to the team")]
    Reviewer(#[from] agent_core::state::AddReviewerError),
    /// The reviews cannot be asked of an item.
    #[error("the reviews cannot be asked of an item")]
    Asking(#[from] agent_core::state::AskReviewsError),
    /// The team or an item of it is not in the state any more.
    #[error("the team or an item of it is not in the state")]
    Lost,
}

/// A pull request that was opened or updated.
struct Published {
    /// The pull request.
    pr: PullRequest,
    /// The commit the gate passed on, which was pushed.
    gated: CommitId,
    /// The commit GitHub says the pull request carries.
    carried: CommitId,
    /// The items that go in it, in the order of the list.
    items: Vec<ItemId>,
}

/// The facts about the lead of a team that a look at its pull requests needs.
struct Lead {
    /// The team's repository.
    repo: RepoName,
    /// The lead's task.
    task: TaskId,
    /// The lead's agent.
    agent: AgentName,
}

impl Owner {
    /// One look at the pull requests of `team`.
    ///
    /// # Errors
    /// [`OwnerError`] when the state cannot be saved or is found inconsistent.
    pub(super) async fn pull_requests(&mut self, team: &TeamId, round: &mut Round) -> Result<(), OwnerError> {
        let Some(pulls) = self.pulls.clone() else {
            return Ok(());
        };
        self.merge_approved(&pulls, team, round).await?;
        self.publish_branches(&pulls, team, round).await?;
        self.ask_for_reviews(team, round).await?;
        self.deliver_reviews(team, round)?;
        self.deliver_fix_passes(team, round)?;
        self.ask_the_human(&pulls, team, round)?;
        Ok(())
    }

    /// The lead of `team`: the coordinator, or the solo agent.
    fn lead_of(&self, team: &TeamId) -> Result<Lead, OwnerError> {
        let found = self
            .parts
            .state
            .team(team)
            .ok_or_else(|| OwnerError::LostTeam { team: team.clone() })?;
        let roster = found.crew().roster();
        let (_, lead) = roster
            .first()
            .ok_or_else(|| OwnerError::LostTeam { team: team.clone() })?;
        Ok(Lead {
            repo: found.repo().clone(),
            task: lead.task().clone(),
            agent: agent_name(lead.agent())?,
        })
    }

    /// Git in the worktree of `task` of a team on `repo`, committing as `agent`.
    fn worktree_git(&self, repo: &RepoName, task: &TaskId, agent: &AgentName) -> Result<Git, PublishError> {
        let path = self.parts.store.home().worktree_dir(repo, task);
        let text = path.to_str().ok_or_else(|| PublishError::Path {
            path: path.clone(),
            source: None,
        })?;
        let worktree = AbsolutePath::from_str(text).map_err(|source| PublishError::Path {
            path: path.clone(),
            source: Some(source),
        })?;
        Ok(Git::new(
            worktree,
            CommitIdentity::for_agent(self.parts.settings.config.git(), agent),
        ))
    }

    /// The client of GitHub for `repo`, whose slug is read from the origin of the checkout `git` works in.
    async fn github_for(&mut self, pulls: &Pulls, repo: &RepoName, git: &Git) -> Result<GitHub, PublishError> {
        if let Some(client) = self.github.get(repo) {
            return Ok(client.clone());
        }
        let slug = RepoSlug::from_remote_url(&git.origin_url().await?)?;
        let client = pulls.client(slug)?;
        self.github.insert(repo.clone(), client.clone());
        Ok(client)
    }

    // ---- publishing -----------------------------------------------------------------------------------------

    /// Publishes the branches of `team_id` that are ready: its pull requests are opened or updated, and its items go in
    /// review.
    async fn publish_branches(&mut self, pulls: &Pulls, team_id: &TeamId, round: &mut Round) -> Result<(), OwnerError> {
        let Some(team) = self.parts.state.team(team_id) else {
            return Ok(());
        };
        let plan = plan_publications(team).map_err(|source| OwnerError::Publications {
            team: team_id.clone(),
            source,
        })?;
        if plan.is_empty() {
            return Ok(());
        }
        let lead = self.lead_of(team_id)?;
        for publication in plan {
            let now = Timestamp::now(&self.parts.clock)?;
            let key = (team_id.clone(), publication.head_branch.clone());
            if self.watch.retry_at.get(&key).is_some_and(|retry| *retry > now) {
                continue;
            }
            match self.try_publish(pulls, team_id, &lead, &publication, now).await {
                Ok(None) => {}
                Ok(Some(published)) => {
                    self.watch.retry_at.remove(&key);
                    self.record_publication(team_id, &publication, &published, now, round)?;
                }
                Err(error) => self.publish_failed(pulls, team_id, &lead, &publication, &error, now)?,
            }
        }
        Ok(())
    }

    /// Pushes the branch of `publication` and opens or updates its pull request when the lead's gate passed at its tip;
    /// `None` when there is nothing to publish yet.
    async fn try_publish(
        &mut self,
        pulls: &Pulls,
        team_id: &TeamId,
        lead: &Lead,
        publication: &Publication,
        now: Timestamp,
    ) -> Result<Option<Published>, PublishError> {
        let git = self.worktree_git(&lead.repo, &lead.task, &lead.agent)?;
        let head_branch = &publication.head_branch;
        let Some(tip) = git.branch_tip(head_branch).await? else {
            return Ok(None);
        };
        let Some(record) = self.parts.store.load_gate_record(&lead.task).await? else {
            return Ok(None);
        };
        if !record.counts_as_pass() || record.result.head != tip {
            return Ok(None);
        }
        match &publication.publish {
            Publish::Open { items } => {
                let ids = contained(&git, items, &tip).await?;
                if ids.is_empty() {
                    return Ok(None);
                }
                let github = self.github_for(pulls, &lead.repo, &git).await?;
                git.push(head_branch).await?;
                let text = self.pull_text_of(pulls, team_id, &lead.repo, &ids, &record)?;
                let base = self.base_branch(&lead.repo)?;
                let remote = match github.find_open_pull_request(head_branch).await? {
                    Some(found) => found,
                    None => {
                        github
                            .create_pull_request(&NewPullRequest {
                                title: text.title,
                                body: text.body,
                                head: head_branch.clone(),
                                base,
                                readiness: Readiness::Draft,
                            })
                            .await?
                    }
                };
                github.mark_ready(remote.number).await?;
                Ok(Some(Published {
                    pr: PullRequest {
                        number: remote.number,
                        branch: head_branch.clone(),
                        opened_at: now,
                    },
                    gated: tip,
                    carried: remote.head,
                    items: ids,
                }))
            }
            Publish::Update { pr, fixing, joining } => {
                let joined = contained(&git, joining, &tip).await?;
                let github = self.github_for(pulls, &lead.repo, &git).await?;
                let before = github.get_pull_request(pr.number).await?;
                if before.head == tip && joined.is_empty() {
                    return Ok(None);
                }
                git.push(head_branch).await?;
                let mut ids = fixing.clone();
                ids.extend(joined.iter().copied());
                let ids = self.in_list_order(team_id, &ids);
                if !joined.is_empty() {
                    let text = self.pull_text_of(pulls, team_id, &lead.repo, &ids, &record)?;
                    github
                        .update_pull_request(
                            pr.number,
                            &PullRequestUpdate {
                                title: Some(text.title),
                                body: Some(text.body),
                                ..PullRequestUpdate::default()
                            },
                        )
                        .await?;
                }
                let pass = self.fix_passes_of(team_id, fixing);
                let comment = pulls.texts.render(
                    AutopilotPrompt::PrUpdate,
                    &Variables::new()
                        .with("head", tip.as_str())?
                        .with("pass", pass.to_string())?
                        .with("gate", record.summary())?,
                )?;
                github.post_comment(pr.number, comment.trim_end()).await?;
                let after = github.get_pull_request(pr.number).await?;
                Ok(Some(Published {
                    pr: pr.clone(),
                    gated: tip,
                    carried: after.head,
                    items: ids,
                }))
            }
        }
    }

    /// The title and the description of the pull request of `ids`.
    fn pull_text_of(
        &self,
        pulls: &Pulls,
        team_id: &TeamId,
        repo: &RepoName,
        ids: &[ItemId],
        record: &GateRecord,
    ) -> Result<crate::daemon::pr::PullText, PublishError> {
        let team = self
            .parts
            .state
            .team(team_id)
            .ok_or_else(|| PublishError::RepoGone { repo: repo.clone() })?;
        Ok(pull_text(&pulls.texts, team, repo, ids, record)?)
    }

    /// The branch the pull requests of `repo` are made against.
    fn base_branch(&self, repo: &RepoName) -> Result<BranchName, PublishError> {
        self.parts
            .settings
            .config
            .repo(repo)
            .map(|configured| configured.base_branch.clone())
            .ok_or_else(|| PublishError::RepoGone { repo: repo.clone() })
    }

    /// `ids` in the order of the task list of `team`.
    fn in_list_order(&self, team: &TeamId, ids: &[ItemId]) -> Vec<ItemId> {
        self.parts.state.team(team).map_or_else(Vec::new, |found| {
            found
                .items()
                .iter()
                .map(agent_core::state::ItemProgress::id)
                .filter(|id| ids.contains(id))
                .collect()
        })
    }

    /// How many fix passes the first of `items` has had.
    fn fix_passes_of(&self, team: &TeamId, items: &[ItemId]) -> usize {
        items
            .first()
            .and_then(|first| self.parts.state.team(team).and_then(|found| found.item(*first)))
            .map_or(0, |progress| progress.fix_passes().len())
    }

    /// Records that the items of `published` are in review, after the round machine checked that the pull request carries
    /// the commit the gate passed on.
    fn record_publication(
        &mut self,
        team_id: &TeamId,
        publication: &Publication,
        published: &Published,
        now: Timestamp,
        round: &mut Round,
    ) -> Result<(), OwnerError> {
        let Some(team) = self.parts.state.team(team_id) else {
            return Ok(());
        };
        let policy = RoundPolicy::new(team.merge(), self.parts.settings.config.review());
        let mut moving = Vec::new();
        for id in &published.items {
            let stage = match (&publication.publish, team.item(*id)) {
                (Publish::Update { fixing, .. }, Some(progress)) if fixing.contains(id) => {
                    let passes = u32::try_from(progress.fix_passes().len()).unwrap_or(u32::MAX);
                    Stage::Fixing {
                        pr: published.pr.number,
                        pass: std::num::NonZeroU32::MIN.saturating_add(passes.saturating_sub(1)),
                    }
                }
                _ => Stage::First,
            };
            let state = RoundState::Publishing {
                stage,
                head: published.gated.clone(),
            };
            let event = RoundEvent::PullRequestPublished {
                pr: published.pr.number,
                head: published.carried.clone(),
            };
            match state.transition(&event, policy) {
                Ok(_) => moving.push(*id),
                Err(error) => {
                    tracing::warn!(team = %team_id, item = %id, error = %error_chain(&error), "a pull request does not carry what the gate passed: it is looked at again");
                }
            }
        }
        for id in moving {
            let mut team_mut = self
                .parts
                .state
                .team_mut(team_id)
                .ok_or_else(|| OwnerError::LostTeam { team: team_id.clone() })?;
            let mut item = team_mut.item_mut(id).ok_or_else(|| OwnerError::LostItem {
                team: team_id.clone(),
                item: id,
            })?;
            item.advance(ItemPhase::InReview {
                pr: published.pr.clone(),
                reviews: Vec::new(),
            })
            .map_err(|source| OwnerError::Item {
                team: team_id.clone(),
                item: id,
                source,
            })?;
            // An item that was being fixed has its pull request already; the others have it opened for them now.
            let opened = !matches!(&publication.publish, Publish::Update { fixing, .. } if fixing.contains(&id));
            if opened {
                round.events.push(EventKind::PullRequestOpened {
                    team: team_id.clone(),
                    item: id,
                    pr: published.pr.number,
                });
            }
            round.events.push(EventKind::ItemStatus {
                team: team_id.clone(),
                item: id,
                status: agent_core::status::ItemStatus::InReview,
            });
            round.changed = true;
            tracing::info!(team = %team_id, item = %id, pr = %published.pr.number, at = %now, "an item is in review");
        }
        Ok(())
    }

    /// What follows a branch that could not be published: it is logged, tried again after a while, and the human is told
    /// once.
    fn publish_failed(
        &mut self,
        pulls: &Pulls,
        team_id: &TeamId,
        lead: &Lead,
        publication: &Publication,
        error: &PublishError,
        now: Timestamp,
    ) -> Result<(), OwnerError> {
        let reason = error_chain(error);
        tracing::warn!(team = %team_id, branch = %publication.head_branch, %reason, "a branch cannot be published: it is tried again in a minute");
        let retry = Timestamp::from_utc(now.as_utc() + PUBLISH_BACKOFF)?;
        self.watch
            .retry_at
            .insert((team_id.clone(), publication.head_branch.clone()), retry);
        let announcement = Announcement::PublishFailed {
            team: team_id.clone(),
            branch: publication.head_branch.clone(),
        };
        if !self.watch.announced.insert(announcement) {
            return Ok(());
        }
        let items = match &publication.publish {
            Publish::Open { items } => items.iter().map(|joining| joining.item.to_string()).collect::<Vec<_>>(),
            Publish::Update { fixing, joining, .. } => fixing
                .iter()
                .copied()
                .chain(joining.iter().map(|joining| joining.item))
                .map(|id| id.to_string())
                .collect(),
        };
        let values = Variables::new()
            .with("team", team_id.as_str())
            .and_then(|values| values.with("items", items.join(", ")))
            .and_then(|values| values.with("reason", &reason));
        self.tell_human(pulls, AutopilotPrompt::NoticePublishFailed, values, lead, now);
        Ok(())
    }

    /// Tells the human `prompt` rendered with `values`, in the name of the lead; what cannot be made is logged: the
    /// harness goes on.
    fn tell_human(
        &self,
        pulls: &Pulls,
        prompt: AutopilotPrompt,
        values: Result<Variables, VariablesError>,
        lead: &Lead,
        now: Timestamp,
    ) {
        let made = values
            .map_err(PublishError::from)
            .and_then(|values| notification(&pulls.texts, prompt, &values));
        let notification = match made {
            Ok(notification) => notification,
            Err(error) => {
                tracing::error!(?prompt, error = %error_chain(&error), "a notification of the autopilot cannot be made");
                return;
            }
        };
        match self.locate(&lead.task) {
            Ok(located) => self.notify(now, &located, &lead.task, Notice::Prepared(notification)),
            Err(error) => {
                tracing::error!(task = %lead.task, %error, "a notification of the autopilot has no task to be about")
            }
        }
    }

    // ---- reviewers ------------------------------------------------------------------------------------------

    /// Asks two free investigators from outside the team for the reviews of each pull request that has none asked.
    async fn ask_for_reviews(&mut self, team_id: &TeamId, round: &mut Round) -> Result<(), OwnerError> {
        let waiting = self.pull_requests_without_reviews(team_id);
        if waiting.is_empty() {
            return Ok(());
        }
        let lead = self.lead_of(team_id)?;
        for (pr, items) in waiting {
            let now = Timestamp::now(&self.parts.clock)?;
            match self.try_ask(team_id, &lead, &pr, &items, now).await {
                Ok(true) => round.changed = true,
                // Fewer than two investigators are free: the pull request waits for them.
                Ok(false) => {}
                Err(error) => {
                    tracing::warn!(team = %team_id, pr = %pr.number, error = %error_chain(&error), "the reviews of a pull request cannot be asked for: it is tried again at the next look");
                }
            }
        }
        Ok(())
    }

    /// The pull requests of `team` that are open and have no review asked, with their items.
    fn pull_requests_without_reviews(&self, team: &TeamId) -> Vec<(PullRequest, Vec<ItemId>)> {
        let mut found: Vec<(PullRequest, Vec<ItemId>)> = Vec::new();
        let Some(team) = self.parts.state.team(team) else {
            return found;
        };
        for progress in team.items() {
            let ItemPhase::InReview { pr, reviews } = progress.phase() else {
                continue;
            };
            if !reviews.is_empty() {
                continue;
            }
            match found.iter_mut().find(|(known, _)| known.number == pr.number) {
                Some((_, items)) => items.push(progress.id()),
                None => found.push((pr.clone(), vec![progress.id()])),
            }
        }
        found
    }

    /// Asks two reviewers for the reviews of `pr`; `false` when fewer than two are free.
    ///
    /// The worktrees of the reviewers are made, or moved to the commit under review, before the state records the
    /// reviews, so that a reviewer that is queued has a worktree to work in; the ones that were made are removed again
    /// when something later fails.
    async fn try_ask(
        &mut self,
        team_id: &TeamId,
        lead: &Lead,
        pr: &PullRequest,
        items: &[ItemId],
        now: Timestamp,
    ) -> Result<bool, PublishError> {
        let free = free_reviewers(&self.parts.state, &self.parts.settings.config, team_id);
        let Some(chosen) = choose_reviewers(free, &mut self.parts.rng) else {
            return Ok(false);
        };
        let git = self.worktree_git(&lead.repo, &lead.task, &lead.agent)?;
        let Some(head) = git.branch_tip(&pr.branch).await? else {
            return Ok(false);
        };
        let mut reviews = Vec::new();
        let mut made: Vec<(Member, AbsolutePath, BranchName)> = Vec::new();
        for reviewer in chosen {
            let agent: AgentName = reviewer.name.as_str().parse()?;
            match reviewer.slot {
                Slot::Task(task) => {
                    let moved = self.worktree_git(&lead.repo, &task, &agent)?;
                    if let Err(error) = moved.detach(&head).await {
                        self.undo(&git, &made).await;
                        return Err(error.into());
                    }
                    reviews.push(pending_review(&reviewer.name, &task, now));
                }
                Slot::New => {
                    let task = TaskId::generate(&self.parts.clock, &mut self.parts.rng)?;
                    match self.make_worktree(&git, lead, &agent, &task, &head).await {
                        Ok((path, branch)) => {
                            reviews.push(pending_review(&reviewer.name, &task, now));
                            made.push((
                                Member::new(task, Participant::Investigator(reviewer.name.clone())),
                                path,
                                branch,
                            ));
                        }
                        Err(error) => {
                            self.undo(&git, &made).await;
                            return Err(error);
                        }
                    }
                }
            }
        }
        for (member, _, _) in &made {
            if let Err(source) = self.parts.state.add_reviewer(team_id, member.clone()) {
                self.undo(&git, &made).await;
                return Err(PublishError::Reviewer(source));
            }
        }
        for id in items {
            let mut team_mut = self.parts.state.team_mut(team_id).ok_or(PublishError::Lost)?;
            let mut item = team_mut.item_mut(*id).ok_or(PublishError::Lost)?;
            item.ask_reviews(reviews.clone())?;
        }
        tracing::info!(team = %team_id, pr = %pr.number, reviewers = reviews.len(), "reviews are asked for");
        Ok(true)
    }

    /// Makes the worktree of a new reviewer at `head`, on a branch of its own.
    async fn make_worktree(
        &self,
        git: &Git,
        lead: &Lead,
        agent: &AgentName,
        task: &TaskId,
        head: &CommitId,
    ) -> Result<(AbsolutePath, BranchName), PublishError> {
        let path = self.parts.store.home().worktree_dir(&lead.repo, task);
        let text = path.to_str().ok_or_else(|| PublishError::Path {
            path: path.clone(),
            source: None,
        })?;
        let worktree = AbsolutePath::from_str(text).map_err(|source| PublishError::Path {
            path: path.clone(),
            source: Some(source),
        })?;
        let branch = task_branch(agent, task)?;
        git.worktree_add(
            &worktree,
            &WorktreeBranch::New {
                branch: branch.clone(),
                start: Revision::Commit(head.clone()),
            },
        )
        .await?;
        Ok((worktree, branch))
    }

    /// Removes the worktrees and branches `made` of reviewers that could not be asked; what cannot be removed is told.
    async fn undo(&self, git: &Git, made: &[(Member, AbsolutePath, BranchName)]) {
        for (member, path, branch) in made {
            if let Err(error) = git.worktree_remove(path, agent_runtime::git::RemoveMode::Force).await {
                tracing::warn!(task = %member.task(), error = %error_chain(&error), "the worktree of a reviewer that was not asked cannot be removed");
            }
            if let Err(error) = git.branch_delete(branch, agent_runtime::git::DeleteMode::Force).await {
                tracing::warn!(task = %member.task(), error = %error_chain(&error), "the branch of a reviewer that was not asked cannot be removed");
            }
        }
    }
}

impl Owner {
    // ---- the verdicts ---------------------------------------------------------------------------------------

    /// Hands the reviews that were asked of reviewers that wait to the reviewers: the task is queued, and its prompt tells
    /// what it is to review.
    fn deliver_reviews(&mut self, team_id: &TeamId, round: &mut Round) -> Result<(), OwnerError> {
        let Some(team) = self.parts.state.team(team_id) else {
            return Ok(());
        };
        for task in review_deliveries(team) {
            self.release(&task, Why::Review, round)?;
        }
        Ok(())
    }

    /// Gives the lead the fix pass that waits for it: its task is queued, and its prompt has the findings.
    fn deliver_fix_passes(&mut self, team_id: &TeamId, round: &mut Round) -> Result<(), OwnerError> {
        let Some(team) = self.parts.state.team(team_id) else {
            return Ok(());
        };
        if let Some(lead) = fix_delivery(team) {
            self.release(&lead, Why::FixPass, round)?;
        }
        Ok(())
    }

    /// What the pull requests that wait for the human need: the lead is woken to put the question; a lead that ran and put
    /// none leaves the human told directly; an answer gives one more fix pass.
    fn ask_the_human(&mut self, pulls: &Pulls, team_id: &TeamId, round: &mut Round) -> Result<(), OwnerError> {
        let Some(team) = self.parts.state.team(team_id) else {
            return Ok(());
        };
        let waiting = exhausted(team);
        if waiting.is_empty() {
            return Ok(());
        }
        let asks = asking(team);
        let answered: Vec<Exhausted> = {
            let roster = team.crew().roster();
            roster.first().map_or_else(Vec::new, |(_, lead)| {
                waiting
                    .iter()
                    .filter(|waits| decision(lead, waits.requested_at) == Decision::Answered)
                    .cloned()
                    .collect()
            })
        };
        let lead = self.lead_of(team_id)?;
        for ask in asks {
            match ask {
                Asking::Wake { lead, .. } => self.release(&lead, Why::AskHuman, round)?,
                Asking::Notify { exhausted } => self.tell_decision(pulls, team_id, &lead, &exhausted)?,
            }
        }
        for waits in &answered {
            self.start_pass_after_answer(team_id, &lead, waits, round)?;
        }
        Ok(())
    }

    /// Tells the human, once, that the reviewers still ask for changes and nobody put the question to them.
    fn tell_decision(
        &mut self,
        pulls: &Pulls,
        team_id: &TeamId,
        lead: &Lead,
        waits: &Exhausted,
    ) -> Result<(), OwnerError> {
        let announcement = Announcement::Decision {
            team: team_id.clone(),
            pr: waits.pr.number,
            requested_at: waits.requested_at,
        };
        if !self.watch.announced.insert(announcement) {
            return Ok(());
        }
        let now = Timestamp::now(&self.parts.clock)?;
        let values = Variables::new()
            .with("team", team_id.as_str())
            .and_then(|values| values.with("pr", waits.pr.number.to_string()))
            .and_then(|values| values.with("passes", waits.passes.to_string()));
        self.tell_human(pulls, AutopilotPrompt::NoticeDecision, values, lead, now);
        Ok(())
    }

    /// The human answered the question about `waits`: the round machine gives the lead one more fix pass.
    fn start_pass_after_answer(
        &mut self,
        team_id: &TeamId,
        lead: &Lead,
        waits: &Exhausted,
        round: &mut Round,
    ) -> Result<(), OwnerError> {
        let Some(team) = self.parts.state.team(team_id) else {
            return Ok(());
        };
        let policy = RoundPolicy::new(team.merge(), self.parts.settings.config.review());
        let mut moving = Vec::new();
        for id in &waits.items {
            let Some(progress) = team.item(*id) else {
                continue;
            };
            let next = after_answer(progress, policy);
            match next {
                Ok(Next::FixPass { .. }) => moving.push(*id),
                Ok(other) => {
                    tracing::warn!(team = %team_id, item = %id, ?other, "the round does not take the answer of the human as a fix pass");
                }
                Err(error) => {
                    tracing::warn!(team = %team_id, item = %id, error = %error_chain(&error), "the round does not take the answer of the human");
                }
            }
        }
        for id in moving {
            let mut team_mut = self
                .parts
                .state
                .team_mut(team_id)
                .ok_or_else(|| OwnerError::LostTeam { team: team_id.clone() })?;
            let mut item = team_mut.item_mut(id).ok_or_else(|| OwnerError::LostItem {
                team: team_id.clone(),
                item: id,
            })?;
            let after = ItemPhase::Fixing {
                assignee: lead.agent.clone(),
                branch: waits.pr.branch.clone(),
                pr: waits.pr.clone(),
            };
            let status = after.kind().status();
            item.advance(after).map_err(|source| OwnerError::Item {
                team: team_id.clone(),
                item: id,
                source,
            })?;
            round.events.push(EventKind::ItemStatus {
                team: team_id.clone(),
                item: id,
                status,
            });
            round.changed = true;
        }
        self.release(&lead.task, Why::FixPass, round)
    }

    /// Takes the final message of the reviewer `task`, whose run ended with `STATUS: DONE`: its verdict is recorded, posted
    /// on the pull request and acted on.
    ///
    /// # Errors
    /// [`OwnerError`] when the state or a file of the home cannot be written.
    pub(super) async fn review_ended(
        &mut self,
        task: &TaskId,
        located: &Located,
        message: Option<&str>,
    ) -> Result<(), OwnerError> {
        let Some(pulls) = self.pulls.clone() else {
            return Ok(());
        };
        let Some(message) = message else {
            return Ok(());
        };
        let started = self
            .parts
            .state
            .find_task(task)
            .and_then(|found| found.member.runs().last().map(agent_core::state::Run::started_at));
        let Some(started) = started else {
            return Ok(());
        };
        let asked = match self.answered_by(&located.team, task, started) {
            Some(Answer::Current(asked)) => asked,
            Some(Answer::Late(late)) => return self.keep_late_review(&pulls, task, located, &late, message).await,
            None => {
                tracing::debug!(task = %task, "a reviewer ended with nothing that it was asked to review: its report is not taken");
                return Ok(());
            }
        };
        match parse_verdict(message) {
            Ok(verdict) => {
                self.watch.bounces.remove(task);
                self.take_verdict(&pulls, task, located, &asked, verdict, message).await
            }
            Err(problem) => self.send_back(&pulls, task, located, &asked, &problem).await,
        }
    }

    /// The review that a run of the reviewer `task` that started at `started` answers: the latest review asked of it at or
    /// before that time that is still pending, in the round that is going or in one the pull request has left behind.
    fn answered_by(&self, team: &TeamId, task: &TaskId, started: Timestamp) -> Option<Answer> {
        let team = self.parts.state.team(team)?;
        let mut offered: Vec<Offered> = Vec::new();
        for progress in team.items() {
            let pending = |reviews: &[Review]| {
                reviews
                    .iter()
                    .find(|review| review.task == *task && matches!(review.outcome, ReviewOutcome::Pending))
                    .map(|review| (review.requested_at, review.reviewer.clone()))
            };
            if let ItemPhase::InReview { pr, reviews } = progress.phase()
                && let Some((requested_at, reviewer)) = pending(reviews)
            {
                offered.push(Offered {
                    requested_at,
                    item: progress.id(),
                    pr: pr.clone(),
                    reviewer,
                    round: progress.fix_passes().len(),
                    current: true,
                });
            }
            let pr = match progress.phase() {
                ItemPhase::InReview { pr, .. } | ItemPhase::Fixing { pr, .. } | ItemPhase::Approved { pr, .. } => pr,
                ItemPhase::Done {
                    completion: Completion::Merged { pr, .. },
                } => pr,
                ItemPhase::Done {
                    completion: Completion::Listed,
                }
                | ItemPhase::Pending
                | ItemPhase::InProgress { .. } => continue,
            };
            for (round, pass) in progress.fix_passes().iter().enumerate() {
                if let Some((requested_at, reviewer)) = pending(&pass.requested_by) {
                    offered.push(Offered {
                        requested_at,
                        item: progress.id(),
                        pr: pr.clone(),
                        reviewer,
                        round,
                        current: false,
                    });
                }
            }
        }
        let latest = offered
            .iter()
            .map(|found| found.requested_at)
            .filter(|requested| *requested <= started)
            .max()?;
        let answered: Vec<Offered> = offered
            .into_iter()
            .filter(|found| found.requested_at == latest)
            .collect();
        let current: Vec<&Offered> = answered.iter().filter(|found| found.current).collect();
        if let Some(first) = current.first() {
            return Some(Answer::Current(Asked {
                pr: first.pr.clone(),
                items: current
                    .iter()
                    .filter(|found| found.pr.number == first.pr.number)
                    .map(|found| found.item)
                    .collect(),
                requested_at: latest,
                passes: first.round,
                reviewer: first.reviewer.clone(),
            }));
        }
        answered.into_iter().next().map(|found| {
            Answer::Late(Late {
                pr: found.pr,
                round: found.round,
                reviewer: found.reviewer,
            })
        })
    }

    /// A report that answers a review of a round the pull request has left behind, because the reviewer was slow and the
    /// other reviewer had asked for changes: the round is decided, and the report is kept and posted all the same.
    async fn keep_late_review(
        &mut self,
        pulls: &Pulls,
        task: &TaskId,
        located: &Located,
        late: &Late,
        message: &str,
    ) -> Result<(), OwnerError> {
        tracing::info!(task = %task, pr = %late.pr.number, "a reviewer ended a review of a round that is decided: its report is kept and posted");
        let report = report_text(message);
        let round = u32::try_from(late.round).unwrap_or(u32::MAX);
        self.parts
            .store
            .save_review_report(task, late.pr.number, round, &report)
            .await?;
        let lead = self.lead_of(&located.team)?;
        let asked = Asked {
            pr: late.pr.clone(),
            items: Vec::new(),
            requested_at: Timestamp::now(&self.parts.clock)?,
            passes: late.round,
            reviewer: late.reviewer.clone(),
        };
        self.post_review(pulls, &lead, task, &asked, &report).await;
        Ok(())
    }

    /// A report without a verdict: the reviewer is asked for it again, and after [`MAX_VERDICT_BOUNCES`] times the human is
    /// told.
    async fn send_back(
        &mut self,
        pulls: &Pulls,
        task: &TaskId,
        located: &Located,
        asked: &Asked,
        problem: &VerdictProblem,
    ) -> Result<(), OwnerError> {
        let count = self.watch.bounces.entry(task.clone()).or_insert(0);
        *count = count.saturating_add(1);
        let count = *count;
        tracing::warn!(task = %task, pr = %asked.pr.number, ?problem, count, "a reviewer's report has no verdict");
        if count <= MAX_VERDICT_BOUNCES {
            let mut round = Round::default();
            self.release(task, Why::VerdictMissing, &mut round)?;
            return self.finish_round(&located.team, round).await;
        }
        let announcement = Announcement::NoVerdict { task: task.clone() };
        if !self.watch.announced.insert(announcement) {
            return Ok(());
        }
        let now = Timestamp::now(&self.parts.clock)?;
        let lead = self.lead_of(&located.team)?;
        let values = Variables::new()
            .with("reviewer", asked.reviewer.as_str())
            .and_then(|values| values.with("pr", asked.pr.number.to_string()))
            .and_then(|values| values.with("task", task.as_str()));
        self.tell_human(pulls, AutopilotPrompt::NoticeNoVerdict, values, &lead, now);
        Ok(())
    }

    /// Records the verdict `verdict` of the reviewer `task`, keeps and posts its report, and does what the round does next.
    async fn take_verdict(
        &mut self,
        pulls: &Pulls,
        task: &TaskId,
        located: &Located,
        asked: &Asked,
        verdict: ReviewVerdict,
        message: &str,
    ) -> Result<(), OwnerError> {
        let team_id = &located.team;
        let now = Timestamp::now(&self.parts.clock)?;
        let report = report_text(message);
        let passes = u32::try_from(asked.passes).unwrap_or(u32::MAX);
        self.parts
            .store
            .save_review_report(task, asked.pr.number, passes, &report)
            .await?;
        let lead = self.lead_of(team_id)?;
        let next = self.decide(team_id, asked, verdict);
        let mut round = Round::default();
        self.record_verdict(team_id, task, asked, verdict, now, &mut round)?;
        self.post_review(pulls, &lead, task, asked, &report).await;
        if let Some(next) = next {
            self.apply_next(team_id, &lead, asked, next, &mut round)?;
        }
        round.changed = true;
        self.finish_round(team_id, round).await
    }

    /// What the verdict leads to, by the round machine; `None` when the round does not take it, which is told: the
    /// verdict is recorded all the same.
    fn decide(&self, team_id: &TeamId, asked: &Asked, verdict: ReviewVerdict) -> Option<Next> {
        let team = self.parts.state.team(team_id)?;
        let progress = asked.items.first().and_then(|id| team.item(*id))?;
        let policy = RoundPolicy::new(team.merge(), self.parts.settings.config.review());
        match after_verdict(progress, &asked.reviewer, verdict, policy) {
            Ok(next) => Some(next),
            Err(VerdictError::Round(RoundError::NotExpected { .. })) => {
                tracing::info!(team = %team_id, pr = %asked.pr.number, "a verdict came after the round was decided: it is recorded and changes nothing");
                None
            }
            Err(error) => {
                tracing::warn!(team = %team_id, pr = %asked.pr.number, error = %error_chain(&error), "the round does not take a verdict: it is recorded and changes nothing");
                None
            }
        }
    }

    /// Records the outcome of the review of `task` in every item of the pull request.
    fn record_verdict(
        &mut self,
        team_id: &TeamId,
        task: &TaskId,
        asked: &Asked,
        verdict: ReviewVerdict,
        now: Timestamp,
        round: &mut Round,
    ) -> Result<(), OwnerError> {
        for id in &asked.items {
            let mut team_mut = self
                .parts
                .state
                .team_mut(team_id)
                .ok_or_else(|| OwnerError::LostTeam { team: team_id.clone() })?;
            let mut item = team_mut.item_mut(*id).ok_or_else(|| OwnerError::LostItem {
                team: team_id.clone(),
                item: *id,
            })?;
            item.submit_review(task, verdict, now)
                .map_err(|source| OwnerError::Review {
                    team: team_id.clone(),
                    item: *id,
                    source,
                })?;
            round.events.push(EventKind::ReviewSubmitted {
                team: team_id.clone(),
                item: *id,
                reviewer: asked.reviewer.clone(),
                verdict,
            });
        }
        Ok(())
    }

    /// Posts the report of the reviewer `task` on the pull request as a comment review that names the commit it reviewed.
    /// The harness opens the pull request and posts the reviews with one account, which GitHub does not let approve its
    /// own pull request: the verdict is in the report.
    ///
    /// A review that cannot be posted is logged and does not hold the round: the verdict counts, and the report is in the
    /// task's directory.
    async fn post_review(&mut self, pulls: &Pulls, lead: &Lead, task: &TaskId, asked: &Asked, report: &str) {
        if let Err(error) = self.try_post_review(pulls, lead, task, asked, report).await {
            tracing::warn!(task = %task, pr = %asked.pr.number, error = %error_chain(&error), "a review cannot be posted on the pull request: its verdict counts and its report is kept");
        }
    }

    /// See [`Owner::post_review`].
    async fn try_post_review(
        &mut self,
        pulls: &Pulls,
        lead: &Lead,
        task: &TaskId,
        asked: &Asked,
        report: &str,
    ) -> Result<(), PublishError> {
        let agent: AgentName = asked.reviewer.as_str().parse()?;
        let git = self.worktree_git(&lead.repo, task, &agent)?;
        let head = git.rev_parse(&Revision::Head).await?;
        let github = self.github_for(pulls, &lead.repo, &git).await?;
        let body = pulls.texts.render(
            AutopilotPrompt::ReviewComment,
            &Variables::new()
                .with("reviewer", asked.reviewer.as_str())?
                .with("head", head.as_str())?
                .with("report", report)?,
        )?;
        github
            .post_comment_review(
                asked.pr.number,
                &CommentReview {
                    body: body.trim_end().to_owned(),
                    commit: head,
                },
            )
            .await?;
        Ok(())
    }

    /// Does what the verdict led to, in every item of the pull request.
    fn apply_next(
        &mut self,
        team_id: &TeamId,
        lead: &Lead,
        asked: &Asked,
        next: Next,
        round: &mut Round,
    ) -> Result<(), OwnerError> {
        for id in &asked.items {
            let mut team_mut = self
                .parts
                .state
                .team_mut(team_id)
                .ok_or_else(|| OwnerError::LostTeam { team: team_id.clone() })?;
            let mut item = team_mut.item_mut(*id).ok_or_else(|| OwnerError::LostItem {
                team: team_id.clone(),
                item: *id,
            })?;
            let ItemPhase::InReview { pr, reviews } = item.phase().clone() else {
                continue;
            };
            let after = match next {
                Next::Wait | Next::AskHuman => continue,
                Next::Merge | Next::HandToHuman => ItemPhase::Approved { pr, reviews },
                Next::FixPass { .. } => ItemPhase::Fixing {
                    assignee: lead.agent.clone(),
                    branch: pr.branch.clone(),
                    pr,
                },
            };
            let status = after.kind().status();
            item.advance(after).map_err(|source| OwnerError::Item {
                team: team_id.clone(),
                item: *id,
                source,
            })?;
            round.events.push(EventKind::ItemStatus {
                team: team_id.clone(),
                item: *id,
                status,
            });
        }
        Ok(())
    }

    /// Saves the state after what `round` changed, and tells the subscribers; the team is looked at again.
    async fn finish_round(&mut self, team: &TeamId, round: Round) -> Result<(), OwnerError> {
        if round.changed {
            self.save().await?;
            let now = Timestamp::now(&self.parts.clock)?;
            for event in round.events {
                self.publish(now, event);
            }
            self.publish_summary(now, team);
        }
        self.look_at(team);
        Ok(())
    }

    // ---- the merge ------------------------------------------------------------------------------------------

    /// Merges the pull requests of `team_id` that both reviewers approved, and completes the items of those that are
    /// merged.
    async fn merge_approved(&mut self, pulls: &Pulls, team_id: &TeamId, round: &mut Round) -> Result<(), OwnerError> {
        let approved = self.approved_pull_requests(team_id);
        if approved.is_empty() {
            return Ok(());
        }
        let lead = self.lead_of(team_id)?;
        for (pr, items) in approved {
            let now = Timestamp::now(&self.parts.clock)?;
            if !self.due(team_id, &pr, now)? {
                continue;
            }
            match self.try_merge(pulls, team_id, &lead, &pr, now).await {
                Ok(Merge::Merged) => {
                    self.watch.polled.remove(&(team_id.clone(), pr.number));
                    self.complete(team_id, &lead, &pr, &items, now, round).await?;
                }
                Ok(Merge::Waits) => {}
                Err(error) => {
                    tracing::warn!(team = %team_id, pr = %pr.number, error = %error_chain(&error), "an approved pull request cannot be merged: it is looked at again in a while");
                }
            }
        }
        Ok(())
    }

    /// The approved pull requests of `team`, with their items.
    fn approved_pull_requests(&self, team: &TeamId) -> Vec<(PullRequest, Vec<ItemId>)> {
        let mut found: Vec<(PullRequest, Vec<ItemId>)> = Vec::new();
        let Some(team) = self.parts.state.team(team) else {
            return found;
        };
        for progress in team.items() {
            let ItemPhase::Approved { pr, .. } = progress.phase() else {
                continue;
            };
            match found.iter_mut().find(|(known, _)| known.number == pr.number) {
                Some((_, items)) => items.push(progress.id()),
                None => found.push((pr.clone(), vec![progress.id()])),
            }
        }
        found
    }

    /// Whether the approved pull request `pr` is looked at now: the first time at once, then once in [`POLL_EVERY`].
    fn due(&mut self, team: &TeamId, pr: &PullRequest, now: Timestamp) -> Result<bool, OwnerError> {
        let key = (team.clone(), pr.number);
        if self
            .watch
            .polled
            .get(&key)
            .is_some_and(|last| Timestamp::from_utc(last.as_utc() + POLL_EVERY).is_ok_and(|next| next > now))
        {
            return Ok(false);
        }
        self.watch.polled.insert(key, now);
        Ok(true)
    }

    /// Merges `pr` when it is open and the team merges by itself; tells the human what the team does not do by itself:
    /// a pull request that is approved and left to them, one that GitHub refuses to merge, one that was closed.
    async fn try_merge(
        &mut self,
        pulls: &Pulls,
        team_id: &TeamId,
        lead: &Lead,
        pr: &PullRequest,
        now: Timestamp,
    ) -> Result<Merge, PublishError> {
        let git = self.worktree_git(&lead.repo, &lead.task, &lead.agent)?;
        let github = self.github_for(pulls, &lead.repo, &git).await?;
        let remote = github.get_pull_request(pr.number).await?;
        let about = |extra: Extra<'_>| -> Result<Variables, VariablesError> {
            let values = Variables::new()
                .with("team", team_id.as_str())?
                .with("pr", pr.number.to_string())?
                .with("url", &remote.url)?;
            match extra {
                Extra::Nothing => Ok(values),
                Extra::Repo => values.with("repo", lead.repo.as_str()),
                Extra::Reason(reason) => values.with("reason", reason),
            }
        };
        let key = (team_id.clone(), pr.number);
        match remote.state {
            PullState::Merged => Ok(Merge::Merged),
            PullState::Closed => {
                let announcement = Announcement::Closed {
                    team: team_id.clone(),
                    pr: pr.number,
                };
                self.announce(
                    pulls,
                    lead,
                    announcement,
                    AutopilotPrompt::NoticeClosed,
                    about(Extra::Nothing),
                    now,
                );
                Ok(Merge::Waits)
            }
            PullState::Open => {
                let autopilot = self
                    .parts
                    .state
                    .team(team_id)
                    .is_some_and(|team| team.merge() == agent_core::state::MergeMode::Autopilot);
                if !autopilot {
                    let announcement = Announcement::Approved {
                        team: team_id.clone(),
                        pr: pr.number,
                    };
                    self.announce(
                        pulls,
                        lead,
                        announcement,
                        AutopilotPrompt::NoticeApproved,
                        about(Extra::Repo),
                        now,
                    );
                    return Ok(Merge::Waits);
                }
                if self.watch.refused.contains(&key) {
                    return Ok(Merge::Waits);
                }
                let request = MergeRequest {
                    method: MergeMethod::Merge,
                    expected_head: remote.head.clone(),
                    commit_title: None,
                    commit_message: None,
                };
                match github.merge_pull_request(pr.number, &request).await {
                    Ok(_) => Ok(Merge::Merged),
                    Err(error) if refuses(&error) => {
                        self.watch.refused.insert(key);
                        let announcement = Announcement::MergeFailed {
                            team: team_id.clone(),
                            pr: pr.number,
                        };
                        let values = about(Extra::Reason(&error_chain(&error)));
                        self.announce(
                            pulls,
                            lead,
                            announcement,
                            AutopilotPrompt::NoticeMergeFailed,
                            values,
                            now,
                        );
                        Ok(Merge::Waits)
                    }
                    Err(error) => Err(error.into()),
                }
            }
        }
    }

    /// Tells the human `prompt` rendered with `values`, unless `announcement` was told before.
    fn announce(
        &mut self,
        pulls: &Pulls,
        lead: &Lead,
        announcement: Announcement,
        prompt: AutopilotPrompt,
        values: Result<Variables, VariablesError>,
        now: Timestamp,
    ) {
        if self.watch.announced.insert(announcement) {
            self.tell_human(pulls, prompt, values, lead, now);
        }
    }

    /// Completes the items of the merged pull request `pr`; the checkout learns what the merge made of the base branch, so
    /// that the next item builds on it.
    async fn complete(
        &mut self,
        team_id: &TeamId,
        lead: &Lead,
        pr: &PullRequest,
        items: &[ItemId],
        now: Timestamp,
        round: &mut Round,
    ) -> Result<(), OwnerError> {
        for id in items {
            let mut team_mut = self
                .parts
                .state
                .team_mut(team_id)
                .ok_or_else(|| OwnerError::LostTeam { team: team_id.clone() })?;
            let mut item = team_mut.item_mut(*id).ok_or_else(|| OwnerError::LostItem {
                team: team_id.clone(),
                item: *id,
            })?;
            item.advance(ItemPhase::Done {
                completion: Completion::Merged {
                    pr: pr.clone(),
                    at: now,
                },
            })
            .map_err(|source| OwnerError::Item {
                team: team_id.clone(),
                item: *id,
                source,
            })?;
            round.events.push(EventKind::ItemStatus {
                team: team_id.clone(),
                item: *id,
                status: agent_core::status::ItemStatus::Done,
            });
            round.changed = true;
            tracing::info!(team = %team_id, item = %id, pr = %pr.number, "an item is merged");
        }
        self.fetch_base(lead).await;
        Ok(())
    }

    /// Fetches the base branch of the repository of `lead` into the checkout. A fetch that fails leaves the next item to
    /// start from the base the checkout had; that is logged.
    async fn fetch_base(&self, lead: &Lead) {
        let result = async {
            let git = self.worktree_git(&lead.repo, &lead.task, &lead.agent)?;
            let base = self.base_branch(&lead.repo)?;
            git.fetch(&base).await?;
            Ok::<(), PublishError>(())
        }
        .await;
        if let Err(error) = result {
            tracing::warn!(repo = %lead.repo, error = %error_chain(&error), "the base branch cannot be fetched after a merge: the next item starts from the checkout's");
        }
    }
}

/// A review that was asked of a reviewer, as the state keeps it, and where it was found.
struct Offered {
    /// When the review was asked.
    requested_at: Timestamp,
    /// The item it was found in.
    item: ItemId,
    /// The pull request.
    pr: PullRequest,
    /// The investigator that reviews.
    reviewer: agent_core::ids::InvestigatorName,
    /// The fix passes the pull request had had when the review was asked.
    round: usize,
    /// Whether it is the round that is going.
    current: bool,
}

/// What a reviewer's report answers.
enum Answer {
    /// A review of the round that is going.
    Current(Asked),
    /// A review of a round the pull request has left behind.
    Late(Late),
}

/// A review of a round that is decided.
struct Late {
    /// The pull request.
    pr: PullRequest,
    /// The fix passes the pull request had had when the review was asked.
    round: usize,
    /// The investigator that reviews.
    reviewer: agent_core::ids::InvestigatorName,
}

/// What a notification about a pull request has besides the team, the number and the address.
enum Extra<'a> {
    /// Nothing.
    Nothing,
    /// The repository.
    Repo,
    /// Why GitHub refused.
    Reason(&'a str),
}

/// What was found of an approved pull request.
enum Merge {
    /// It is merged, by the harness or by someone else.
    Merged,
    /// It is not merged and is left as it is for now.
    Waits,
}

/// The review a reviewer's final message answers: the pull request, its items and when it was asked.
struct Asked {
    /// The pull request.
    pr: PullRequest,
    /// The items of the pull request that have the review pending, in the order of the list.
    items: Vec<ItemId>,
    /// When the review was asked.
    requested_at: Timestamp,
    /// The fix passes the pull request has had.
    passes: usize,
    /// The investigator that reviews.
    reviewer: agent_core::ids::InvestigatorName,
}

/// Whether GitHub refused the merge for a reason that waiting does not change: the pull request cannot be merged as it is,
/// its head moved, the values are refused, or the token may not do it. A busy or failing GitHub is not a refusal.
fn refuses(error: &GitHubError) -> bool {
    matches!(
        error,
        GitHubError::Status {
            reason: Rejection::NotAllowed
                | Rejection::Conflict
                | Rejection::Invalid
                | Rejection::Forbidden
                | Rejection::Unauthorized
                | Rejection::NotFound,
            ..
        }
    )
}

/// A review of the task `task` that waits for its outcome, asked at `now`.
fn pending_review(reviewer: &agent_core::ids::InvestigatorName, task: &TaskId, now: Timestamp) -> Review {
    Review {
        reviewer: reviewer.clone(),
        task: task.clone(),
        requested_at: now,
        outcome: ReviewOutcome::Pending,
    }
}

/// The items of `items` whose branch is part of the history of `tip`.
async fn contained(git: &Git, items: &[Joining], tip: &CommitId) -> Result<Vec<ItemId>, PublishError> {
    let mut found = Vec::new();
    for joining in items {
        let part = git
            .is_ancestor(
                &Revision::Branch(joining.branch.clone()),
                &Revision::Commit(tip.clone()),
            )
            .await?;
        if part {
            found.push(joining.item);
        }
    }
    Ok(found)
}

/// The notification of the text `prompt` rendered with `values`: its first line is the title, the rest the body.
fn notification(
    texts: &AutopilotPrompts,
    prompt: AutopilotPrompt,
    values: &Variables,
) -> Result<Notification, PublishError> {
    let rendered = texts.render(prompt, values)?;
    let (title, body) = rendered.split_once('\n').unwrap_or((rendered.as_str(), ""));
    Ok(Notification::new(title.trim_end(), body.trim())?)
}

#[cfg(test)]
mod fake_github;
#[cfg(test)]
mod rig;
#[cfg(test)]
mod tests;
