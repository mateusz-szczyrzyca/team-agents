//! Bringing the commit of a worker's step to the requester's worktree.
//!
//! A worker's run ends `STATUS: DONE` with its commit on its own branch. Nothing of it may reach the requester's branch
//! before the request's check passes, and the requester's uncommitted and untracked work must be exactly as it was
//! whatever happens.
//!
//! The way chosen is a merge of the commits' trees made from the objects alone, not a cherry-pick and not
//! `git merge`: both read the requester's worktree and refuse, or stop in the middle of the merge, when a file they must
//! change is changed uncommitted, and the requester goes on working in that worktree. [`Git::merge_into_scratch`]
//! settles the merge of the worker's commit into the requester's head in a scratch directory and writes it as a commit
//! on the head; the requester's worktree is neither read nor written, so a conflict cannot touch it. [`Git::bring_over`]
//! takes the snapshot of the requester's worktree first, then merges, then moves the worktree and the branch to the merge
//! with `git read-tree -m -u`, which writes only the files the merge changes, keeps the requester's uncommitted and
//! untracked files, and refuses rather than overwrite a file the requester changed.

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;
use std::str::FromStr;

use agent_core::config::EnvName;
use agent_core::ids::{RoundId, TaskId};
use agent_core::repo_path::RepoPath;
use agent_core::state::CommitId;

use super::changes::{close_scratch, make_scratch};
use super::snapshot::Snapshot;
use super::{Git, GitError, GitOperation, Reach, Revision, ScratchError, arg, diagnostic, whole};

/// The name of the environment variable that points git at the index of a merge.
const INDEX_VARIABLE: &str = "GIT_INDEX_FILE";

/// The ID of the tree with no file, which every repository knows without holding it: the base of a merge of two
/// histories with no commit in common.
const EMPTY_TREE: &str = "4b825dc642cb6eb9a060e54bf8d69288fbee4904";

/// What bringing a worker's commit over the requester's head came to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BringOver {
    /// The worker's commit merged cleanly and is now on the requester's head: the commit written for the merge, whose
    /// parent is the head the requester had.
    Applied {
        /// The snapshot taken of the requester's worktree before the merge was written.
        snapshot: Snapshot,
        /// The commit that holds the requester's head with the worker's change merged into it.
        commit: CommitId,
    },
    /// The merge applies to the branch, but writing it would overwrite files the requester changed uncommitted, or
    /// untracked files in its way: nothing was written, and the worker's commit is returned as the reference.
    InTheWay {
        /// The snapshot taken of the requester's worktree before the merge was tried.
        snapshot: Snapshot,
        /// The worker's commit, as a reference: nothing of it is on the requester's branch.
        worker: CommitId,
        /// The files the merge writes that the requester has changed, or has untracked, in its worktree.
        files: Vec<RepoPath>,
    },
    /// The worker's commit conflicts with the requester's head: nothing of it was applied, the requester's worktree,
    /// index and files are as before, and the worker's commit is returned as the reference to do the step from.
    Conflicted {
        /// The snapshot taken of the requester's worktree before the merge was tried.
        snapshot: Snapshot,
        /// The worker's commit, as a reference: nothing of it is on the requester's branch.
        worker: CommitId,
        /// The paths of the files the merge could not settle, oldest path first.
        files: Vec<RepoPath>,
    },
}

/// One entry of a tree: the mode of the file or submodule and the object it holds.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Entry {
    /// `100644`, `100755`, `120000` for a symbolic link, or `160000` for a submodule.
    mode: String,
    /// The ID of the blob, or of the commit of a submodule.
    object: String,
}

/// What the objects of a merge came to.
enum Settled {
    /// The merged tree, written in the scratch index.
    Clean(String),
    /// The files the two sides changed differently.
    Conflicts(Vec<RepoPath>),
}

/// What a merge in a scratch directory came to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Merged {
    /// The merge applies: the commit that holds it, on the requester's head.
    Clean(CommitId),
    /// The merge conflicts: the files it could not settle.
    Conflicts(Vec<RepoPath>),
}

/// What writing a merge to the requester's worktree and branch came to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Written {
    /// The worktree and the branch are at the merge.
    Done,
    /// Files the merge writes are changed uncommitted, or untracked, in the requester's worktree: nothing was written.
    InTheWay {
        /// Those files.
        files: Vec<RepoPath>,
    },
}

impl Git {
    /// The files `commit` changes against its first parent, each path once, a rename as both the old and the new path.
    ///
    /// # Errors
    /// [`GitError::Failed`] when the commit is not in the repository; [`GitError::Unexpected`], [`GitError::NotUtf8`],
    /// [`GitError::NotARepoPath`], [`GitError::Run`] and [`GitError::OutputTooLong`] for what git prints.
    pub async fn changed_in(&self, commit: &CommitId) -> Result<Vec<RepoPath>, GitError> {
        let operation = GitOperation::DiffTree;
        let revision = commit.to_string();
        let captured = self
            .run_ok(
                operation,
                Reach::Local,
                &[
                    arg("diff-tree"),
                    arg("--no-commit-id"),
                    arg("--name-only"),
                    arg("--no-renames"),
                    arg("-r"),
                    arg("-z"),
                    arg("--root"),
                    arg(&revision),
                ],
            )
            .await?;
        let listed = whole(operation, &captured.stdout, self.limits)?;
        let mut paths = Vec::new();
        for name in listed.split(|byte| *byte == 0).filter(|name| !name.is_empty()) {
            paths.push(repo_path(operation, name)?);
        }
        paths.sort();
        paths.dedup();
        Ok(paths)
    }

    /// The files `commit` changes that `allowed` does not name: a path the request did not give the worker.
    ///
    /// # Errors
    /// The errors of [`Git::changed_in`].
    pub async fn outside(&self, commit: &CommitId, allowed: &[RepoPath]) -> Result<Vec<RepoPath>, GitError> {
        let changed = self.changed_in(commit).await?;
        Ok(changed.into_iter().filter(|path| !allowed.contains(path)).collect())
    }

    /// Merges `commit` into the requester's head with an index and an object directory in a scratch directory, and
    /// returns the commit that holds the merged tree on the head: the requester's worktree and its index are neither read
    /// nor written, so nothing of the merge is written when it conflicts, and then the files it could not settle are
    /// returned instead.
    ///
    /// # Errors
    /// [`GitError::Failed`] when the commit or the head is not in the repository; [`GitError::Scratch`] when the scratch
    /// directory cannot be made; [`GitError::NotACommit`] when the written commit is not one; [`GitError::Run`],
    /// [`GitError::OutputTooLong`], [`GitError::Unexpected`] and [`GitError::NotUtf8`].
    pub async fn merge_into_scratch(
        &self,
        head: &CommitId,
        commit: &CommitId,
        round: &RoundId,
        task: &TaskId,
    ) -> Result<Merged, GitError> {
        let scratch = make_scratch().await?;
        let merged = self.write_merge(scratch.path(), head, commit, round, task).await;
        close_scratch(scratch).await;
        merged
    }

    /// Merges `commit` into `head` in `scratch` and writes the merged tree as a commit on `head`, under a reference of
    /// the round and the task, so that the objects it wrote have an owner while it is written to the worktree.
    async fn write_merge(
        &self,
        scratch: &std::path::Path,
        head: &CommitId,
        commit: &CommitId,
        round: &RoundId,
        task: &TaskId,
    ) -> Result<Merged, GitError> {
        // The merge is read in a scratch index and written in the repository's own objects: the merged commit must
        // outlive the scratch directory to be written to the worktree, and the reference of the round and task keeps
        // it. Nothing is written to the worktree or its index here.
        let handle = self.clone().with_env(
            index_name()?,
            scratch
                .join("index")
                .to_str()
                .map(str::to_owned)
                .ok_or_else(|| GitError::Scratch {
                    source: ScratchError::NotText(scratch.join("index")),
                })?,
        );
        // No hooks run and no filter is asked: the merge writes the bytes of commits, not the output of programs of the
        // repository's configuration.
        let overrides = handle
            .capture_overrides(scratch.join("hooks").to_str().ok_or_else(|| GitError::Scratch {
                source: ScratchError::NotText(scratch.join("hooks")),
            })?)
            .await?;
        // The tree is settled from the objects alone: `merge-recursive` reads the worktree and a file the requester
        // changed uncommitted, or a directory where one side has a file, makes it refuse before it writes. What it
        // would have written is exactly what the two sides' trees say, and the requester's files play no part in it.
        match handle.settle_tree(&overrides, head, commit).await? {
            Settled::Clean(tree) => handle.commit_tree(&overrides, &tree, head, round, task).await,
            Settled::Conflicts(conflicts) => Ok(Merged::Conflicts(conflicts)),
        }
    }

    /// Writes `tree` as a commit on `head`, under a reference of the round and the task, and returns it.
    async fn commit_tree(
        &self,
        overrides: &[OsString],
        tree: &str,
        head: &CommitId,
        round: &RoundId,
        task: &TaskId,
    ) -> Result<Merged, GitError> {
        let head_text = head.to_string();
        let message = format!("The worker's change brought over {head_text} for {task} in round {round}");
        let written = self
            .scratch_text(
                GitOperation::CommitTree,
                overrides,
                &[
                    arg("commit-tree"),
                    arg(tree),
                    arg("-p"),
                    arg(&head_text),
                    arg("-m"),
                    arg(&message),
                ],
            )
            .await?;
        let applied: CommitId = written.parse().map_err(|source| GitError::NotACommit {
            operation: GitOperation::CommitTree,
            source,
        })?;
        let reference = format!("refs/agents/brought/{round}/{task}/{applied}");
        let applied_text = applied.to_string();
        self.run_scratch(
            GitOperation::UpdateRef,
            overrides,
            &[arg("update-ref"), arg(&reference), arg(&applied_text)],
        )
        .await?;
        Ok(Merged::Clean(applied))
    }

    /// The tree of the requester's head with `worker` merged into it, built from the objects alone: each file takes the
    /// side that changed it, and a file both sides changed differently, or one side added where the other has one, is a
    /// conflict. A file removed on one side and left as it was on the other is removed.
    async fn settle_tree(
        &self,
        overrides: &[OsString],
        head: &CommitId,
        worker: &CommitId,
    ) -> Result<Settled, GitError> {
        let base = match self
            .merge_base(&Revision::Head, &Revision::Commit(worker.clone()))
            .await
        {
            Ok(base) => base.to_string(),
            // No common ancestor: the worker's commit adds a history of its own, and the empty tree is its base.
            Err(GitError::NoMergeBase { .. }) => EMPTY_TREE.to_owned(),
            Err(error) => return Err(error),
        };
        let base = self.tree_entries(overrides, &base).await?;
        let ours = self.tree_entries(overrides, &head.to_string()).await?;
        let theirs = self.tree_entries(overrides, &worker.to_string()).await?;
        let mut paths: BTreeSet<&String> = BTreeSet::new();
        paths.extend(base.keys());
        paths.extend(ours.keys());
        paths.extend(theirs.keys());
        let mut merged = BTreeMap::new();
        let mut conflicts = Vec::new();
        for path in paths {
            let (base_side, our_side, their_side) = (base.get(path), ours.get(path), theirs.get(path));
            let taken = if our_side == their_side {
                our_side.cloned()
            } else if our_side == base_side {
                // Only the worker changed it, or only the worker has it.
                their_side.cloned()
            } else if their_side == base_side {
                // Only the requester changed it, or only the requester has it.
                our_side.cloned()
            } else if our_side.is_none() || their_side.is_none() {
                // Each side changed it, one of them by removing it: nothing shows which change wins.
                conflicts.push(path_of(GitOperation::Diff, path)?);
                None
            } else {
                conflicts.push(path_of(GitOperation::Diff, path)?);
                None
            };
            if let Some(entry) = taken {
                merged.insert(path.clone(), entry);
            }
        }
        if !conflicts.is_empty() {
            return Ok(Settled::Conflicts(conflicts));
        }
        let tree = self.scratch_index_from(&merged, overrides).await?;
        Ok(Settled::Clean(tree))
    }

    /// The entries of `tree`: each path with the mode and the object of its blob, as `git ls-tree -r -z` prints them.
    async fn tree_entries(&self, overrides: &[OsString], tree: &str) -> Result<BTreeMap<String, Entry>, GitError> {
        let operation = GitOperation::LsTree;
        let captured = self
            .run_scratch(operation, overrides, &[arg("ls-tree"), arg("-r"), arg("-z"), arg(tree)])
            .await?;
        let listed = whole(operation, &captured.stdout, self.limits)?;
        let mut entries = BTreeMap::new();
        for line in listed.split(|byte| *byte == 0).filter(|line| !line.is_empty()) {
            let Some(tab) = line.iter().position(|byte| *byte == b'\t') else {
                return Err(GitError::Unexpected {
                    operation,
                    output: diagnostic(line),
                });
            };
            let (meta, rest) = line.split_at(tab);
            let path = rest.get(1..).unwrap_or_default();
            let meta = String::from_utf8(meta.to_vec()).map_err(|source| GitError::NotUtf8 { operation, source })?;
            let mut parts = meta.split_whitespace();
            let (Some(mode), Some(_kind), Some(object)) = (parts.next(), parts.next(), parts.next()) else {
                return Err(GitError::Unexpected {
                    operation,
                    output: diagnostic(line),
                });
            };
            let path = String::from_utf8_lossy(path).into_owned();
            let entry = Entry {
                mode: mode.to_owned(),
                object: object.to_owned(),
            };
            // A submodule is another repository's commit: it is kept as the entry the side that has it wrote.
            entries.insert(path, entry);
        }
        Ok(entries)
    }

    /// Writes `entries` as a tree in the scratch index and returns its ID: the merged tree of [`Git::settle_tree`].
    async fn scratch_index_from(
        &self,
        entries: &BTreeMap<String, Entry>,
        overrides: &[OsString],
    ) -> Result<String, GitError> {
        let operation = GitOperation::ReadTree;
        self.run_scratch(operation, overrides, &[arg("read-tree"), arg("--empty")])
            .await?;
        for (path, entry) in entries {
            let cacheinfo = format!("{},{},{path}", entry.mode, entry.object);
            let operation = GitOperation::UpdateIndex;
            self.run_scratch(
                operation,
                overrides,
                &[arg("update-index"), arg("--add"), arg("--cacheinfo"), arg(&cacheinfo)],
            )
            .await?;
        }
        self.scratch_text(GitOperation::WriteTree, overrides, &[arg("write-tree")])
            .await
    }

    /// Brings `commit` to the requester's branch after a snapshot of the requester's worktree: the snapshot is taken
    /// first, the merge is settled in a scratch directory, and only a merge that applies is written, to the worktree
    /// and then to the branch, so the head moves to the merge commit and `git status` shows nothing of it.
    ///
    /// # Errors
    /// The errors of [`Git::snapshot`] and [`Git::merge_into_scratch`]; [`GitError::Failed`] when writing the merge
    /// would overwrite a file the requester changed or an untracked file in its way, or the branch moved meanwhile; in
    /// both cases the worktree is as it was.
    pub async fn bring_over(&self, commit: &CommitId, round: &RoundId, task: &TaskId) -> Result<BringOver, GitError> {
        let snapshot = self.snapshot(round, task).await?;
        match self.merge_into_scratch(&snapshot.head, commit, round, task).await? {
            Merged::Conflicts(files) => Ok(BringOver::Conflicted {
                snapshot,
                worker: commit.clone(),
                files,
            }),
            Merged::Clean(applied) => match self.write_merged(&snapshot.head, &applied).await? {
                Written::Done => Ok(BringOver::Applied {
                    snapshot,
                    commit: applied,
                }),
                Written::InTheWay { files } => Ok(BringOver::InTheWay {
                    snapshot,
                    worker: commit.clone(),
                    files,
                }),
            },
        }
    }

    /// Writes the merge `applied`, made on `head`, to the worktree and the branch.
    ///
    /// # Errors
    /// The errors of [`Git::write_applied`].
    pub async fn write_merged(&self, head: &CommitId, applied: &CommitId) -> Result<Written, GitError> {
        let _ = head;
        self.write_applied(applied).await?;
        Ok(Written::Done)
    }

    /// Moves the index and the files of the worktree from the head to `applied`, whose parent is the head, then the
    /// branch. `read-tree -m -u` writes only the files that differ between the two trees and refuses, changing nothing,
    /// when one of them is changed uncommitted or an untracked file is in its way. The branch moves only if it is still at
    /// the head the merge was made on; when it is not, the files are put back.
    async fn write_applied(&self, applied: &CommitId) -> Result<(), GitError> {
        let head = self.rev_parse(&Revision::Head).await?;
        let (head_text, applied_text) = (head.to_string(), applied.to_string());
        self.read_tree_update(&head_text, &applied_text).await?;
        let message = format!("Bring over the worker's change {applied_text}");
        let moved = self
            .run_ok(
                GitOperation::UpdateRef,
                Reach::Local,
                &[
                    arg("update-ref"),
                    arg("-m"),
                    arg(&message),
                    arg("HEAD"),
                    arg(&applied_text),
                    arg(&head_text),
                ],
            )
            .await;
        if let Err(error) = moved {
            // The requester committed while the merge was written: the files go back to the head it had, and the merge is
            // reported as not applied. A failure to put them back is the worse one and is the one reported.
            self.read_tree_update(&applied_text, &head_text).await?;
            return Err(error);
        }
        Ok(())
    }

    /// Moves the index and the worktree from the tree of `from` to the tree of `to`: `git read-tree -m -u`.
    async fn read_tree_update(&self, from: &str, to: &str) -> Result<(), GitError> {
        self.run_ok(
            GitOperation::ReadTree,
            Reach::Local,
            &[arg("read-tree"), arg("-m"), arg("-u"), arg(from), arg(to)],
        )
        .await?;
        Ok(())
    }
}

/// The name of the environment variable that points git at the index of a merge.
fn index_name() -> Result<EnvName, GitError> {
    EnvName::from_str(INDEX_VARIABLE).map_err(|source| GitError::Scratch {
        source: ScratchError::Name {
            name: INDEX_VARIABLE,
            source,
        },
    })
}

/// A repository-relative path from a path of a tree.
fn path_of(operation: GitOperation, path: &str) -> Result<RepoPath, GitError> {
    RepoPath::try_from(path.to_owned()).map_err(|source| GitError::NotARepoPath { operation, source })
}

/// A repository-relative path from what git printed.
fn repo_path(operation: GitOperation, bytes: &[u8]) -> Result<RepoPath, GitError> {
    let text = String::from_utf8(bytes.to_vec()).map_err(|source| GitError::NotUtf8 { operation, source })?;
    RepoPath::try_from(text).map_err(|source| GitError::NotARepoPath { operation, source })
}
