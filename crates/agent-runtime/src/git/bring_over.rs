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
//! takes the snapshot of the requester's worktree first, then merges on the head the snapshot was taken at, then, if the
//! branch is still there, moves the worktree and the branch to the merge
//! with `git read-tree -m -u`, which writes only the files the merge changes, keeps the requester's uncommitted and
//! untracked files, and refuses rather than overwrite a file the requester changed.

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::{OsStr, OsString};
use std::os::unix::ffi::OsStrExt;
use std::str::FromStr;

use agent_core::config::EnvName;
use agent_core::ids::{RoundId, TaskId};
use agent_core::repo_path::RepoPath;
use agent_core::state::CommitId;

use super::changes::{close_scratch, make_scratch};
use super::snapshot::Snapshot;
use super::{
    Git, GitError, GitLimits, GitOperation, Reach, Revision, ScratchError, arg, diagnostic, failed, text, whole,
};
use crate::process::{CaptureLimits, capture_with_input};

/// The name of the environment variable that points git at the index of a merge.
const INDEX_VARIABLE: &str = "GIT_INDEX_FILE";

/// The bytes of the output of a listing of a whole tree that are kept: 64 MiB, which is a tree of about 600,000 files.
const TREE_OUTPUT_BYTES: u64 = 64 * 1024 * 1024;

/// What the line of the message of a commit brought over starts with, before the worker's commit: how
/// [`Git::brought_over`] finds it again.
const BROUGHT_OVER_FROM: &str = "Brought over from";

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
    /// The files the worker's `commit` brings: those it changes since the merge base with the requester's head, which is
    /// the range [`Git::bring_over`] brings in, whatever the shape of the worker's history (several commits, a merge). A
    /// history with no commit in common with the head counts from the empty tree. Each path once, a rename as both the
    /// old and the new path.
    ///
    /// # Errors
    /// [`GitError::Failed`] when the commit is not in the repository; [`GitError::Unexpected`], [`GitError::NotUtf8`],
    /// [`GitError::NotARepoPath`], [`GitError::Run`] and [`GitError::OutputTooLong`] for what git prints.
    pub async fn changed_in(&self, commit: &CommitId) -> Result<Vec<RepoPath>, GitError> {
        let base = match self
            .merge_base(&Revision::Head, &Revision::Commit(commit.clone()))
            .await
        {
            Ok(base) => base.to_string(),
            Err(GitError::NoMergeBase { .. }) => EMPTY_TREE.to_owned(),
            Err(error) => return Err(error),
        };
        let operation = GitOperation::DiffTree;
        let revision = commit.to_string();
        let captured = self
            .run_ok(
                operation,
                Reach::Local,
                &[
                    arg("diff-tree"),
                    arg("--name-only"),
                    arg("--no-renames"),
                    arg("-r"),
                    arg("-z"),
                    arg(&base),
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

    /// The files that differ between the commits `from` and `to`, each path once: what a worker changed in its own
    /// worktree between two of its commits.
    ///
    /// # Errors
    /// The errors of [`Git::changed_in`].
    pub async fn changed_between(&self, from: &CommitId, to: &CommitId) -> Result<Vec<RepoPath>, GitError> {
        self.names_between(&from.to_string(), &to.to_string()).await
    }

    /// The paths that differ between the trees `from` and `to`, sorted, each once.
    async fn names_between(&self, from: &str, to: &str) -> Result<Vec<RepoPath>, GitError> {
        let operation = GitOperation::DiffTree;
        let captured = self
            .run_ok(
                operation,
                Reach::Local,
                &[
                    arg("diff-tree"),
                    arg("--name-only"),
                    arg("--no-renames"),
                    arg("-r"),
                    arg("-z"),
                    arg(from),
                    arg(to),
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

    /// The commit on the current branch that brought the worker's `commit` over, if there is one: the merge commit
    /// [`Git::bring_over`] writes names the worker's commit in a line of its message.
    ///
    /// # Errors
    /// [`GitError::Run`], [`GitError::Failed`], [`GitError::OutputTooLong`], [`GitError::NotUtf8`] and
    /// [`GitError::NotACommit`].
    pub async fn brought_over(&self, commit: &CommitId) -> Result<Option<CommitId>, GitError> {
        let operation = GitOperation::Log;
        let line = format!("{BROUGHT_OVER_FROM} {commit} ");
        let captured = self
            .run_ok(
                operation,
                Reach::Local,
                &[
                    arg("log"),
                    arg("-1"),
                    arg("--fixed-strings"),
                    arg("--format=%H"),
                    arg("--grep"),
                    arg(&line),
                ],
            )
            .await?;
        let printed = text(operation, &captured.stdout)?;
        let found = printed.trim();
        if found.is_empty() {
            return Ok(None);
        }
        found
            .parse()
            .map(Some)
            .map_err(|source| GitError::NotACommit { operation, source })
    }

    /// The files of the worktree that are changed, staged or untracked and not ignored: what a commit does not hold.
    ///
    /// # Errors
    /// [`GitError::Run`], [`GitError::Failed`], [`GitError::OutputTooLong`], [`GitError::Unexpected`] and
    /// [`GitError::NotARepoPath`].
    pub async fn uncommitted_files(&self) -> Result<Vec<RepoPath>, GitError> {
        Ok(Vec::new())
    }

    /// The files `commit` changes that `allowed` does not name: a path the request did not give the worker.
    ///
    /// # Errors
    /// The errors of [`Git::changed_in`].
    pub async fn outside(&self, commit: &CommitId, allowed: &[RepoPath]) -> Result<Vec<RepoPath>, GitError> {
        let changed = self.changed_in(commit).await?;
        Ok(changed.into_iter().filter(|path| !allowed.contains(path)).collect())
    }

    /// Merges `commit` into `head`, the head the requester is at, with an index of its own in a scratch directory, and
    /// returns the commit that holds the merged tree on `head`: the merge's trees and commit are written in the
    /// repository's own objects, under a reference of the round and the task. The requester's worktree and its index are
    /// neither read nor written, so nothing of the merge is written when it conflicts, and then the files it could not
    /// settle are returned instead. [`Git::write_merged`] writes the merge to the worktree.
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
        self.merge_since_into_scratch(head, None, commit, round, task).await
    }

    /// As [`Git::merge_into_scratch`], but what is merged is what `commit` changed since `since`; `None` is since the merge
    /// base with `head`.
    ///
    /// # Errors
    /// The errors of [`Git::merge_into_scratch`].
    pub async fn merge_since_into_scratch(
        &self,
        head: &CommitId,
        since: Option<&CommitId>,
        commit: &CommitId,
        round: &RoundId,
        task: &TaskId,
    ) -> Result<Merged, GitError> {
        let scratch = make_scratch().await?;
        let merged = self
            .write_merge(scratch.path(), (head, since), commit, round, task)
            .await;
        close_scratch(scratch).await;
        merged
    }

    /// Merges `commit` into `head` in `scratch` and writes the merged tree as a commit on `head`, under a reference of
    /// the round and the task, so that the objects it wrote have an owner while it is written to the worktree.
    async fn write_merge(
        &self,
        scratch: &std::path::Path,
        (head, since): (&CommitId, Option<&CommitId>),
        commit: &CommitId,
        round: &RoundId,
        task: &TaskId,
    ) -> Result<Merged, GitError> {
        // The merge is read in a scratch index and written in the repository's own objects: the merged commit must
        // outlive the scratch directory to be written to the worktree, and the reference of the round and task keeps
        // it. Nothing is written to the worktree or its index here.
        // A tree of a big repository is listed in more bytes than a command's output may be by default.
        let handle = self
            .clone()
            .with_limits(GitLimits {
                max_output_bytes: TREE_OUTPUT_BYTES,
                ..self.limits
            })
            .with_env(
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
        match handle.settle_tree(&overrides, (head, since), commit).await? {
            Settled::Clean(tree) => handle.commit_tree(&overrides, &tree, head, commit, round, task).await,
            Settled::Conflicts(conflicts) => Ok(Merged::Conflicts(conflicts)),
        }
    }

    /// The worker's message for the commit brought over, with a line that names the worker's commit, and a handle that
    /// commits under the worker's name and date: the requester's branch keeps what the worker wrote.
    async fn worker_message(
        &self,
        worker: &CommitId,
        task: &TaskId,
        round: &RoundId,
    ) -> Result<(Self, String), GitError> {
        let operation = GitOperation::Log;
        let revision = worker.to_string();
        let captured = self
            .run_ok(
                operation,
                Reach::Local,
                &[
                    arg("log"),
                    arg("-1"),
                    arg("--format=%an%x00%ae%x00%aI%x00%B"),
                    arg(&revision),
                ],
            )
            .await?;
        let printed = text(operation, &captured.stdout)?;
        let mut parts = printed.splitn(4, '\0');
        let (Some(name), Some(email), Some(date), Some(message)) =
            (parts.next(), parts.next(), parts.next(), parts.next())
        else {
            return Err(GitError::Unexpected {
                operation,
                output: diagnostic(printed.as_bytes()),
            });
        };
        let handle = self
            .clone()
            .with_env(env_name("GIT_AUTHOR_NAME")?, name)
            .with_env(env_name("GIT_AUTHOR_EMAIL")?, email)
            .with_env(env_name("GIT_AUTHOR_DATE")?, date);
        let message = format!(
            "{}\n\n{BROUGHT_OVER_FROM} {worker} for {task} in round {round}",
            message.trim_end()
        );
        Ok((handle, message))
    }

    /// Writes `tree` as a commit on `head`, under a reference of the round and the task, and returns it.
    async fn commit_tree(
        &self,
        overrides: &[OsString],
        tree: &str,
        head: &CommitId,
        worker: &CommitId,
        round: &RoundId,
        task: &TaskId,
    ) -> Result<Merged, GitError> {
        let head_text = head.to_string();
        let (author, message) = self.worker_message(worker, task, round).await?;
        let written = author
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
        (head, since): (&CommitId, Option<&CommitId>),
        worker: &CommitId,
    ) -> Result<Settled, GitError> {
        let base = match since {
            Some(since) => since.to_string(),
            None => match self
                .merge_base(&Revision::Commit(head.clone()), &Revision::Commit(worker.clone()))
                .await
            {
                Ok(base) => base.to_string(),
                // No common ancestor: the worker's commit adds a history of its own, and the empty tree is its base.
                Err(GitError::NoMergeBase { .. }) => EMPTY_TREE.to_owned(),
                Err(error) => return Err(error),
            },
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
            } else {
                // Each side changed it, one of them perhaps by removing it: nothing shows which change wins.
                conflicts.push(path_of(GitOperation::Diff, path)?);
                None
            };
            if let Some(entry) = taken {
                merged.insert(path.clone(), entry);
            }
        }
        // A file where the other side has a directory of that name cannot be written as a tree: both paths are named.
        for path in merged.keys() {
            for (slash, _) in path.match_indices('/') {
                let directory = path.get(..slash).unwrap_or_default();
                if merged.contains_key(directory) {
                    conflicts.push(path_of(GitOperation::Diff, directory)?);
                    conflicts.push(path_of(GitOperation::Diff, path)?);
                }
            }
        }
        if !conflicts.is_empty() {
            conflicts.sort();
            conflicts.dedup();
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
            let path = String::from_utf8(path.to_vec()).map_err(|source| GitError::NotUtf8 { operation, source })?;
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
        // One process for the whole tree: a line for each entry, ended by a NUL so that no name needs quoting.
        let mut lines = Vec::new();
        for (path, entry) in entries {
            lines.extend_from_slice(format!("{} {} 0\t{path}", entry.mode, entry.object).as_bytes());
            lines.push(0);
        }
        self.run_scratch_fed(
            GitOperation::UpdateIndex,
            overrides,
            &[arg("update-index"), arg("--add"), arg("-z"), arg("--index-info")],
            &lines,
        )
        .await?;
        self.scratch_text(GitOperation::WriteTree, overrides, &[arg("write-tree")])
            .await
    }

    /// Runs a scratch-index command and gives it `input` on its standard input.
    async fn run_scratch_fed(
        &self,
        operation: GitOperation,
        overrides: &[OsString],
        args: &[&OsStr],
        input: &[u8],
    ) -> Result<(), GitError> {
        let args: Vec<&OsStr> = overrides
            .iter()
            .map(OsString::as_os_str)
            .chain(args.iter().copied())
            .collect();
        let limits = CaptureLimits {
            timeout: self.limits.local_timeout,
            max_bytes: self.limits.max_output_bytes,
        };
        let captured = capture_with_input(self.command(&args), limits, input)
            .await
            .map_err(|source| GitError::Run { operation, source })?;
        if captured.exit.succeeded() {
            return Ok(());
        }
        Err(failed(operation, &captured))
    }

    /// Brings what the worker committed since `base`, the commit its worktree was at when the request was given to it, to
    /// the requester's branch, as [`Git::bring_over`] does: the commits before `base` — the rejected result of an earlier
    /// request of the same worker, or one that was brought over already — are not part of it.
    ///
    /// # Errors
    /// The errors of [`Git::bring_over`].
    pub async fn bring_over_since(
        &self,
        base: &CommitId,
        commit: &CommitId,
        round: &RoundId,
        task: &TaskId,
    ) -> Result<BringOver, GitError> {
        self.bring_over_from(Some(base), commit, round, task).await
    }

    /// Brings `commit` to the requester's branch after a snapshot of the requester's worktree: the snapshot is taken
    /// first, the merge is settled in a scratch directory, and only a merge that applies is written, to the worktree
    /// and then to the branch, so the head moves to the merge commit and `git status` shows nothing of it.
    ///
    /// # Errors
    /// The errors of [`Git::snapshot`], [`Git::merge_into_scratch`] and [`Git::write_merged`]: among them
    /// [`GitError::NotUtf8`] for a name that is not UTF-8, and [`GitError::HeadMoved`] when the requester committed while
    /// the merge was made. Files of the merge that the requester changed or has untracked are an answer, not an error:
    /// [`BringOver::InTheWay`]. Whatever the outcome, an error leaves the worktree and the branch as they were.
    pub async fn bring_over(&self, commit: &CommitId, round: &RoundId, task: &TaskId) -> Result<BringOver, GitError> {
        self.bring_over_from(None, commit, round, task).await
    }

    /// [`Git::bring_over`] and [`Git::bring_over_since`]: what is merged is what `commit` changed since `since`, or since
    /// the merge base with the requester's head.
    async fn bring_over_from(
        &self,
        since: Option<&CommitId>,
        commit: &CommitId,
        round: &RoundId,
        task: &TaskId,
    ) -> Result<BringOver, GitError> {
        // The snapshot lists every file of the worktree: a big repository needs more bytes than a command may print.
        let snapshot = self
            .clone()
            .with_limits(GitLimits {
                max_output_bytes: TREE_OUTPUT_BYTES,
                ..self.limits
            })
            .snapshot(round, task)
            .await?;
        match self
            .merge_since_into_scratch(&snapshot.head, since, commit, round, task)
            .await?
        {
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

    /// Writes the merge `applied`, made on `head`, to the worktree and then to the branch. Nothing is written unless the
    /// branch is still at `head` and no file the merge writes is changed or untracked in the worktree: the requester's
    /// own commit, made while the merge waited, would otherwise be written over and fall off the branch.
    ///
    /// `read-tree -m -u` writes only the files that differ between the two trees. The branch moves with `update-ref`
    /// against `head`, so that a commit made after the check above still keeps the branch where it is; then the files
    /// are put back.
    ///
    /// # Errors
    /// [`GitError::HeadMoved`] when the branch is not at `head`; [`GitError::Failed`] when git refuses to write the
    /// merge or the branch; the errors of reading the status and the trees.
    pub async fn write_merged(&self, head: &CommitId, applied: &CommitId) -> Result<Written, GitError> {
        let found = self.rev_parse(&Revision::Head).await?;
        if found != *head {
            return Err(GitError::HeadMoved {
                expected: head.clone(),
                found,
            });
        }
        let (head_text, applied_text) = (head.to_string(), applied.to_string());
        let files = self.in_the_way(&head_text, &applied_text).await?;
        if !files.is_empty() {
            return Ok(Written::InTheWay { files });
        }
        self.read_tree_update(&head_text, &applied_text).await?;
        self.move_branch(head, applied).await?;
        Ok(Written::Done)
    }

    /// Moves the branch from `head` to the merge `applied`, whose tree is already in the worktree and the index; when
    /// the branch cannot be moved, the files are put back.
    async fn move_branch(&self, head: &CommitId, applied: &CommitId) -> Result<(), GitError> {
        let (head_text, applied_text) = (head.to_string(), applied.to_string());
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
            // The files go back to the head the branch is at now, not to the one the merge was made on: when the requester
            // committed meanwhile, its commit is what the worktree must agree with, and it may be the staged merge itself.
            // The requester may commit again while they are put back, so the branch is read again after each step until
            // the files agree with it. A failure to put them back is the worse one and is the one reported.
            let mut at = applied.clone();
            let found = loop {
                let now = self.rev_parse(&Revision::Head).await?;
                if now != at {
                    self.read_tree_update(&at.to_string(), &now.to_string()).await?;
                    at = now;
                }
                if self.rev_parse(&Revision::Head).await? == at {
                    break at;
                }
            };
            if found != *head {
                return Err(GitError::HeadMoved {
                    expected: head.clone(),
                    found,
                });
            }
            return Err(error);
        }
        Ok(())
    }

    /// The files the merge `applied` writes over `head` that are in the requester's way: changed, staged or untracked in
    /// its worktree, and, for a file the merge adds, anything the worktree has at that path or in the place of a
    /// directory of it, ignored or not. `read-tree -u` would overwrite an ignored file without a word.
    async fn in_the_way(&self, head: &str, applied: &str) -> Result<Vec<RepoPath>, GitError> {
        let operation = GitOperation::DiffTree;
        let written = self
            .run_ok(
                operation,
                Reach::Local,
                &[
                    arg("diff-tree"),
                    arg("-r"),
                    arg("--name-status"),
                    arg("--no-renames"),
                    arg("-z"),
                    arg(head),
                    arg(applied),
                ],
            )
            .await?;
        let written = whole(operation, &written.stdout, self.limits)?;
        let mut changes = Vec::new();
        let mut added = Vec::new();
        let mut fields = written.split(|byte| *byte == 0).filter(|field| !field.is_empty());
        while let Some(kind) = fields.next() {
            let Some(path) = fields.next() else {
                return Err(GitError::Unexpected {
                    operation,
                    output: diagnostic(kind),
                });
            };
            changes.push(path);
            if kind == b"A" {
                added.push(path);
            }
        }
        let operation = GitOperation::Status;
        let status = self
            .run_ok(
                operation,
                Reach::Local,
                &[
                    arg("status"),
                    arg("--porcelain=v1"),
                    arg("-z"),
                    arg("--untracked-files=all"),
                ],
            )
            .await?;
        let status = whole(operation, &status.stdout, self.limits)?;
        let mut touched: BTreeSet<&[u8]> = BTreeSet::new();
        let mut fields = status.split(|byte| *byte == 0).filter(|field| !field.is_empty());
        while let Some(field) = fields.next() {
            let Some(path) = field.get(3..) else {
                return Err(GitError::Unexpected {
                    operation,
                    output: diagnostic(field),
                });
            };
            touched.insert(path);
            // A rename or a copy is followed by the path it came from.
            if (matches!(field.first(), Some(b'R' | b'C')) || matches!(field.get(1), Some(b'R' | b'C')))
                && let Some(origin) = fields.next()
            {
                touched.insert(origin);
            }
        }
        let mut files: BTreeSet<Vec<u8>> = changes
            .iter()
            .filter(|path| touched.contains(**path))
            .map(|path| path.to_vec())
            .collect();
        for path in added {
            if let Some(obstacle) = self.obstacle_in_worktree(head, path).await? {
                files.insert(obstacle);
            }
        }
        let mut found = Vec::new();
        for path in files {
            found.push(repo_path(GitOperation::DiffTree, &path)?);
        }
        found.sort();
        Ok(found)
    }

    /// What the worktree has where the merge adds `path` that the merge does not clear itself: the path itself, in
    /// whatever form, or the first file in the place of one of its directories. A tracked file the merge replaces by a
    /// directory, and a directory of tracked files it replaces by a file, are the merge's to clear (when they are clean;
    /// the status of the paths the merge writes says when they are not). `None` when the way is free.
    async fn obstacle_in_worktree(&self, head: &str, path: &[u8]) -> Result<Option<Vec<u8>>, GitError> {
        let root = self.dir.as_path();
        let mut prefix = Vec::new();
        for (n, part) in path.split(|byte| *byte == b'/').enumerate() {
            if n > 0 {
                prefix.push(b'/');
            }
            prefix.extend_from_slice(part);
            let last = prefix.len() == path.len();
            let metadata = match tokio::fs::symlink_metadata(root.join(OsStr::from_bytes(&prefix))).await {
                Ok(metadata) => metadata,
                Err(source) if source.kind() == std::io::ErrorKind::NotFound => return Ok(None),
                // A file where a directory of the path should be: git reports it as not a directory.
                Err(source) if source.kind() == std::io::ErrorKind::NotADirectory => {
                    return self.unless_tracked(head, prefix).await;
                }
                Err(source) => {
                    return Err(GitError::Scratch {
                        source: ScratchError::Make(source),
                    });
                }
            };
            if metadata.is_dir() {
                if last {
                    // A directory where the merge adds a file: in the way if it holds anything the head does not track,
                    // ignored or not, for the merge removes only what is tracked.
                    return Ok(self.holds_untracked(head, &prefix).await?.then_some(prefix));
                }
            } else {
                return self.unless_tracked(head, prefix).await;
            }
        }
        Ok(None)
    }

    /// Whether the worktree directory `directory` holds a file or a link that `head` does not track, whether git
    /// ignores it or not.
    async fn holds_untracked(&self, head: &str, directory: &[u8]) -> Result<bool, GitError> {
        let operation = GitOperation::LsTree;
        let mut under = directory.to_vec();
        under.push(b'/');
        let listed = self
            .run_ok(
                operation,
                Reach::Local,
                &[
                    arg("ls-tree"),
                    arg("-r"),
                    arg("-z"),
                    arg("--name-only"),
                    arg(head),
                    arg("--"),
                    OsStr::from_bytes(&under),
                ],
            )
            .await?;
        let tracked: BTreeSet<&[u8]> = whole(operation, &listed.stdout, self.limits)?
            .split(|byte| *byte == 0)
            .filter(|path| !path.is_empty())
            .collect();
        let root = self.dir.as_path();
        let mut pending = vec![directory.to_vec()];
        while let Some(current) = pending.pop() {
            let mut entries = match tokio::fs::read_dir(root.join(OsStr::from_bytes(&current))).await {
                Ok(entries) => entries,
                Err(source) if source.kind() == std::io::ErrorKind::NotFound => continue,
                Err(source) => {
                    return Err(GitError::Scratch {
                        source: ScratchError::Make(source),
                    });
                }
            };
            loop {
                let entry = match entries.next_entry().await {
                    Ok(Some(entry)) => entry,
                    Ok(None) => break,
                    Err(source) => {
                        return Err(GitError::Scratch {
                            source: ScratchError::Make(source),
                        });
                    }
                };
                let mut path = current.clone();
                path.push(b'/');
                path.extend_from_slice(entry.file_name().as_bytes());
                let kind = entry.file_type().await.map_err(|source| GitError::Scratch {
                    source: ScratchError::Make(source),
                })?;
                if kind.is_dir() {
                    pending.push(path);
                } else if !tracked.contains(path.as_slice()) {
                    return Ok(true);
                }
            }
        }
        Ok(false)
    }

    /// `path` as an obstacle, unless the head tracks it: then the merge removes or replaces it.
    async fn unless_tracked(&self, head: &str, path: Vec<u8>) -> Result<Option<Vec<u8>>, GitError> {
        let operation = GitOperation::LsTree;
        let mut spec = OsString::from(head);
        spec.push(":");
        spec.push(OsStr::from_bytes(&path));
        let tracked = self
            .run(operation, Reach::Local, &[arg("cat-file"), arg("-e"), &spec])
            .await?;
        Ok((!tracked.exit.succeeded()).then_some(path))
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
    env_name(INDEX_VARIABLE)
}

/// The name of an environment variable of a git command.
fn env_name(name: &'static str) -> Result<EnvName, GitError> {
    EnvName::from_str(name).map_err(|source| GitError::Scratch {
        source: ScratchError::Name { name, source },
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

#[cfg(test)]
mod tests;
