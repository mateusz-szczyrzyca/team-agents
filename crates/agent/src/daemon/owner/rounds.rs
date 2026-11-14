//! The owner's side of the autopilot: publishing the branch of a team as a pull request, asking investigators from
//! outside the team to review it, taking their verdicts, giving the team its fix passes, asking the human when they are
//! used up, and merging (or, with `--no-merge`, telling the human).
//!
//! What is decided is in `daemon/pr`, `daemon/review` and `daemon/merge`; the state of each round is the state of the
//! items, so a daemon that restarts finds the same things to do at its first look. Without the token of a GitHub account
//! in `secrets.toml` the autopilot is off: pull requests are not opened and the items stay where they are.

mod checks;
mod human;
mod merges;
mod prompts;
mod publish;
mod reviewers;
mod verdicts;

use std::collections::{HashMap, HashSet};
use std::str::FromStr;
use std::sync::Arc;

use agent_core::config::{AbsolutePath, GithubLogin, InvalidValue, RepoName, Secret};
use agent_core::ids::{AgentName, BranchName, GenerateIdError, InvalidBranchName, InvalidName, TaskId, TeamId};
use agent_core::prompt::{AutopilotPrompt, Variables, VariablesError};
use agent_core::state::{PrNumber, Timestamp};
use agent_runtime::autopilot_prompts::{AutopilotPrompts, RenderAutopilotTextError};
use agent_runtime::checks_prompts::ChecksPrompts;
use agent_runtime::git::{CommitIdentity, Git, GitError};
use agent_runtime::github::{ApiBase, GitHub, GitHubError, GitHubSettings, InvalidApiBase, InvalidRepoSlug, RepoSlug};
use agent_runtime::notify::{InvalidNotification, Notification};
use agent_runtime::process::error_chain;
use agent_runtime::settings::Settings;
use agent_runtime::store::StoreError;
use agent_runtime::text_file::ReplaceError;

use super::{Notice, Owner, OwnerError};

use super::teams::Look;
use crate::daemon::checks::ChecksTextError;
use crate::daemon::pr::PullTextError;
use crate::daemon::questions::QuestionsProblem;
use crate::daemon::team::agent_name;

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
    /// The texts of the checks before a review.
    checks: Arc<ChecksPrompts>,
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
    pub(crate) fn new(
        texts: Arc<AutopilotPrompts>,
        checks: Arc<ChecksPrompts>,
        settings: &Settings,
    ) -> Result<Option<Self>, PullsError> {
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
            checks,
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

    /// The texts of the checks before a review.
    pub(super) fn checks(&self) -> &ChecksPrompts {
        &self.checks
    }

    /// A client of the repository `slug`.
    ///
    /// # Errors
    /// [`GitHubError::Client`] when the HTTP client cannot be built.
    pub(super) fn client(&self, slug: RepoSlug) -> Result<GitHub, GitHubError> {
        GitHub::new(self.settings.clone(), slug, &self.token, &self.trusted)
    }
}

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
    /// The checks before a review need a question put to the human, and the lead did not put it.
    NotAsked {
        /// The team.
        team: TeamId,
        /// The lead's task.
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
    /// A text of the checks before a review cannot be made.
    #[error("cannot make a text of the checks before a review")]
    Checks(#[from] ChecksTextError),
    /// The questions of the checks cannot be read back.
    #[error("cannot read the questions of the checks")]
    Questions(#[from] QuestionsProblem),
    /// The questions of the checks cannot be written.
    #[error("cannot write the questions of the checks")]
    Write(#[from] ReplaceError),
    /// The lead cannot ask any more questions.
    #[error("the lead cannot ask more questions: the numbers are used up")]
    QuestionNumbers,
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

/// The facts about the lead of a team that a look at its pull requests needs.
struct Lead {
    /// The team's repository.
    repo: RepoName,
    /// The lead's task.
    task: TaskId,
    /// The lead's agent.
    agent: AgentName,
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

impl Owner {
    /// One look at the pull requests of `team`.
    ///
    /// # Errors
    /// [`OwnerError`] when the state cannot be saved or is found inconsistent.
    pub(super) async fn pull_requests(&mut self, team: &TeamId, look: &mut Look) -> Result<(), OwnerError> {
        let Some(pulls) = self.pulls.clone() else {
            return Ok(());
        };
        self.merge_approved(&pulls, team, look).await?;
        self.publish_branches(&pulls, team, look).await?;
        self.ask_for_reviews(team, look).await?;
        self.deliver_reviews(team, look)?;
        self.deliver_fix_passes(team, look)?;
        self.ask_the_human(&pulls, team, look)?;
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
                tracing::error!(task = %lead.task, %error, "a notification of the autopilot has no task to be about");
            }
        }
    }

    /// Saves the state after what `look` changed, and tells the subscribers; the team is looked at again.
    async fn finish_look(&mut self, team: &TeamId, look: Look) -> Result<(), OwnerError> {
        if look.changed {
            self.save().await?;
            let now = Timestamp::now(&self.parts.clock)?;
            for event in look.events {
                self.publish(now, event);
            }
            self.publish_summary(now, team);
        }
        self.look_at(team);
        Ok(())
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
}

pub(super) use self::prompts::{Addition, SectionError, sections_for};

#[cfg(test)]
#[path = "rounds/tests/checks.rs"]
mod checks_tests;
#[cfg(test)]
#[path = "rounds/tests/fake_github.rs"]
mod fake_github;
#[cfg(test)]
#[path = "rounds/tests/history_records.rs"]
mod history_records_tests;
#[cfg(test)]
#[path = "rounds/tests/history_sessions.rs"]
mod history_sessions_tests;
#[cfg(test)]
#[path = "rounds/tests/rig.rs"]
mod rig;
#[cfg(test)]
#[path = "rounds/tests/round_records.rs"]
mod round_records_tests;
#[cfg(test)]
mod tests;
