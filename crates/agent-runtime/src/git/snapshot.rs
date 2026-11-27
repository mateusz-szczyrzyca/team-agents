//! A snapshot of a worktree: everything in it, kept as a commit under a reference of its round and task, without changing
//! the worktree, its index or the files. The snapshot is the part of a run's work that nobody else can see when the run is
//! cut; taking it is read-only for the checkout.

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::{OsStr, OsString};
use std::os::unix::ffi::OsStrExt;
use std::path::Path;
use std::str::FromStr;

use agent_core::config::EnvName;
use agent_core::ids::{RoundId, TaskId};
use agent_core::state::CommitId;

use super::changes::{close_scratch, make_scratch};
use super::{
    ExitReport, Git, GitError, GitOperation, Reach, Revision, ScratchError, arg, diagnostic, failed, text, whole,
};

/// The namespace of the snapshot references: `<prefix>/<round>/<task>/<n>`, and `<prefix>/<round>/<task>/c<n>` for a
/// checkpoint's snapshot.
const SNAPSHOT_REFS: &str = "refs/agents/snapshots";

/// What a checkpoint's reference has before its number, so that its numbers do not mix with those of the snapshots a
/// restart or a briefing takes: those are numbered `1`, `2`, ... and the rest of the harness reads them by that.
const CHECKPOINT_MARK: char = 'c';

/// Who asks for a snapshot; each kind has the numbers of its own under one task.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SnapshotKind {
    /// A snapshot of a cut run, for a briefing: `<n>`.
    Briefing,
    /// A snapshot a checkpoint keeps: `c<n>`.
    Checkpoint,
}

impl SnapshotKind {
    /// The leaf of a reference of this kind with the number `number`.
    fn leaf(self, number: u32) -> String {
        match self {
            Self::Briefing => number.to_string(),
            Self::Checkpoint => format!("{CHECKPOINT_MARK}{number}"),
        }
    }

    /// The kind and the number a reference's last component tells; `None` when it tells no number.
    fn parse_leaf(leaf: &str) -> Option<Result<(Self, u32), std::num::ParseIntError>> {
        if leaf.is_empty() {
            return None;
        }
        Some(match leaf.strip_prefix(CHECKPOINT_MARK) {
            Some(number) => number.parse().map(|number| (Self::Checkpoint, number)),
            None => leaf.parse().map(|number| (Self::Briefing, number)),
        })
    }
}

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

/// Whether a snapshot was written for a request or an earlier one stood in for it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SnapshotOrigin {
    /// A new commit and reference were written.
    Taken,
    /// The worktree's tree and head were those of the earlier snapshot: its reference is kept as it is.
    Reused,
}

/// A snapshot taken for a checkpoint by [`Git::snapshot_unless_unchanged`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckpointSnapshot {
    /// The snapshot, new or the earlier one that stands in for it.
    pub snapshot: Snapshot,
    /// The reference that keeps the snapshot's commit: `refs/agents/snapshots/<round>/<task>/c<n>`.
    pub reference: String,
    /// Whether the snapshot is new.
    pub origin: SnapshotOrigin,
}

/// What [`Git::write_snapshot`] leaves.
struct Written {
    /// The commit that holds the tree.
    commit: CommitId,
    /// The bytes of the files of the tree.
    bytes: u64,
    /// The number of the earlier snapshot that was reused; `None` when a new commit was written.
    reused_number: Option<u32>,
}

impl Git {
    /// Keeps the worktree like [`Git::snapshot`], unless the snapshot under `previous` already holds the same tree on the
    /// same head: then that snapshot is the answer and nothing is written.
    ///
    /// # Errors
    /// The errors of [`Git::snapshot`].
    pub async fn snapshot_unless_unchanged(
        &self,
        round: &RoundId,
        task: &TaskId,
        previous: Option<&str>,
    ) -> Result<CheckpointSnapshot, GitError> {
        let (snapshot, origin) = self
            .snapshot_after(round, task, SnapshotKind::Checkpoint, previous)
            .await?;
        let reference = format!(
            "{SNAPSHOT_REFS}/{round}/{task}/{}",
            SnapshotKind::Checkpoint.leaf(snapshot.number)
        );
        Ok(CheckpointSnapshot {
            snapshot,
            reference,
            origin,
        })
    }

    /// Removes the snapshot references `references`; one that is already gone is no error.
    ///
    /// # Errors
    /// [`GitError`] when git cannot delete a reference.
    pub async fn delete_snapshot_references(&self, references: &[String]) -> Result<(), GitError> {
        for reference in references {
            // `update-ref -d` on a reference that does not exist succeeds without doing anything.
            self.run_ok(
                GitOperation::UpdateRef,
                Reach::Local,
                &[arg("update-ref"), arg("-d"), arg(reference)],
            )
            .await?;
        }
        Ok(())
    }

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
        let (snapshot, _origin) = self.snapshot_after(round, task, SnapshotKind::Briefing, None).await?;
        Ok(snapshot)
    }

    /// Takes the snapshot of [`Git::snapshot`], or answers with the one under `previous` when it holds the same tree on
    /// the same head.
    async fn snapshot_after(
        &self,
        round: &RoundId,
        task: &TaskId,
        kind: SnapshotKind,
        previous: Option<&str>,
    ) -> Result<(Snapshot, SnapshotOrigin), GitError> {
        let head = self.rev_parse(&Revision::Head).await?;
        let number = self.next_snapshot_number(task, kind).await?;
        let scratch = make_scratch().await?;
        let written = self
            .write_snapshot(scratch.path(), &head, round, task, (kind, number), previous)
            .await;
        close_scratch(scratch).await;
        let written = written?;
        let (number, origin) = match written.reused_number {
            Some(earlier) => (earlier, SnapshotOrigin::Reused),
            None => (number, SnapshotOrigin::Taken),
        };
        let snapshot = Snapshot {
            number,
            commit: written.commit,
            head,
            bytes: written.bytes,
        };
        Ok((snapshot, origin))
    }

    /// The number of the next snapshot of `kind` of `task`: one more than the highest number any reference of that kind of
    /// `task` under [`SNAPSHOT_REFS`] already has, across all rounds.
    async fn next_snapshot_number(&self, task: &TaskId, kind: SnapshotKind) -> Result<u32, GitError> {
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
            let Some(parsed) = SnapshotKind::parse_leaf(number_text) else {
                continue;
            };
            let (ref_kind, parsed) = parsed.map_err(|source| GitError::NotANumber {
                operation,
                text: number_text.to_owned(),
                source,
            })?;
            if ref_kind == kind {
                highest = highest.max(parsed);
            }
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
        (kind, number): (SnapshotKind, u32),
        previous: Option<&str>,
    ) -> Result<Written, GitError> {
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
        if let Some(previous) = previous
            && let Some((commit, earlier)) = self
                .reusable_snapshot(previous, kind, (round, task), &tree, head)
                .await?
        {
            let bytes = self.tree_bytes(&tree).await?;
            return Ok(Written {
                commit,
                bytes,
                reused_number: Some(earlier),
            });
        }
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
        let reference = format!("{SNAPSHOT_REFS}/{round}/{task}/{}", kind.leaf(number));
        let commit_str = commit.to_string();
        self.run_scratch(
            GitOperation::UpdateRef,
            &overrides,
            &[arg("update-ref"), arg(&reference), arg(&commit_str), arg("")],
        )
        .await?;
        let bytes = self.tree_bytes(&tree).await?;
        Ok(Written {
            commit,
            bytes,
            reused_number: None,
        })
    }

    /// The commit and the number of the snapshot under `reference` when it holds `tree` on `head`; `None` when the
    /// reference is not a snapshot reference of `kind` of this `owner` (round and task), is gone, or holds something else.
    /// A reference of another round or task is never reused: the new snapshot would be recorded under a reference that
    /// was never written.
    async fn reusable_snapshot(
        &self,
        reference: &str,
        kind: SnapshotKind,
        owner: (&RoundId, &TaskId),
        tree: &str,
        head: &CommitId,
    ) -> Result<Option<(CommitId, u32)>, GitError> {
        let (round, task) = owner;
        let Some(number) = reference
            .strip_prefix(&format!("{SNAPSHOT_REFS}/{round}/{task}/"))
            .and_then(SnapshotKind::parse_leaf)
            .and_then(Result::ok)
            .filter(|(found, _)| *found == kind)
            .map(|(_, number)| number)
        else {
            return Ok(None);
        };
        let Some(commit) = self.resolve_quietly(&format!("{reference}^{{commit}}")).await? else {
            return Ok(None);
        };
        let held_tree = self.resolve_quietly(&format!("{reference}^{{tree}}")).await?;
        let held_head = self.resolve_quietly(&format!("{reference}^1")).await?;
        if held_tree.as_deref() != Some(tree) || held_head.as_deref() != Some(head.to_string().as_str()) {
            return Ok(None);
        }
        let commit: CommitId = commit.parse().map_err(|source| GitError::NotACommit {
            operation: GitOperation::RevParse,
            source,
        })?;
        Ok(Some((commit, number)))
    }

    /// The object `spec` names, or `None` when it names nothing.
    async fn resolve_quietly(&self, spec: &str) -> Result<Option<String>, GitError> {
        let operation = GitOperation::RevParse;
        let captured = self
            .run(
                operation,
                Reach::Local,
                &[
                    arg("rev-parse"),
                    arg("--verify"),
                    arg("--quiet"),
                    arg("--end-of-options"),
                    arg(spec),
                ],
            )
            .await?;
        match captured.exit {
            ExitReport::Exited { code: 0 } => Ok(Some(text(operation, &captured.stdout)?.trim_end().to_owned())),
            // With `--verify --quiet`, 1 is git's answer for a name that names nothing.
            ExitReport::Exited { code: 1 } => Ok(None),
            ExitReport::Exited { .. } | ExitReport::Signaled { .. } => Err(failed(operation, &captured)),
        }
    }

    /// The regular files (`100644` and `100755`) in the scratch index, mapped from path to `(mode, oid)`.
    async fn staged_regular_files(
        &self,
        overrides: &[OsString],
    ) -> Result<BTreeMap<OsString, (String, String)>, GitError> {
        let operation = GitOperation::LsFiles;
        let captured = self
            .run_scratch(operation, overrides, &[arg("ls-files"), arg("-z"), arg("-s")])
            .await?;
        let listed = whole(operation, &captured.stdout, self.limits)?;
        let mut files = BTreeMap::new();
        for entry in listed.split(|byte| *byte == 0).filter(|entry| !entry.is_empty()) {
            let Some(tab_pos) = entry.iter().position(|byte| *byte == b'\t') else {
                return Err(GitError::Unexpected {
                    operation,
                    output: diagnostic(entry),
                });
            };
            let (meta_bytes, rest) = entry.split_at(tab_pos);
            let Some((_, path_bytes)) = rest.split_first() else {
                return Err(GitError::Unexpected {
                    operation,
                    output: diagnostic(entry),
                });
            };
            let meta =
                String::from_utf8(meta_bytes.to_vec()).map_err(|source| GitError::NotUtf8 { operation, source })?;
            let mut parts = meta.split_whitespace();
            let (Some(mode), Some(oid)) = (parts.next(), parts.next()) else {
                return Err(GitError::Unexpected {
                    operation,
                    output: diagnostic(entry),
                });
            };
            if mode == "100644" || mode == "100755" {
                files.insert(
                    OsStr::from_bytes(path_bytes).to_owned(),
                    (mode.to_owned(), oid.to_owned()),
                );
            }
        }
        Ok(files)
    }

    /// The worktree paths that are modified or untracked (excluding ignored files) relative to the scratch index.
    async fn worktree_dirty_paths(&self, overrides: &[OsString]) -> Result<BTreeSet<OsString>, GitError> {
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
            paths.insert(OsStr::from_bytes(entry).to_owned());
        }
        Ok(paths)
    }

    /// Replaces any regular file blob in the scratch index that `git add` altered through built-in `.gitattributes` rules
    /// (`text`, `eol`, `crlf`, `ident`, `working-tree-encoding`) with the exact bytes of the worktree file via
    /// `git hash-object -w --no-filters`.
    async fn store_verbatim_blobs(
        &self,
        overrides: &[OsString],
        head_entries: &BTreeMap<OsString, (String, String)>,
        dirty_paths: &BTreeSet<OsString>,
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
                        path.as_os_str(),
                    ],
                )
                .await?;
            if raw_oid != oid {
                let mut cacheinfo = OsString::from(format!("{mode},{raw_oid},"));
                cacheinfo.push(&path);
                self.run_scratch(
                    GitOperation::UpdateIndex,
                    overrides,
                    &[arg("update-index"), arg("--cacheinfo"), cacheinfo.as_os_str()],
                )
                .await?;
            }
        }
        Ok(())
    }

    /// What a command of the scratch index prints, without its last line break.
    pub(super) async fn scratch_text(
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
