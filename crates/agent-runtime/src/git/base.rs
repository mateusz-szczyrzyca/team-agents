//! Fetch and comparison before a team creates any durable resources.

use super::{Git, GitError, GitOperation, Reach, arg, text, whole};
use agent_core::ids::BranchName;
use agent_core::state::UnpushedBase;
use std::num::NonZeroU64;

impl Git {
    /// Fetches origin's base and reports local commits missing from it; no origin leaves the checkout unchanged.
    ///
    /// # Errors
    /// [`GitError`] if remote discovery, fetching or reading the local history fails.
    pub async fn unpushed_base(&self, base: &BranchName) -> Result<Option<UnpushedBase>, GitError> {
        let remotes = self.base_output(GitOperation::OriginUrl, &["remote"]).await?;
        if !remotes.lines().any(|remote| remote == "origin") {
            return Ok(None);
        }
        self.fetch(base).await?;
        let range = format!("refs/remotes/origin/{base}..refs/heads/{base}");
        let count_text = self
            .base_output(GitOperation::Log, &["rev-list", "--count", &range, "--"])
            .await?;
        let count = count_text
            .trim()
            .parse::<u64>()
            .map_err(|source| GitError::NotANumber {
                operation: GitOperation::Log,
                text: count_text.clone(),
                source,
            })?;
        let Some(count) = NonZeroU64::new(count) else {
            return Ok(None);
        };
        let log = self
            .base_output(
                GitOperation::Log,
                &["log", "--reverse", "--topo-order", "--format=%H%x00%s", &range, "--"],
            )
            .await?;
        let (first, subject) = log
            .lines()
            .next()
            .and_then(|line| line.split_once('\0'))
            .ok_or_else(|| GitError::Unexpected {
                operation: GitOperation::Log,
                output: log.clone(),
            })?;
        let first = first.parse().map_err(|source| GitError::NotACommit {
            operation: GitOperation::Log,
            source,
        })?;
        Ok(Some(UnpushedBase {
            branch: base.clone(),
            count,
            first,
            subject: subject.to_owned(),
        }))
    }

    /// Reads bounded UTF-8 output of a local comparison command.
    async fn base_output(&self, operation: GitOperation, args: &[&str]) -> Result<String, GitError> {
        let args = args.iter().copied().map(arg).collect::<Vec<_>>();
        let captured = self.run_ok(operation, Reach::Local, &args).await?;
        whole(operation, &captured.stdout, self.limits)?;
        text(operation, &captured.stdout)
    }
}
