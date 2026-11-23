//! Commit times used to reconstruct activity before the daemon observed a branch.
use super::{Git, GitError, GitOperation, Reach, Revision, arg, text};
use agent_core::state::{InvalidTimestamp, Timestamp};

/// Why a commit's time could not be read.
#[derive(Debug, thiserror::Error)]
pub enum CommitTimeError {
    /// Git could not read the commit.
    #[error("cannot read commit time")]
    Git(#[from] GitError),
    /// The committer timestamp is outside the supported format or range.
    #[error("invalid committer timestamp")]
    Invalid(#[from] InvalidTimestamp),
}

impl Git {
    /// Reads the committer time, including for a root commit or detached HEAD.
    ///
    /// # Errors
    /// [`CommitTimeError`] if Git fails or prints an invalid timestamp.
    pub async fn commit_time(&self, revision: &Revision) -> Result<Timestamp, CommitTimeError> {
        let operation = GitOperation::Log;
        let revision = revision.to_string();
        let output = self
            .run_ok(
                operation,
                Reach::Local,
                &[arg("log"), arg("-1"), arg("--format=%cI"), arg(&revision), arg("--")],
            )
            .await?;
        Ok(text(operation, &output.stdout)?.trim().parse()?)
    }
}

#[cfg(test)]
mod tests;
