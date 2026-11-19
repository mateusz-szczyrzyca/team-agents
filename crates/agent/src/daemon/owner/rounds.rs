//! The owner's side of the autopilot: publishing the branch of a team as a pull request, asking investigators from
//! outside the team to review it, taking their verdicts, giving the team its fix passes, asking the human when they are
//! used up, and merging (or, with `--no-merge`, telling the human).
//!
//! What is decided is in `daemon/pr`, `daemon/review` and `daemon/merge`; the state of each round is the state of the
//! items, so a daemon that restarts finds the same things to do at its first look. Without the token of a GitHub account
//! in `secrets.toml` the autopilot is off: pull requests are not opened and the items stay where they are.

mod asleep;
mod checks;
mod cleanup;
mod findings;
mod human;
mod merges;
mod opinions;
mod prompts;
mod publish;
mod replace;
mod responses;
pub(crate) use opinions::OpinionDaemonError;
pub(crate) use responses::ResponseTextError;
mod reviewers;
mod verdicts;

use std::collections::{HashMap, HashSet};
use std::str::FromStr;
use std::sync::Arc;

use agent_core::config::{AbsolutePath, GithubLogin, InvalidValue, RepoName, Secret};
use agent_core::ids::{AgentName, BranchName, GenerateIdError, InvalidBranchName, InvalidName, TaskId, TeamId};
use agent_core::prompt::{AutopilotPrompt, Variables, VariablesError};
use agent_core::state::{CommitId, ItemPhase, PrNumber, State, Timestamp};
use agent_runtime::autopilot_prompts::{AutopilotPrompts, RenderAutopilotTextError};
use agent_runtime::checks_prompts::ChecksPrompts;
use agent_runtime::git::{CommitIdentity, Git, GitError};
use agent_runtime::github::{ApiBase, GitHub, GitHubError, GitHubSettings, InvalidApiBase, InvalidRepoSlug, RepoSlug};
use agent_runtime::notify::{InvalidNotification, Notification};
use agent_runtime::process::error_chain;
use agent_runtime::settings::Settings;
use agent_runtime::store::StoreError;
use agent_runtime::text_file::ReplaceError;

use super::{Owner, OwnerError};
use crate::daemon::notice::{NoticeContext, NoticeKind, layout_notification};

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
    /// The gates the harness runs or ran by itself for the leads that were done without one.
    own_gates: HashMap<TaskId, publish::OwnGate>,
    /// How many times in a row the lead of each team was woken because the whole team slept.
    asleep_wakes: HashMap<TeamId, u32>,
    /// How many times in a row each lead was sent back because the gate had not passed at the tip of its branch.
    gate_sent_back: HashMap<TaskId, u32>,
    /// The pull requests whose merge GitHub refused: they are not merged again, only looked at for the human's merge.
    refused: HashSet<(TeamId, PrNumber)>,
}

impl PullWatch {
    /// Clears blocked item announcements for any items in progress or fixing assigned to `task`.
    pub(super) fn reset_item_blocked_for_task(&mut self, state: &State, team_id: &TeamId, task: &TaskId) {
        let Some(team) = state.team(team_id) else {
            return;
        };
        let Some((_, member)) = team.crew().roster().into_iter().find(|(_, m)| m.task() == task) else {
            return;
        };
        let member_name = member.agent().name().to_string();
        for item in team.items() {
            if let ItemPhase::InProgress { assignee, .. } | ItemPhase::Fixing { assignee, .. } = item.phase()
                && assignee.as_str() == member_name
            {
                self.announced.remove(&Announcement::ItemBlocked {
                    team: team_id.clone(),
                    item: item.id().to_string(),
                });
            }
        }
    }
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
    /// The pull request was merged.
    Merged {
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
    /// A team sleeps with work unfinished after its lead was woken for it as often as it is.
    TeamAsleep {
        /// The team.
        team: TeamId,
        /// When its last run ended: a team that worked since and sleeps again is told again.
        since: Timestamp,
    },
    /// A team's item is blocked behind a failed member while its lead is done.
    ItemBlocked {
        /// The team.
        team: TeamId,
        /// The item.
        item: String,
    },
    /// The gate still does not pass at this tip after the lead was sent back as often as it is.
    GateNotPassed {
        /// The team.
        team: TeamId,
        /// The branch.
        branch: BranchName,
        /// The tip the gate does not pass at.
        tip: agent_core::state::CommitId,
    },
}

/// Why a branch could not be published: what the human is told, and what is logged.
#[derive(Debug, thiserror::Error)]
enum PublishError {
    /// A findings text cannot be rendered.
    #[error("cannot render findings")]
    Findings(#[from] findings::TextError),
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
    /// A review cannot be given to another reviewer.
    #[error("a review cannot be given to another reviewer")]
    Replacing(#[from] agent_core::state::ReplaceReviewError),
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
    /// Whether the lead's last run ended with `STATUS: DONE`: it says its work is finished, not that it waits for its team.
    done: bool,
}

/// The autopilot notifications sent to the human.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(super) enum AutopilotNotice {
    /// Human decision needed.
    Decision,
    /// Reviewer completed without a verdict.
    NoVerdict,
    /// Branch publish failed.
    PublishFailed,
    /// Team asleep.
    TeamAsleep,
    /// Standing item behind a failed member while lead is done.
    StandingItem,
    /// Pull request closed without merge.
    Closed,
    /// Pull request approved.
    Approved,
    /// Pull request merge failed.
    MergeFailed,
    /// Pull request merged.
    Merged,
}

impl AutopilotNotice {
    /// The prompt template for this notice.
    #[must_use]
    pub(super) const fn prompt(self) -> AutopilotPrompt {
        match self {
            Self::Decision => AutopilotPrompt::NoticeDecision,
            Self::NoVerdict => AutopilotPrompt::NoticeNoVerdict,
            Self::PublishFailed => AutopilotPrompt::NoticePublishFailed,
            Self::TeamAsleep => AutopilotPrompt::NoticeTeamAsleep,
            Self::StandingItem => AutopilotPrompt::NoticeStandingItem,
            Self::Closed => AutopilotPrompt::NoticeClosed,
            Self::Approved => AutopilotPrompt::NoticeApproved,
            Self::MergeFailed => AutopilotPrompt::NoticeMergeFailed,
            Self::Merged => AutopilotPrompt::NoticeMerged,
        }
    }

    /// The notification kind for this notice.
    #[must_use]
    pub(super) const fn kind(self) -> NoticeKind {
        match self {
            Self::Decision => NoticeKind::Decision,
            Self::NoVerdict => NoticeKind::NoVerdict,
            Self::PublishFailed => NoticeKind::PublishFailed,
            Self::TeamAsleep | Self::StandingItem => NoticeKind::TeamAsleep,
            Self::Closed => NoticeKind::Closed,
            Self::Approved => NoticeKind::Approved,
            Self::MergeFailed => NoticeKind::MergeFailed,
            Self::Merged => NoticeKind::Merged,
        }
    }

    /// Renders this notice with `values` into a notification for `context`.
    ///
    /// # Errors
    /// [`PublishError`] when variable rendering fails or layout fails.
    fn render(
        self,
        texts: &AutopilotPrompts,
        values: &Variables,
        context: &NoticeContext<'_>,
    ) -> Result<Notification, PublishError> {
        let rendered = texts.render(self.prompt(), values)?;
        Ok(layout_notification(self.kind(), context, &rendered)?)
    }
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
        self.read_fix_responses(&pulls, team, look).await?;
        self.ask_opinions(&pulls, team, look).await?;
        self.continue_opinions(&pulls, team, look)?;
        self.publish_branches(&pulls, team, look).await?;
        self.ask_for_reviews(team, look).await?;
        self.replace_reviewers(&pulls, team, look).await?;
        self.deliver_reviews(team, look)?;
        self.deliver_fix_passes(team, look)?;
        self.ask_the_human(&pulls, team, look)?;
        self.wake_sleeping_team(&pulls, team, look).await?;
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
            done: matches!(
                lead.runs().last().map(agent_core::state::Run::state),
                Some(agent_core::state::RunState::Ended {
                    end: agent_core::state::RunEnd::Reported {
                        outcome: agent_core::status::RunOutcome::Done
                    },
                    ..
                })
            ),
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

    /// Rewrites commit IDs and file paths in `text` into markdown links to GitHub.
    ///
    /// If commit resolution, tree listing or origin URL lookup fails, the failure is logged with [`LogEvent::GitCallFailed`](agent_core::log::LogEvent::GitCallFailed) and `text`
    /// is returned unchanged.
    pub(super) async fn link_commits(&self, git: &Git, commit: &CommitId, text: &str) -> String {
        let origin_url = match git.origin_url().await {
            Ok(url) => url,
            Err(error) => {
                tracing::warn!(
                    event = agent_core::log::LogEvent::GitCallFailed.as_str(),
                    error = %error_chain(&error),
                    "cannot determine origin url for commit links: text is sent unchanged"
                );
                return text.to_owned();
            }
        };
        let slug = match RepoSlug::from_remote_url(&origin_url) {
            Ok(slug) => slug,
            Err(error) => {
                tracing::warn!(
                    event = agent_core::log::LogEvent::GitCallFailed.as_str(),
                    error = %error_chain(&error),
                    "cannot determine repository slug for commit links: text is sent unchanged"
                );
                return text.to_owned();
            }
        };

        let tree_blobs = match git.tree_blobs(commit).await {
            Ok(blobs) => blobs,
            Err(error) => {
                tracing::warn!(
                    event = agent_core::log::LogEvent::GitCallFailed.as_str(),
                    error = %error_chain(&error),
                    "cannot list tree blobs for file links: text is sent unchanged"
                );
                return text.to_owned();
            }
        };

        let mut candidates = HashSet::new();
        let scanned = agent_core::commits::link_references(
            text,
            "",
            commit,
            |candidate| {
                candidates.insert(candidate.to_owned());
                None
            },
            |path| tree_blobs.contains(path),
        );
        drop(scanned);

        let mut resolved = HashMap::new();
        for candidate in &candidates {
            match git.resolve_commit(candidate).await {
                Ok(Some(id)) => {
                    resolved.insert(candidate.as_str(), id);
                }
                Ok(None) => {}
                Err(error) => {
                    tracing::warn!(
                        event = agent_core::log::LogEvent::GitCallFailed.as_str(),
                        candidate = %candidate,
                        error = %error_chain(&error),
                        "cannot resolve candidate commit: text is sent unchanged"
                    );
                    return text.to_owned();
                }
            }
        }

        let repo_url = format!("https://github.com/{}/{}", slug.owner(), slug.name());
        agent_core::commits::link_references(
            text,
            &repo_url,
            commit,
            |candidate| resolved.get(candidate).map(ToString::to_string),
            |path| tree_blobs.contains(path),
        )
    }

    /// Tells the human `notice` rendered with `values`, in the name of the lead; what cannot be made is logged: the
    /// harness goes on.
    fn tell_human(
        &self,
        pulls: &Pulls,
        notice: AutopilotNotice,
        values: Result<Variables, VariablesError>,
        lead: &Lead,
        now: Timestamp,
    ) {
        let pr = values
            .as_ref()
            .ok()
            .and_then(|v| v.names().find(|n| n.as_str() == "url").and_then(|n| v.get(n)))
            .map(str::to_owned);
        let context = match self.notice_context(&lead.task, now, pr.as_deref()) {
            Ok(c) => c,
            Err(error) => {
                tracing::error!(
                    event = agent_core::log::LogEvent::NotificationFailed.as_str(),
                    task = %lead.task,
                    %error,
                    "a notification of the autopilot has no task context"
                );
                return;
            }
        };
        let made = match values {
            Ok(v) => notice.render(&pulls.texts, &v, &context),
            Err(e) => Err(PublishError::from(e)),
        };
        let notification = match made {
            Ok(notification) => notification,
            Err(error) => {
                let prompt = notice.prompt();
                tracing::error!(
                    event = agent_core::log::LogEvent::NotificationFailed.as_str(),
                    ?prompt,
                    error = %error_chain(&error),
                    "a notification of the autopilot cannot be made"
                );
                return;
            }
        };
        match self.locate(&lead.task) {
            Ok(located) => self.notify(now, &located, &lead.task, notification),
            Err(error) => {
                tracing::error!(
                    event = agent_core::log::LogEvent::NotificationFailed.as_str(),
                    task = %lead.task,
                    %error,
                    "a notification of the autopilot has no task to be about"
                );
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

    /// Tells the human `notice`, unless `announcement` was told before.
    fn announce(
        &mut self,
        pulls: &Pulls,
        lead: &Lead,
        announcement: Announcement,
        notice: AutopilotNotice,
        values: Result<Variables, VariablesError>,
        now: Timestamp,
    ) {
        if self.watch.announced.insert(announcement) {
            self.tell_human(pulls, notice, values, lead, now);
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
#[path = "rounds/tests/history_events.rs"]
mod history_events_tests;
#[cfg(test)]
#[path = "rounds/tests/history_records.rs"]
mod history_records_tests;
#[cfg(test)]
#[path = "rounds/tests/history_sessions.rs"]
mod history_sessions_tests;
#[cfg(test)]
#[path = "rounds/tests/history_stories.rs"]
mod history_stories_tests;
#[cfg(test)]
#[path = "rounds/tests/review_head.rs"]
mod review_head_tests;
#[cfg(test)]
#[path = "rounds/tests/rig.rs"]
mod rig;
#[cfg(test)]
#[path = "rounds/tests/round_records.rs"]
mod round_records_tests;
#[cfg(test)]
mod tests;

#[cfg(test)]
#[path = "rounds/tests/history_failures.rs"]
mod history_failures_tests;

#[cfg(test)]
#[path = "rounds/tests/history_redaction.rs"]
mod history_redaction_tests;

#[cfg(test)]
#[path = "rounds/tests/history_events_redaction.rs"]
mod history_events_redaction_tests;

#[cfg(test)]
#[path = "rounds/tests/history_controls.rs"]
mod history_controls_tests;

#[cfg(test)]
#[path = "rounds/tests/history_acceptance.rs"]
mod history_acceptance_tests;

#[cfg(test)]
#[path = "rounds/tests/history_deadline.rs"]
mod history_deadline_tests;

#[cfg(test)]
#[path = "rounds/tests/daemon_log.rs"]
mod daemon_log_tests;

#[cfg(test)]
#[path = "rounds/tests/daemon_log_causes.rs"]
mod daemon_log_cause_tests;

#[cfg(test)]
#[path = "rounds/tests/history_queries.rs"]
mod history_queries_tests;

#[cfg(test)]
#[path = "rounds/tests/turn_limits.rs"]
mod turn_limits_tests;

#[cfg(test)]
#[path = "rounds/tests/turn_limit_restart.rs"]
mod turn_limit_restart_tests;

#[cfg(test)]
#[path = "rounds/tests/turn_limit_round.rs"]
mod turn_limit_round_tests;

#[cfg(test)]
#[path = "rounds/tests/notice_mapping.rs"]
mod notice_mapping_tests;

#[cfg(test)]
#[path = "rounds/tests/cleanup.rs"]
mod cleanup_tests;

#[cfg(test)]
#[path = "rounds/tests/publication_findings.rs"]
mod publication_findings_tests;

#[cfg(test)]
#[path = "rounds/tests/joined_pass.rs"]
mod joined_pass_tests;

#[cfg(test)]
#[path = "rounds/tests/verification.rs"]
mod verification_tests;

/// Failure to retain a reviewer's commit for subsequent verification.
#[derive(Debug, thiserror::Error)]
#[error("cannot retain the reviewed commit of task {task}")]
pub(crate) struct ReviewedHeadError {
    /// The reviewer whose worktree carries the commit.
    task: TaskId,
    /// The failed worktree or git operation.
    #[source]
    source: PublishError,
}

#[cfg(test)]
#[path = "rounds/tests/opinions.rs"]
mod opinion_tests;
