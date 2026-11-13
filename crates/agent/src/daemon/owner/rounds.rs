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
use agent_core::decide::round::{RoundEvent, RoundPolicy, RoundState, Stage};
use agent_core::ids::{AgentName, BranchName, GenerateIdError, InvalidBranchName, InvalidName, ItemId, TaskId, TeamId};
use agent_core::kinds::Role;
use agent_core::prompt::{AutopilotPrompt, Variables, VariablesError};
use agent_core::protocol::EventKind;
use agent_core::state::{
    CommitId, GateRecord, ItemPhase, Member, Participant, PullRequest, Review, ReviewOutcome, TaskRef, Timestamp,
};
use agent_runtime::autopilot_prompts::{AutopilotPrompts, RenderAutopilotTextError};
use agent_runtime::git::{CommitIdentity, Git, GitError, Revision, WorktreeBranch};
use agent_runtime::github::{
    ApiBase, GitHub, GitHubError, GitHubSettings, InvalidApiBase, InvalidRepoSlug, NewPullRequest, PullRequestUpdate,
    Readiness, RepoSlug,
};
use agent_runtime::notify::{InvalidNotification, Notification};
use agent_runtime::process::error_chain;
use agent_runtime::run::compose::{ComposeError, PromptTexts, item_sections};
use agent_runtime::settings::Settings;
use agent_runtime::store::StoreError;

use super::start::Context;
use super::teams::Round;
use super::{Notice, Owner, OwnerError};
use crate::daemon::pr::{Joining, Publication, Publish, PullTextError, plan_publications, pull_text};
use crate::daemon::review::{Slot, choose_reviewers, free_reviewers};
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
}

/// What the autopilot adds to the prompt of the run of the task `found` that works in `working`: for a reviewer, the pull
/// request it is asked for a review of and the items it is for; `None` when the task has nothing of the autopilot to be
/// told.
///
/// # Errors
/// [`SectionError`].
pub(super) async fn sections_for(
    ctx: &Context<'_>,
    texts: &AutopilotPrompts,
    found: &TaskRef<'_>,
    working: &std::path::Path,
) -> Result<Option<String>, SectionError> {
    match found.role {
        Role::Reviewer => review_request(ctx, texts, found, working).await,
        Role::Solo | Role::Coordinator | Role::Member | Role::Worker => Ok(None),
    }
}

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
}

/// Something the human is told once.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(super) enum Announcement {
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
        self.publish_branches(&pulls, team, round).await?;
        self.ask_for_reviews(team, round).await?;
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
