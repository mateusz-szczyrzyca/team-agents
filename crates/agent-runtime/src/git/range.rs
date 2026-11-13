//! What git says about a range of commits: where it starts, which files it changed, the commits in it and the lines it
//! added. The checks before a review read these and decide with no model (`agent_core::decide::checks`).
//!
//! Every list of files is read from a NUL-separated form of git's output, so that a path with any character but NUL is
//! read as it is; a path that is not valid text, or not a path of a repository, is an error. The messages of the
//! commits and the added lines are read as text with the bytes that are not UTF-8 replaced: only ASCII words and
//! markers are looked for in them.

use std::collections::HashMap;

use agent_core::decide::checks::{
    AddedLine, Change, CommitFacts, FileChange, Lines, ParentChanges, RoundFacts, markers,
};
use agent_core::repo_path::RepoPath;
use agent_core::state::CommitId;

use super::{Git, GitError, GitOperation, Reach, Revision, arg, failed, text, whole};
use crate::process::ExitReport;

/// The most bytes of the diff of one file that are read: a generated file of added lines may be long.
const MAX_PATCH_BYTES: u64 = 8 << 20;

/// What a commit message line's NUL-terminated record in `git log` has: the ID, the parents, the subject and the body.
const LOG_FIELDS: usize = 4;

impl Git {
    /// The best common ancestor of two revisions: where a branch left its base.
    ///
    /// # Errors
    /// [`GitError::NoMergeBase`] when they have no commit in common; [`GitError::Failed`] when a revision names no
    /// commit; [`GitError::Run`], [`GitError::OutputTooLong`], [`GitError::NotUtf8`] and [`GitError::NotACommit`].
    pub async fn merge_base(&self, first: &Revision, second: &Revision) -> Result<CommitId, GitError> {
        let operation = GitOperation::MergeBase;
        let (first, second) = (first.to_string(), second.to_string());
        let captured = self
            .run(operation, Reach::Local, &[arg("merge-base"), arg(&first), arg(&second)])
            .await?;
        match captured.exit {
            ExitReport::Exited { code: 0 } => {}
            // Without `--is-ancestor`, 1 is git's answer for histories that share no commit.
            ExitReport::Exited { code: 1 } => return Err(GitError::NoMergeBase { first, second }),
            ExitReport::Exited { .. } | ExitReport::Signaled { .. } => return Err(failed(operation, &captured)),
        }
        text(operation, &captured.stdout)?
            .trim_end()
            .parse()
            .map_err(|source| GitError::NotACommit { operation, source })
    }

    /// The files that differ between `base` and `head`, as git prints them: a rename is one entry, with the lines it
    /// changed.
    ///
    /// # Errors
    /// [`GitError::Failed`] when a commit is not in the repository; [`GitError::Unexpected`], [`GitError::NotUtf8`]
    /// and [`GitError::NotARepoPath`] for output that is not what git prints; [`GitError::Run`] and
    /// [`GitError::OutputTooLong`].
    pub async fn range_changes(&self, base: &CommitId, head: &CommitId) -> Result<Vec<FileChange>, GitError> {
        let operation = GitOperation::Diff;
        let (base, head) = (base.to_string(), head.to_string());
        let mut listed = Vec::new();
        for format in ["--name-status", "--numstat"] {
            let args = [
                arg("diff"),
                arg(format),
                arg("-z"),
                arg("-M"),
                arg("--no-ext-diff"),
                arg("--no-textconv"),
                arg(&base),
                arg(&head),
                arg("--"),
            ];
            let captured = self.run_ok(operation, Reach::Local, &args).await?;
            listed.push(whole(operation, &captured.stdout, self.limits)?.to_vec());
        }
        match listed.as_slice() {
            [names, counts] => join_changes(
                operation,
                parse_name_status(operation, names)?,
                parse_numstat(operation, counts)?,
            ),
            _ => Err(GitError::Unexpected {
                operation,
                output: String::new(),
            }),
        }
    }

    /// The commits after `base` up to `head`, oldest first (a commit after its parents), each with its parents, its
    /// message and the files it changed against its first parent. Merges retain separate diffs against every parent
    /// in `merge_parents`; their `files` list stays empty. A rename is a deletion and an addition.
    ///
    /// # Errors
    /// [`GitError::TooManyCommits`] when the range has more than [`GitLimits::max_commits`](super::GitLimits); the
    /// errors of [`Git::range_changes`].
    pub async fn range_commits(&self, base: &CommitId, head: &CommitId) -> Result<Vec<CommitFacts>, GitError> {
        let operation = GitOperation::Log;
        let range = format!("{base}..{head}");
        let one_more = self.limits.max_commits.saturating_add(1).to_string();
        let captured = self
            .run_ok(
                operation,
                Reach::Local,
                &[
                    arg("log"),
                    arg("--topo-order"),
                    arg("--reverse"),
                    arg("--max-count"),
                    arg(&one_more),
                    arg("--format=%H%x00%P%x00%s%x00%b%x00"),
                    arg(&range),
                    arg("--"),
                ],
            )
            .await?;
        let mut commits = parse_log(operation, whole(operation, &captured.stdout, self.limits)?)?;
        if commits.len() > self.limits.max_commits {
            return Err(GitError::TooManyCommits {
                limit: self.limits.max_commits,
            });
        }
        for commit in &mut commits {
            if commit.parents.len() > 1 {
                for parent in &commit.parents {
                    commit.merge_parents.push(ParentChanges {
                        parent: parent.clone(),
                        files: self.commit_files(&commit.id, Some(parent)).await?,
                    });
                }
            } else {
                commit.files = self.commit_files(&commit.id, None).await?;
            }
        }
        Ok(commits)
    }

    /// The lines added between `base` and `head` to each of `paths`, with their numbers in the file at `head`. A file
    /// that is binary, deleted or unchanged has none.
    ///
    /// # Errors
    /// The errors of [`Git::range_changes`].
    pub async fn range_added_lines(
        &self,
        base: &CommitId,
        head: &CommitId,
        paths: &[RepoPath],
    ) -> Result<Vec<AddedLine>, GitError> {
        let mut added = Vec::new();
        for path in paths {
            added.extend(self.added_in(base, head, path, None).await?);
        }
        Ok(added)
    }

    /// The facts the checks before a review decide from: the files, the commits, the added lines of the files whose
    /// extension has markers, and `description`, the pull request's.
    ///
    /// # Errors
    /// The errors of [`Git::range_commits`] and [`Git::range_added_lines`].
    pub async fn round_facts(
        &self,
        base: &CommitId,
        head: &CommitId,
        description: &str,
    ) -> Result<RoundFacts, GitError> {
        let files = self.range_changes(base, head).await?;
        let commits = self.range_commits(base, head).await?;
        let mut added = Vec::new();
        for file in &files {
            let scanned = file.path.extension().is_some_and(markers::scans) && file.puts_lines_in();
            if scanned && file.change != Change::Deleted {
                let from = match &file.change {
                    Change::Renamed { from } => Some(from),
                    Change::Added | Change::Modified | Change::Deleted => None,
                };
                added.extend(self.added_in(base, head, &file.path, from).await?);
            }
        }
        Ok(RoundFacts {
            files,
            commits,
            added,
            description: description.to_owned(),
        })
    }

    /// The files `commit` changed against `parent`, or its sole parent when none is supplied. A root compares with
    /// the empty tree. Explicit merge parents preserve changes hidden by git's default merge-diff behavior.
    async fn commit_files(&self, commit: &CommitId, parent: Option<&CommitId>) -> Result<Vec<FileChange>, GitError> {
        let operation = GitOperation::DiffTree;
        let mut args = vec![
            arg("diff-tree"),
            arg("-r"),
            arg("-z"),
            arg("--no-renames"),
            arg("--root"),
            arg("--no-commit-id"),
            arg("--no-ext-diff"),
            arg("--no-textconv"),
            arg("--raw"),
            arg("--numstat"),
        ];
        args.extend(parent.map(|id| arg(id.as_str())));
        args.extend([arg(commit.as_str()), arg("--")]);
        let captured = self.run_ok(operation, Reach::Local, &args).await?;
        parse_tree_diff(operation, whole(operation, &captured.stdout, self.limits)?)
    }

    /// The lines added to the file `path` between `base` and `head`; `from` is where the file was when it was renamed.
    async fn added_in(
        &self,
        base: &CommitId,
        head: &CommitId,
        path: &RepoPath,
        from: Option<&RepoPath>,
    ) -> Result<Vec<AddedLine>, GitError> {
        let operation = GitOperation::Diff;
        let (base, head) = (base.to_string(), head.to_string());
        let mut specs = vec![literal(path)];
        specs.extend(from.map(literal));
        let mut args = vec![
            arg("diff"),
            arg("-U0"),
            arg("-M"),
            arg("--no-color"),
            arg("--no-ext-diff"),
            arg("--no-textconv"),
            arg(&base),
            arg(&head),
            arg("--"),
        ];
        args.extend(specs.iter().map(|spec| arg(spec)));
        let mut patching = self.clone();
        patching.limits.max_output_bytes = patching.limits.max_output_bytes.max(MAX_PATCH_BYTES);
        let captured = patching.run_ok(operation, Reach::Local, &args).await?;
        parse_patch(operation, path, whole(operation, &captured.stdout, patching.limits)?)
    }
}

/// The pathspec that names exactly `path`: no wildcard of it is a pattern.
fn literal(path: &RepoPath) -> String {
    format!(":(literal){path}")
}

/// A path git printed, as a [`RepoPath`].
fn repo_path(operation: GitOperation, bytes: &[u8]) -> Result<RepoPath, GitError> {
    RepoPath::try_from(utf8(operation, bytes)?).map_err(|source| GitError::NotARepoPath { operation, source })
}

/// What git printed as text.
fn utf8(operation: GitOperation, bytes: &[u8]) -> Result<String, GitError> {
    String::from_utf8(bytes.to_vec()).map_err(|source| GitError::NotUtf8 { operation, source })
}

/// What git printed as a number.
fn number(operation: GitOperation, bytes: &[u8]) -> Result<u32, GitError> {
    let text = utf8(operation, bytes)?;
    text.parse().map_err(|source| GitError::NotANumber {
        operation,
        text,
        source,
    })
}

/// What git printed as a commit ID.
fn commit_id(operation: GitOperation, text: &str) -> Result<CommitId, GitError> {
    text.trim()
        .parse()
        .map_err(|source| GitError::NotACommit { operation, source })
}

/// The error for output that is not what git prints for the operation.
fn unexpected(operation: GitOperation, output: &[u8]) -> GitError {
    GitError::Unexpected {
        operation,
        output: String::from_utf8_lossy(output).chars().take(200).collect(),
    }
}

/// What a file entry of `git diff --name-status -z` says: how it changed and its path (where it is now).
type Named = (Change, RepoPath);

/// Reads the output of `git diff --name-status -z -M`: a status, then the path, or for a rename the old and the new.
fn parse_name_status(operation: GitOperation, bytes: &[u8]) -> Result<Vec<Named>, GitError> {
    let mut tokens = bytes.split(|byte| *byte == 0).filter(|token| !token.is_empty());
    let mut named = Vec::new();
    while let Some(status) = tokens.next() {
        let first = tokens.next().ok_or_else(|| unexpected(operation, status))?;
        let entry = match status.split_first() {
            Some((b'A', [])) => (Change::Added, repo_path(operation, first)?),
            Some((b'M' | b'T', [])) => (Change::Modified, repo_path(operation, first)?),
            Some((b'D', [])) => (Change::Deleted, repo_path(operation, first)?),
            Some((b'R', score)) if score.iter().all(u8::is_ascii_digit) => {
                let second = tokens.next().ok_or_else(|| unexpected(operation, status))?;
                (
                    Change::Renamed {
                        from: repo_path(operation, first)?,
                    },
                    repo_path(operation, second)?,
                )
            }
            _ => return Err(unexpected(operation, status)),
        };
        named.push(entry);
    }
    Ok(named)
}

/// The lines of one entry of a `--numstat`: a number for each of added and removed, or `-` for a binary file.
fn parse_lines(operation: GitOperation, added: &[u8], removed: &[u8]) -> Result<Lines, GitError> {
    if added == b"-" && removed == b"-" {
        return Ok(Lines::Binary);
    }
    Ok(Lines::Text {
        added: number(operation, added)?,
        removed: number(operation, removed)?,
    })
}

/// Reads the output of `git diff --numstat -z -M`: `added TAB removed TAB path`, or for a rename an empty path and then
/// the old and the new. The lines of each file are keyed by the path it has now.
fn parse_numstat(operation: GitOperation, bytes: &[u8]) -> Result<HashMap<RepoPath, Lines>, GitError> {
    let mut tokens = bytes.split(|byte| *byte == 0).filter(|token| !token.is_empty());
    let mut counted = HashMap::new();
    while let Some(entry) = tokens.next() {
        let mut parts = entry.splitn(3, |byte| *byte == b'\t');
        let (Some(added), Some(removed), Some(path)) = (parts.next(), parts.next(), parts.next()) else {
            return Err(unexpected(operation, entry));
        };
        let lines = parse_lines(operation, added, removed)?;
        let path = if path.is_empty() {
            // A rename: the old path, then the new.
            tokens.next().ok_or_else(|| unexpected(operation, entry))?;
            tokens.next().ok_or_else(|| unexpected(operation, entry))?
        } else {
            path
        };
        counted.insert(repo_path(operation, path)?, lines);
    }
    Ok(counted)
}

/// Puts the lines of each file next to how it changed.
fn join_changes(
    operation: GitOperation,
    named: Vec<Named>,
    mut counted: HashMap<RepoPath, Lines>,
) -> Result<Vec<FileChange>, GitError> {
    named
        .into_iter()
        .map(|(change, path)| match counted.remove(&path) {
            Some(lines) => Ok(FileChange { path, change, lines }),
            None => Err(unexpected(operation, path.as_str().as_bytes())),
        })
        .collect()
}

/// Reads the output of `git log` with the format `%H %x00 %P %x00 %s %x00 %b %x00`: four fields of each commit, each
/// ended by a NUL, and a line break after the commit. The files are not in it yet.
fn parse_log(operation: GitOperation, bytes: &[u8]) -> Result<Vec<CommitFacts>, GitError> {
    let mut fields: Vec<&[u8]> = bytes.split(|byte| *byte == 0).collect();
    // What follows the last NUL is the line break git ends the last commit with.
    match fields.pop() {
        Some(rest) if rest.iter().all(u8::is_ascii_whitespace) => {}
        Some(rest) => return Err(unexpected(operation, rest)),
        None => return Ok(Vec::new()),
    }
    let mut commits = Vec::new();
    for record in fields.chunks(LOG_FIELDS) {
        let &[id, parents, subject, body] = record else {
            return Err(unexpected(operation, &record.concat()));
        };
        commits.push(CommitFacts {
            id: commit_id(operation, &utf8(operation, id)?)?,
            parents: utf8(operation, parents)?
                .split_whitespace()
                .map(|parent| commit_id(operation, parent))
                .collect::<Result<_, _>>()?,
            subject: String::from_utf8_lossy(subject).trim().to_owned(),
            body: String::from_utf8_lossy(body).trim_end().to_owned(),
            files: Vec::new(),
            merge_parents: Vec::new(),
        });
    }
    Ok(commits)
}

/// Reads the output of `git diff-tree -r -z --no-renames --raw --numstat`: the raw entries (`:modes ids status`, then the
/// path), then the numstat entries.
fn parse_tree_diff(operation: GitOperation, bytes: &[u8]) -> Result<Vec<FileChange>, GitError> {
    let mut tokens = bytes
        .split(|byte| *byte == 0)
        .filter(|token| !token.is_empty())
        .peekable();
    let mut named = Vec::new();
    while let Some(raw) = tokens.next_if(|token| token.starts_with(b":")) {
        let status = raw
            .rsplit(|byte| *byte == b' ')
            .next()
            .ok_or_else(|| unexpected(operation, raw))?;
        let path = repo_path(operation, tokens.next().ok_or_else(|| unexpected(operation, raw))?)?;
        let change = match status {
            b"A" => Change::Added,
            b"M" | b"T" => Change::Modified,
            b"D" => Change::Deleted,
            _ => return Err(unexpected(operation, raw)),
        };
        named.push((change, path));
    }
    let rest: Vec<u8> = tokens
        .flat_map(|token| token.iter().copied().chain(std::iter::once(0)))
        .collect();
    join_changes(operation, named, parse_numstat(operation, &rest)?)
}

/// Reads a diff made with no lines of context: the lines that each hunk adds, from the number its header gives.
fn parse_patch(operation: GitOperation, path: &RepoPath, bytes: &[u8]) -> Result<Vec<AddedLine>, GitError> {
    let mut added = Vec::new();
    // Lines of the hunk that are still to be read, and the number of the next added line.
    let (mut removing, mut adding, mut number) = (0_u32, 0_u32, 0_u32);
    for line in bytes.split(|byte| *byte == b'\n') {
        if removing == 0 && adding == 0 {
            if let Some(header) = line.strip_prefix(b"@@ -") {
                (removing, number, adding) = parse_hunk_header(operation, header)?;
            }
            continue;
        }
        match line.split_first() {
            Some((b'-', _)) if removing > 0 => removing = removing.saturating_sub(1),
            Some((b'+', text)) if adding > 0 => {
                let text = text.strip_suffix(b"\r").unwrap_or(text);
                added.push(AddedLine {
                    path: path.clone(),
                    number,
                    text: String::from_utf8_lossy(text).into_owned(),
                });
                number = number.saturating_add(1);
                adding = adding.saturating_sub(1);
            }
            // The note that the last line of a file has no line break belongs to the line before it.
            Some((b'\\', _)) => {}
            _ => return Err(unexpected(operation, line)),
        }
    }
    Ok(added)
}

/// Reads what follows `@@ -` in a hunk header: `old[,count] +new[,count] @@`, as the lines the hunk removes, the number of
/// its first new line and the lines it adds.
fn parse_hunk_header(operation: GitOperation, header: &[u8]) -> Result<(u32, u32, u32), GitError> {
    let text = utf8(operation, header)?;
    let wrong = || unexpected(operation, header);
    let (ranges, _) = text.split_once(" @@").ok_or_else(wrong)?;
    let (old, new) = ranges.split_once(" +").ok_or_else(wrong)?;
    // A range with no count is one line.
    let range = |text: &str| -> Result<(u32, u32), GitError> {
        let (start, count) = text.split_once(',').unwrap_or((text, "1"));
        Ok((
            number(operation, start.as_bytes())?,
            number(operation, count.as_bytes())?,
        ))
    };
    let (_, removed) = range(old)?;
    let (start, added) = range(new)?;
    Ok((removed, start, added))
}
