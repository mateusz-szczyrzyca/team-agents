//! A snapshot of a worktree: everything in it, kept as a commit under a reference of its round and task, without changing
//! the worktree, its index or the files. The snapshot is the part of a run's work that nobody else can see when the run is
//! cut; taking it is read-only for the checkout.

use std::ffi::{OsStr, OsString};
use std::path::Path;
use std::str::FromStr;

use agent_core::config::EnvName;
use agent_core::ids::{RoundId, TaskId};
use agent_core::state::CommitId;

use super::changes::{close_scratch, make_scratch};
use super::{Git, GitError, GitOperation, Reach, Revision, ScratchError, arg, diagnostic, text, whole};

/// The namespace of the snapshot references: `<prefix>/<round>/<task>/<n>`.
const SNAPSHOT_REFS: &str = "refs/agents/snapshots";

/// The name of the environment variable that points git at the scratch index of a snapshot.
const INDEX_VARIABLE: &str = "GIT_INDEX_FILE";

/// A snapshot taken by [`Git::snapshot`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Snapshot {
    /// The number of the snapshot among those of its task, counted from 1.
    pub number: u32,
    /// The commit that holds the snapshot's tree.
    pub commit: CommitId,
    /// The head of the worktree the snapshot stands on: the parent of [`Snapshot::commit`].
    pub head: CommitId,
    /// The bytes of the files the snapshot holds.
    pub bytes: u64,
}

impl Git {
    /// Keeps everything in the worktree — modified, staged, deleted, untracked and binary files; ignored files are left out
    /// — as a commit under `refs/agents/snapshots/<round>/<task>/<n>`, whose parent is the head.
    ///
    /// The tree is built in a scratch index, so the worktree's index and files are not touched; the objects are written to
    /// the repository, where the reference keeps them. The repository's filters, hooks and filesystem monitor are switched
    /// off as for a read of the uncommitted changes.
    ///
    /// # Errors
    /// [`GitError`] of the git commands that read the head, the worktree and the references, and write the commit; the
    /// scratch directory is removed again whatever happens.
    pub async fn snapshot(&self, round: &RoundId, task: &TaskId) -> Result<Snapshot, GitError> {
        let head = self.rev_parse(&Revision::Head).await?;
        let prefix = format!("{SNAPSHOT_REFS}/{round}/{task}");
        let number = self.next_snapshot_number(&prefix).await?;
        let scratch = make_scratch().await?;
        let written = self.write_snapshot(scratch.path(), &head, round, task, number).await;
        close_scratch(scratch).await;
        let (commit, bytes) = written?;
        Ok(Snapshot {
            number,
            commit,
            head,
            bytes,
        })
    }

    /// The number of the next snapshot of a task: one more than the references under `prefix` already name.
    async fn next_snapshot_number(&self, prefix: &str) -> Result<u32, GitError> {
        let operation = GitOperation::ForEachRef;
        let pattern = format!("{prefix}/");
        let captured = self
            .run_ok(
                operation,
                Reach::Local,
                &[arg("for-each-ref"), arg("--format=%(refname)"), arg(&pattern)],
            )
            .await?;
        whole(operation, &captured.stdout, self.limits)?;
        let listed = text(operation, &captured.stdout)?;
        let mut count = 0_u32;
        for _line in listed.lines() {
            count = count.checked_add(1).ok_or_else(|| GitError::Unexpected {
                operation,
                output: diagnostic(&captured.stdout.bytes),
            })?;
        }
        count.checked_add(1).ok_or_else(|| GitError::Unexpected {
            operation,
            output: diagnostic(&captured.stdout.bytes),
        })
    }

    /// Builds the tree of the worktree in a scratch index in `scratch`, commits it on `head` and points the next reference
    /// of the task at the commit. Returns the commit and the bytes of its files.
    async fn write_snapshot(
        &self,
        scratch: &Path,
        head: &CommitId,
        round: &RoundId,
        task: &TaskId,
        number: u32,
    ) -> Result<(CommitId, u64), GitError> {
        let index_path = scratch.join("index");
        let index = index_path.to_str().ok_or_else(|| GitError::Scratch {
            source: ScratchError::NotText(index_path.clone()),
        })?;
        let name = EnvName::from_str(INDEX_VARIABLE).map_err(|source| GitError::Scratch {
            source: ScratchError::Name {
                name: INDEX_VARIABLE,
                source,
            },
        })?;
        let scratch_git = self.clone().with_env(name, index);
        let hooks_path = scratch.join("hooks");
        let hooks = hooks_path.to_str().ok_or_else(|| GitError::Scratch {
            source: ScratchError::NotText(hooks_path.clone()),
        })?;
        let overrides = scratch_git.capture_overrides(hooks).await?;
        let head_text = head.to_string();
        scratch_git
            .run_scratch(GitOperation::ReadTree, &overrides, &[arg("read-tree"), arg(&head_text)])
            .await?;
        scratch_git
            .run_scratch(GitOperation::Add, &overrides, &[arg("add"), arg("-A"), arg("--")])
            .await?;
        let tree = scratch_git
            .scratch_text(GitOperation::WriteTree, &overrides, &[arg("write-tree")])
            .await?;
        let message = format!("Snapshot of {task} in round {round}, number {number}");
        let commit_text = self
            .scratch_text(
                GitOperation::CommitTree,
                &overrides,
                &[
                    arg("commit-tree"),
                    arg(&tree),
                    arg("-p"),
                    arg(&head_text),
                    arg("-m"),
                    arg(&message),
                ],
            )
            .await?;
        let commit: CommitId = commit_text.parse().map_err(|source| GitError::NotACommit {
            operation: GitOperation::CommitTree,
            source,
        })?;
        let reference = format!("{SNAPSHOT_REFS}/{round}/{task}/{number}");
        let commit_str = commit.to_string();
        self.run_ok(
            GitOperation::UpdateRef,
            Reach::Local,
            &[arg("update-ref"), arg(&reference), arg(&commit_str)],
        )
        .await?;
        let bytes = self.tree_bytes(&tree).await?;
        Ok((commit, bytes))
    }

    /// What a command of the scratch index prints, without its last line break.
    async fn scratch_text(
        &self,
        operation: GitOperation,
        overrides: &[OsString],
        args: &[&OsStr],
    ) -> Result<String, GitError> {
        let captured = self.run_scratch(operation, overrides, args).await?;
        whole(operation, &captured.stdout, self.limits)?;
        Ok(text(operation, &captured.stdout)?.trim_end().to_owned())
    }

    /// The bytes of the blobs of `tree`, all of them: what the snapshot holds.
    async fn tree_bytes(&self, tree: &str) -> Result<u64, GitError> {
        let operation = GitOperation::LsTree;
        let captured = self
            .run_ok(
                operation,
                Reach::Local,
                &[arg("ls-tree"), arg("-r"), arg("-l"), arg("-z"), arg(tree)],
            )
            .await?;
        let listed = whole(operation, &captured.stdout, self.limits)?;
        let mut total = 0_u64;
        for entry in listed.split(|byte| *byte == 0).filter(|entry| !entry.is_empty()) {
            let Some(tab_pos) = entry.iter().position(|byte| *byte == b'\t') else {
                return Err(GitError::Unexpected {
                    operation,
                    output: diagnostic(entry),
                });
            };
            let (meta, _path) = entry.split_at(tab_pos);
            let meta_text =
                String::from_utf8(meta.to_vec()).map_err(|source| GitError::NotUtf8 { operation, source })?;
            let size = meta_text
                .split_whitespace()
                .last()
                .ok_or_else(|| GitError::Unexpected {
                    operation,
                    output: diagnostic(entry),
                })?;
            if size == "-" {
                continue;
            }
            let bytes: u64 = size.parse().map_err(|source| GitError::NotANumber {
                operation,
                text: size.to_owned(),
                source,
            })?;
            // A sum of file sizes cannot exceed `u64` in practice; saturating keeps the record instead of failing it.
            total = total.saturating_add(bytes);
        }
        Ok(total)
    }
}
