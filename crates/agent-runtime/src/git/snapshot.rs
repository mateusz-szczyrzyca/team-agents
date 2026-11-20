//! A snapshot of a worktree: everything in it, kept as a commit under a reference of its round and task, without changing
//! the worktree, its index or the files. The snapshot is the part of a run's work that nobody else can see when the run is
//! cut; taking it is read-only for the checkout.

use std::collections::{BTreeMap, BTreeSet};
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
        let number = self.next_snapshot_number(task).await?;
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

    /// The number of the next snapshot of `task`: one more than the highest number any reference of `task` under
    /// [`SNAPSHOT_REFS`] already has, across all rounds.
    async fn next_snapshot_number(&self, task: &TaskId) -> Result<u32, GitError> {
        let operation = GitOperation::ForEachRef;
        let pattern = format!("{SNAPSHOT_REFS}/");
        let captured = self
            .run_ok(
                operation,
                Reach::Local,
                &[arg("for-each-ref"), arg("--format=%(refname)"), arg(&pattern)],
            )
            .await?;
        whole(operation, &captured.stdout, self.limits)?;
        let listed = text(operation, &captured.stdout)?;
        let mut highest = 0_u32;
        for line in listed.lines() {
            let Some(rest) = line.strip_prefix(SNAPSHOT_REFS).and_then(|s| s.strip_prefix('/')) else {
                continue;
            };
            let Some((_round, after_round)) = rest.split_once('/') else {
                continue;
            };
            let Some((ref_task, number_text)) = after_round.split_once('/') else {
                continue;
            };
            if ref_task != task.as_str() {
                continue;
            }
            let parsed: u32 = number_text.parse().map_err(|source| GitError::NotANumber {
                operation,
                text: number_text.to_owned(),
                source,
            })?;
            highest = highest.max(parsed);
        }
        highest.checked_add(1).ok_or_else(|| GitError::Unexpected {
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
        let head_entries = scratch_git.staged_regular_files(&overrides).await?;
        let dirty_paths = scratch_git.worktree_dirty_paths(&overrides).await?;
        scratch_git
            .run_scratch(GitOperation::Add, &overrides, &[arg("add"), arg("-A"), arg("--")])
            .await?;
        scratch_git
            .store_verbatim_blobs(&overrides, &head_entries, &dirty_paths)
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
        self.run_scratch(
            GitOperation::UpdateRef,
            &overrides,
            &[arg("update-ref"), arg(&reference), arg(&commit_str), arg("")],
        )
        .await?;
        let bytes = self.tree_bytes(&tree).await?;
        Ok((commit, bytes))
    }

    /// The regular files (`100644` and `100755`) in the scratch index, mapped from path to `(mode, oid)`.
    async fn staged_regular_files(
        &self,
        overrides: &[OsString],
    ) -> Result<BTreeMap<String, (String, String)>, GitError> {
        let operation = GitOperation::LsFiles;
        let captured = self
            .run_scratch(operation, overrides, &[arg("ls-files"), arg("-z"), arg("-s")])
            .await?;
        let listed = whole(operation, &captured.stdout, self.limits)?;
        let mut files = BTreeMap::new();
        for entry in listed.split(|byte| *byte == 0).filter(|entry| !entry.is_empty()) {
            let line = String::from_utf8(entry.to_vec()).map_err(|source| GitError::NotUtf8 { operation, source })?;
            let Some((meta, path)) = line.split_once('\t') else {
                return Err(GitError::Unexpected {
                    operation,
                    output: diagnostic(entry),
                });
            };
            let mut parts = meta.split_whitespace();
            let (Some(mode), Some(oid)) = (parts.next(), parts.next()) else {
                return Err(GitError::Unexpected {
                    operation,
                    output: diagnostic(entry),
                });
            };
            if mode == "100644" || mode == "100755" {
                files.insert(path.to_owned(), (mode.to_owned(), oid.to_owned()));
            }
        }
        Ok(files)
    }

    /// The worktree paths that are modified or untracked (excluding ignored files) relative to the scratch index.
    async fn worktree_dirty_paths(&self, overrides: &[OsString]) -> Result<BTreeSet<String>, GitError> {
        let operation = GitOperation::LsFiles;
        let captured = self
            .run_scratch(
                operation,
                overrides,
                &[
                    arg("ls-files"),
                    arg("-z"),
                    arg("--modified"),
                    arg("--others"),
                    arg("--exclude-standard"),
                ],
            )
            .await?;
        let listed = whole(operation, &captured.stdout, self.limits)?;
        let mut paths = BTreeSet::new();
        for entry in listed.split(|byte| *byte == 0).filter(|entry| !entry.is_empty()) {
            let path = String::from_utf8(entry.to_vec()).map_err(|source| GitError::NotUtf8 { operation, source })?;
            paths.insert(path);
        }
        Ok(paths)
    }

    /// Replaces any regular file blob in the scratch index that `git add` altered through built-in `.gitattributes` rules
    /// (`text`, `eol`, `crlf`, `ident`, `working-tree-encoding`) with the exact bytes of the worktree file via
    /// `git hash-object -w --no-filters`.
    async fn store_verbatim_blobs(
        &self,
        overrides: &[OsString],
        head_entries: &BTreeMap<String, (String, String)>,
        dirty_paths: &BTreeSet<String>,
    ) -> Result<(), GitError> {
        let current = self.staged_regular_files(overrides).await?;
        for (path, (mode, oid)) in current {
            let changed_from_head = head_entries
                .get(&path)
                .is_none_or(|(head_mode, head_oid)| head_mode != &mode || head_oid != &oid);
            if !changed_from_head && !dirty_paths.contains(&path) {
                continue;
            }
            let raw_oid = self
                .scratch_text(
                    GitOperation::HashObject,
                    overrides,
                    &[
                        arg("hash-object"),
                        arg("-w"),
                        arg("--no-filters"),
                        arg("--"),
                        arg(&path),
                    ],
                )
                .await?;
            if raw_oid != oid {
                let cacheinfo = format!("{mode},{raw_oid},{path}");
                self.run_scratch(
                    GitOperation::UpdateIndex,
                    overrides,
                    &[arg("update-index"), arg("--cacheinfo"), arg(&cacheinfo)],
                )
                .await?;
            }
        }
        Ok(())
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
