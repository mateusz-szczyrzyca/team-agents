//! The owner's side of the autopilot: publishing the branch of a team as a pull request, asking investigators from
//! outside the team to review it, taking their verdicts, giving the team its fix passes, asking the human when they are
//! used up, and merging (or, with `--no-merge`, telling the human).
//!
//! What is decided is in `daemon/pr`, `daemon/review` and `daemon/merge`; the state of each round is the state of the
//! items, so a daemon that restarts finds the same things to do at its first look. Without the token of a GitHub account
//! in `secrets.toml` the autopilot is off: pull requests are not opened and the items stay where they are.

use std::sync::Arc;

use agent_core::config::{GithubLogin, Secret};
use agent_core::ids::TeamId;
use agent_runtime::autopilot_prompts::AutopilotPrompts;
use agent_runtime::github::{ApiBase, GitHub, GitHubError, GitHubSettings, InvalidApiBase, RepoSlug};
use agent_runtime::settings::Settings;

use super::Owner;
use super::OwnerError;
use super::teams::Round;

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

    /// A client of the repository `slug`.
    ///
    /// # Errors
    /// [`GitHubError::Client`] when the HTTP client cannot be built.
    pub(super) fn client(&self, slug: RepoSlug) -> Result<GitHub, GitHubError> {
        GitHub::new(self.settings.clone(), slug, &self.token, &self.trusted)
    }
}

impl Owner {
    /// One look at the pull requests of `team`.
    ///
    /// # Errors
    /// [`OwnerError`] when the state cannot be saved or is found inconsistent.
    pub(super) async fn pull_requests(&mut self, team: &TeamId, round: &mut Round) -> Result<(), OwnerError> {
        let _ = (team, round);
        Ok(())
    }
}

#[cfg(test)]
mod fake_github;
#[cfg(test)]
mod rig;
#[cfg(test)]
mod tests;
