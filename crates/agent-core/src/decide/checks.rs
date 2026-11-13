//! The checks before a review: the facts of a round's diff in, the issues out.
//!
//! The daemon reads the facts from git — what changed from the merge base with the base branch to the head, the commits
//! on the way, the lines they added and the pull request's description — and [`assess`] turns them into [`Issue`]s with
//! no model and no I/O. What is a test, what is production code and what judges a round comes from the repository's
//! [`Checks`] table; nothing here names a repository or its layout.
//!
//! The rules, each of them one function of the facts:
//!
//! - **An existing test file changed other than by added lines**, removed or renamed ([`Issue::TestChanged`]): only the
//!   human may allow it.
//! - **A protected file changed** ([`Issue::ProtectedChanged`]): only the human may allow it.
//! - **A test file that a `RED:` commit touched has lines removed or replaced by a later commit**
//!   ([`Issue::RedTestEdited`], TDD-14), unless that commit touches test files only, its subject starts with
//!   `TEST-FIX:` and the pull request's description names it under `## Tests changed after RED`.
//! - **The round changes production code and adds no line to a test file** ([`Issue::NoTests`], TDD-1), unless a commit
//!   of the round says `Test-First: not possible — <reason>`.
//! - **An added line that skips a test, suppresses a lint or asks whether the code runs under test**
//!   ([`Issue::Marker`], TDD-11, LINT-3, TDD-12), in a file the description does not name; the markers are a built-in
//!   table per file extension ([`markers`]).
//!
//! A test module inside a source file is not a test file: it stays with the reviewers.
//!
//! The issues of the first two rules are [`Judge::Human`]'s: the team cannot allow them. The others are the team's to
//! fix, and the human's to allow when the team cannot.

use std::collections::{BTreeMap, HashSet};

use serde::{Deserialize, Serialize};

use crate::config::Checks;
use crate::repo_path::RepoPath;
use crate::state::CommitId;

pub mod markers;

/// What a subject starts with when its commit is the first half of a test-first cycle.
const RED_PREFIX: &str = "RED:";

/// What a subject starts with when its commit corrects a test that a `RED:` commit committed (TDD-14).
const TEST_FIX_PREFIX: &str = "TEST-FIX:";

/// The heading of the section of a pull request's description that names the `TEST-FIX:` commits, in lowercase.
const TEST_FIX_HEADING: &str = "## tests changed after red";

/// What a line of a commit message says when the change cannot have its test first; the reason follows after a dash.
const TEST_FIRST_NOT_POSSIBLE: &str = "Test-First: not possible";

/// The dashes that may stand between [`TEST_FIRST_NOT_POSSIBLE`] and its reason.
const REASON_DASHES: [char; 3] = ['\u{2014}', '\u{2013}', '-'];

/// The fewest digits of a commit ID that name the commit in a description: git's own shortest abbreviation.
const MIN_ABBREVIATION: usize = 7;

/// How a file differs between two commits.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Change {
    /// The file is new.
    Added,
    /// The file has other contents.
    Modified,
    /// The file is gone.
    Deleted,
    /// The file moved; it may have other contents too.
    Renamed {
        /// Where it was.
        from: RepoPath,
    },
}

/// How many lines a change added and took away.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lines {
    /// A text file.
    Text {
        /// Lines the change added.
        added: u32,
        /// Lines the change removed; a replaced line counts as one added and one removed.
        removed: u32,
    },
    /// A binary file: git counts no lines.
    Binary,
}

/// One changed file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileChange {
    /// The file's path after the change; for a deleted file, where it was.
    pub path: RepoPath,
    /// How it changed.
    pub change: Change,
    /// What the change did to its lines.
    pub lines: Lines,
}

/// One commit of the round with the files it changed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitFacts {
    /// The commit.
    pub id: CommitId,
    /// Its parents; none for a commit with no history.
    pub parents: Vec<CommitId>,
    /// The first line of its message.
    pub subject: String,
    /// The rest of its message.
    pub body: String,
    /// The files it changed against its first parent; none for a merge. A rename is a deletion and an addition here.
    pub files: Vec<FileChange>,
    /// For a merge, the files it changed against each of its parents, in the order of `parents`: what reached that
    /// parent's side through the merge, and what its resolution changed. Empty for a commit with one parent or none.
    pub merge_files: Vec<Vec<FileChange>>,
}

/// One line a round added to a file, for the markers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AddedLine {
    /// The file.
    pub path: RepoPath,
    /// The line's number in the file at the head, counted from 1.
    pub number: u32,
    /// The line's text, without its line break.
    pub text: String,
}

/// One line of a file at the head that the round did not change, near a line it added: the rest of an attribute that
/// rustfmt wrapped over lines, when only one of them is new.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnchangedLine {
    /// The file.
    pub path: RepoPath,
    /// The line's number in the file at the head, counted from 1.
    pub number: u32,
    /// The line's text, without its line break.
    pub text: String,
}

/// What the checks look at: the round's diff and its commits.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RoundFacts {
    /// The files that differ between the merge base and the head.
    pub files: Vec<FileChange>,
    /// The commits from the merge base to the head, oldest first: a commit comes after its parents.
    pub commits: Vec<CommitFacts>,
    /// The lines the round added to the files whose extension has markers ([`markers::scans`]).
    pub added: Vec<AddedLine>,
    /// The description of the pull request; empty when there is none yet.
    pub description: String,
    /// The files whose mode (an executable bit) differs between the merge base and the head, by their path at the head.
    pub mode_changed: Vec<RepoPath>,
}

/// What the checks found wrong with a round.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Issue {
    /// An existing test file was changed other than by added lines.
    TestChanged {
        /// The file as it was at the merge base.
        path: RepoPath,
        /// What was done to it.
        how: TestChange,
    },
    /// A protected file was changed.
    ProtectedChanged {
        /// The file; where it was, when it moved.
        path: RepoPath,
        /// What was done to it.
        change: Touch,
    },
    /// A commit removed or replaced lines of a test file that a `RED:` commit of the round touched.
    RedTestEdited {
        /// The test file.
        path: RepoPath,
        /// The `RED:` commit that touched it.
        red: CommitId,
        /// The later commit that took lines out of it.
        edit: CommitId,
    },
    /// The round changes production code and adds no line to a test file.
    NoTests,
    /// Added lines that skip a test, suppress a lint or ask whether the code runs under test.
    Marker {
        /// The file.
        path: RepoPath,
        /// Which kind of marker.
        marker: markers::Marker,
        /// The first line that has one.
        line: u32,
        /// How many lines of the file have one.
        count: u32,
    },
}

/// What was done to an existing test file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TestChange {
    /// It was deleted.
    Deleted,
    /// It was renamed.
    Renamed {
        /// Where it is now.
        to: RepoPath,
    },
    /// Lines were removed or replaced.
    Edited {
        /// How many lines were taken out.
        removed: u32,
    },
    /// A binary file was replaced.
    EditedBinary,
    /// Its mode (an executable bit) changed, and its lines are the same or only gained some.
    ModeChanged,
}

/// What was done to a protected file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, strum::Display)]
#[strum(serialize_all = "snake_case")]
pub enum Touch {
    /// It was added.
    Added,
    /// Its contents changed.
    Modified,
    /// It was deleted.
    Deleted,
    /// It was renamed.
    Renamed,
}

/// Who can allow an issue.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Judge {
    /// The team can: it fixes the round, or says why in the commits or the description.
    Team,
    /// Only the human can.
    Human,
}

/// What an issue is about: what the human allows or refuses, and what is recorded for the item when they allow it.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Subject {
    /// A file.
    Path(RepoPath),
    /// That the round has no tests for the code it changes.
    NoTests,
}

impl FileChange {
    /// Whether the change takes lines out of the file: removes or replaces some, deletes it or replaces a binary file.
    #[must_use]
    pub const fn takes_lines_out(&self) -> bool {
        match (&self.change, self.lines) {
            (Change::Deleted, _) | (_, Lines::Text { removed: 1.., .. }) => true,
            (Change::Added, Lines::Binary) | (_, Lines::Text { removed: 0, .. }) => false,
            (_, Lines::Binary) => true,
        }
    }

    /// Whether the change puts at least one line into the file.
    #[must_use]
    pub const fn puts_lines_in(&self) -> bool {
        matches!(self.lines, Lines::Text { added: 1.., .. })
    }
}

impl Issue {
    /// Who can allow the issue.
    #[must_use]
    pub const fn judge(&self) -> Judge {
        match self {
            Self::TestChanged { .. } | Self::ProtectedChanged { .. } => Judge::Human,
            Self::RedTestEdited { .. } | Self::NoTests | Self::Marker { .. } => Judge::Team,
        }
    }

    /// What the issue is about.
    #[must_use]
    pub fn subject(&self) -> Subject {
        match self {
            Self::TestChanged { path, .. }
            | Self::ProtectedChanged { path, .. }
            | Self::RedTestEdited { path, .. }
            | Self::Marker { path, .. } => Subject::Path(path.clone()),
            Self::NoTests => Subject::NoTests,
        }
    }
}

/// The issues the checks find in `facts`, under the lists of `rules`, without those about a [`Subject`] in `allowed`:
/// [`assess_near`] with no line of the files besides the added ones.
///
/// The order is fixed: protected files, changed tests, edited `RED:` tests, missing tests, markers; within a rule, the
/// order of the facts, and for the markers the file, then the kind.
#[must_use]
pub fn assess(rules: &Checks, facts: &RoundFacts, allowed: &[Subject]) -> Vec<Issue> {
    assess_near(rules, facts, &[], allowed)
}

/// The issues the checks find in `facts` and `unchanged`, the lines near the added ones that the round left as they
/// were, in the same files and in the order of their numbers; the rest as in [`assess`].
#[must_use]
pub fn assess_near(rules: &Checks, facts: &RoundFacts, unchanged: &[UnchangedLine], allowed: &[Subject]) -> Vec<Issue> {
    let mut issues = protected_changes(rules, facts);
    issues.extend(changed_tests(rules, facts));
    issues.extend(edited_red_tests(rules, facts));
    issues.extend(missing_tests(rules, facts));
    issues.extend(markers_added(rules, facts, unchanged));
    issues.retain(|issue| !allowed.contains(&issue.subject()));
    issues
}

/// The protected path `file` touches: where it was when it moved out of one, otherwise where it is.
fn protected_path<'a>(rules: &Checks, file: &'a FileChange) -> Option<&'a RepoPath> {
    match &file.change {
        Change::Renamed { from } if rules.protected_paths.matches(from) => Some(from),
        Change::Added | Change::Modified | Change::Deleted | Change::Renamed { .. } => {
            rules.protected_paths.matches(&file.path).then_some(&file.path)
        }
    }
}

/// The protected files the round changed.
fn protected_changes(rules: &Checks, facts: &RoundFacts) -> Vec<Issue> {
    facts
        .files
        .iter()
        .filter_map(|file| {
            let path = protected_path(rules, file)?;
            let change = match file.change {
                Change::Added => Touch::Added,
                Change::Modified => Touch::Modified,
                Change::Deleted => Touch::Deleted,
                Change::Renamed { .. } => Touch::Renamed,
            };
            Some(Issue::ProtectedChanged {
                path: path.clone(),
                change,
            })
        })
        .collect()
}

/// The existing test files the round changed other than by added lines; a protected file is the first rule's.
fn changed_tests(rules: &Checks, facts: &RoundFacts) -> Vec<Issue> {
    facts
        .files
        .iter()
        .filter(|file| protected_path(rules, file).is_none())
        .filter_map(|file| {
            let mode_changed = facts.mode_changed.contains(&file.path);
            let (existing, how) = match (&file.change, file.lines) {
                (Change::Added, _) => return None,
                (Change::Modified, Lines::Text { added: 1.., removed: 0 }) if !mode_changed => return None,
                (Change::Deleted, _) => (&file.path, TestChange::Deleted),
                (Change::Renamed { from }, _) => (from, TestChange::Renamed { to: file.path.clone() }),
                (Change::Modified, Lines::Binary) => (&file.path, TestChange::EditedBinary),
                // Lines only added, or none: what else changed is the mode. Git lists a file as modified with no line
                // changed when only its mode changed.
                (Change::Modified, Lines::Text { removed: 0, .. }) => (&file.path, TestChange::ModeChanged),
                (Change::Modified, Lines::Text { removed, .. }) => (&file.path, TestChange::Edited { removed }),
            };
            rules.test_paths.matches(existing).then(|| Issue::TestChanged {
                path: existing.clone(),
                how,
            })
        })
        .collect()
}

/// The test files a `RED:` commit touched that a later commit took lines out of, unless that commit is a `TEST-FIX:`
/// of test files that the description names.
fn edited_red_tests(rules: &Checks, facts: &RoundFacts) -> Vec<Issue> {
    let mut issues = Vec::new();
    let mut reported: HashSet<(&RepoPath, &CommitId)> = HashSet::new();
    for (position, red) in facts
        .commits
        .iter()
        .enumerate()
        .filter(|(_, commit)| commit.subject.starts_with(RED_PREFIX))
    {
        let tests: Vec<&RepoPath> = red
            .files
            .iter()
            .map(|file| &file.path)
            .filter(|path| rules.test_paths.matches(path))
            .collect();
        if tests.is_empty() {
            continue;
        }
        let later = later_commits(&facts.commits, position);
        let having_red: HashSet<&CommitId> = std::iter::once(&red.id)
            .chain(later.iter().map(|commit| &commit.id))
            .collect();
        for edit in later {
            if is_named_test_fix(rules, facts, edit) {
                continue;
            }
            for file in changes_after(edit, &having_red).filter(|file| file.takes_lines_out()) {
                if tests.contains(&&file.path) && reported.insert((&file.path, &edit.id)) {
                    issues.push(Issue::RedTestEdited {
                        path: file.path.clone(),
                        red: red.id.clone(),
                        edit: edit.id.clone(),
                    });
                }
            }
        }
    }
    issues
}

/// The files `commit` changed against the parents that have a commit of `having`: its own files, and those of a merge
/// against each such parent — against a parent without it, a merge's lines are those of the other side.
fn changes_after<'a>(commit: &'a CommitFacts, having: &HashSet<&CommitId>) -> impl Iterator<Item = &'a FileChange> {
    let through: Vec<&'a Vec<FileChange>> = commit
        .parents
        .iter()
        .zip(&commit.merge_files)
        .filter(|(parent, _)| having.contains(parent))
        .map(|(_, files)| files)
        .collect();
    commit.files.iter().chain(through.into_iter().flatten())
}

/// The commits that come after the one at `position` in its history, in the order of `commits`: those that have it or
/// another of them as a parent.
fn later_commits(commits: &[CommitFacts], position: usize) -> Vec<&CommitFacts> {
    let Some((first, after)) = commits.get(position..).and_then(<[CommitFacts]>::split_first) else {
        return Vec::new();
    };
    let mut ancestors: HashSet<&CommitId> = HashSet::from([&first.id]);
    let mut later = Vec::new();
    for commit in after {
        if commit.parents.iter().any(|parent| ancestors.contains(parent)) {
            ancestors.insert(&commit.id);
            later.push(commit);
        }
    }
    later
}

/// Whether `commit` is a `TEST-FIX:` that touches test files only and that the description names.
fn is_named_test_fix(rules: &Checks, facts: &RoundFacts, commit: &CommitFacts) -> bool {
    commit.subject.starts_with(TEST_FIX_PREFIX)
        && !commit.files.is_empty()
        && commit.files.iter().all(|file| rules.test_paths.matches(&file.path))
        && names_commit(&facts.description, &commit.id)
}

/// Whether the section of `description` that is headed `## Tests changed after RED` names `commit`, by its ID or by
/// an abbreviation of at least seven digits.
fn names_commit(description: &str, commit: &CommitId) -> bool {
    let mut inside = false;
    for line in description.lines() {
        let line = line.trim();
        if line.starts_with('#') {
            inside = line.trim_end_matches(':').eq_ignore_ascii_case(TEST_FIX_HEADING);
        } else if inside
            && line
                .split(|c: char| !c.is_ascii_alphanumeric())
                .any(|word| word.len() >= MIN_ABBREVIATION && commit.as_str().starts_with(&word.to_ascii_lowercase()))
        {
            return true;
        }
    }
    false
}

/// Whether `path` is production code: a path of the code list that is not a test.
fn is_production(rules: &Checks, path: &RepoPath) -> bool {
    rules.code_paths.matches(path) && !rules.test_paths.matches(path)
}

/// `NoTests` when the round changes production code, adds no line to a test file and has no commit that says why.
fn missing_tests(rules: &Checks, facts: &RoundFacts) -> Option<Issue> {
    let changes_code = facts.files.iter().any(|file| {
        is_production(rules, &file.path)
            || matches!(&file.change, Change::Renamed { from } if is_production(rules, from))
    });
    let adds_tests = facts
        .files
        .iter()
        .any(|file| rules.test_paths.matches(&file.path) && file.puts_lines_in());
    let excused = facts
        .commits
        .iter()
        .any(|commit| says_no_test_first(&commit.subject) || says_no_test_first(&commit.body));
    (changes_code && !adds_tests && !excused).then_some(Issue::NoTests)
}

/// Whether a line of `message` is `Test-First: not possible`, a dash and a reason.
fn says_no_test_first(message: &str) -> bool {
    message.lines().any(|line| {
        line.trim()
            .strip_prefix(TEST_FIRST_NOT_POSSIBLE)
            .and_then(|rest| rest.trim_start().strip_prefix(REASON_DASHES))
            .is_some_and(|reason| !reason.trim().is_empty())
    })
}

/// Whether `description` names the file `path` with its whole path: where it occurs, a delimiter or the edge of the text
/// comes right before it, and after it come a delimiter or the edge of the text, with dots, question marks and bangs of
/// a sentence between. Naming `crates/other/src/a.rs`, `src/a.rs.orig` or `src/a.rs@backup` does not name `src/a.rs`.
fn names_path(description: &str, path: &RepoPath) -> bool {
    let path = path.as_str();
    // `get` finds no text only off a character boundary, which `match_indices` never gives: such a place names nothing.
    description.match_indices(path).any(|(start, _)| {
        let starts = description
            .get(..start)
            .is_some_and(|before| before.chars().next_back().is_none_or(is_delimiter));
        let ends = description
            .get(start..)
            .and_then(|from| from.strip_prefix(path))
            .is_some_and(|after| {
                after
                    .trim_start_matches(SENTENCE_MARKS)
                    .chars()
                    .next()
                    .is_none_or(is_delimiter)
            });
        starts && ends
    })
}

/// The characters that end a sentence and may follow a path in it.
const SENTENCE_MARKS: [char; 3] = ['.', '?', '!'];

/// The ASCII punctuation that ends a path in a description: markdown's quotes, brackets, emphasis, anchors and cells, and
/// prose's commas, semicolons and colons.
const DELIMITERS: &str = "`\"'()[]{}<>,;:#=*|";

/// Whether `c` ends the text of a path in a description: white space, punctuation outside ASCII (`«…»`) or one of
/// [`DELIMITERS`]. Every other character may be part of a file name, so that a longer name is no mention of a shorter.
fn is_delimiter(c: char) -> bool {
    c.is_whitespace() || (!c.is_alphanumeric() && !c.is_ascii_punctuation()) || DELIMITERS.contains(c)
}

/// The added lines that have a marker — and the attributes wrapped over lines that have one and an added line, as far as
/// `unchanged` shows them —, one issue for each file and kind, in a file the description does not name.
fn markers_added(rules: &Checks, facts: &RoundFacts, unchanged: &[UnchangedLine]) -> Vec<Issue> {
    let mut found: BTreeMap<(&RepoPath, markers::Marker), (u32, u32)> = BTreeMap::new();
    // The lines of a file follow each other in the facts; a wrapped attribute is read over the lines of one file.
    for lines in facts.added.chunk_by(|one, next| one.path == next.path) {
        let Some(path) = lines.first().map(|line| &line.path) else {
            continue;
        };
        let Some(extension) = path.extension() else {
            continue;
        };
        if names_path(&facts.description, path) {
            continue;
        }
        let mut numbered: Vec<markers::Line<'_>> = lines
            .iter()
            .map(|line| markers::Line {
                number: line.number,
                text: line.text.as_str(),
                added: true,
            })
            .chain(
                unchanged
                    .iter()
                    .filter(|line| line.path == *path)
                    .map(|line| markers::Line {
                        number: line.number,
                        text: line.text.as_str(),
                        added: false,
                    }),
            )
            .collect();
        numbered.sort_by_key(|line| line.number);
        for (number, marker) in markers::markers_touching(extension, &numbered) {
            // A test may ask whether it runs under test; production code may not.
            if marker == markers::Marker::ChecksUnderTest && !is_production(rules, path) {
                continue;
            }
            let (first, count) = found.entry((path, marker)).or_insert((number, 0));
            *first = (*first).min(number);
            *count = count.saturating_add(1);
        }
    }
    found
        .into_iter()
        .map(|((path, marker), (line, count))| Issue::Marker {
            path: path.clone(),
            marker,
            line,
            count,
        })
        .collect()
}

#[cfg(test)]
mod tests;
