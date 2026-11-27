//! The check of a commit that is about to be made: would it change a test that a `RED:` commit froze (TDD-14)?
//!
//! The checks before a review find such a commit after it is made, and by then it may be published and nothing but a
//! rewrite of history could remove it. This is the same rule asked earlier, while the commit is only staged: a commit
//! that takes lines out of a test file that a `RED:` commit of the round touched is made as a `TEST-FIX:` commit that
//! changes test files only, or not at all. [`frozen_tests`] says which files are frozen and [`judge_commit`] decides on
//! the staged change; the daemon's checks stay the second look, where a commit slipped through (`--no-verify`).

use crate::config::Checks;
use crate::decide::checks::{Change, CommitFacts, FileChange, Lines, RED_PREFIX, TEST_FIX_PREFIX};
use crate::ids::ItemId;
use crate::plan::{Plan, Step};
use crate::repo_path::RepoPath;
use crate::state::CommitId;

/// A test file that a `RED:` commit of the round touched.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrozenTest {
    /// The file.
    pub path: RepoPath,
    /// The first `RED:` commit of the round that touched it.
    pub red: CommitId,
}

/// The test files that the `RED:` commits among `commits` touched, each once, with the first such commit, in the order
/// the files first appear.
#[must_use]
pub fn frozen_tests(rules: &Checks, commits: &[CommitFacts]) -> Vec<FrozenTest> {
    let mut frozen: Vec<FrozenTest> = Vec::new();
    for red in commits.iter().filter(|commit| commit.subject.starts_with(RED_PREFIX)) {
        for file in red.files.iter().filter(|file| rules.test_paths.matches(&file.path)) {
            if !frozen.iter().any(|known| known.path == file.path) {
                frozen.push(FrozenTest {
                    path: file.path.clone(),
                    red: red.id.clone(),
                });
            }
        }
    }
    frozen
}

/// The commit that is about to be made: its subject and what it changes against its parent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PendingCommit<'a> {
    /// The first line of its message.
    pub subject: &'a str,
    /// The files it changes against its parent; a rename is a deletion and an addition.
    pub files: &'a [FileChange],
}

/// What a commit takes out of a frozen test.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Taken {
    /// Some lines of a text file: the lines removed or replaced.
    Lines(u32),
    /// The whole file: it is deleted, or it is binary and replaced.
    WholeFile,
}

/// Why a commit is refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    /// The commit takes lines out of a frozen test and its subject does not start with `TEST-FIX:`.
    FrozenTestEdited {
        /// The frozen test.
        path: RepoPath,
        /// The `RED:` commit that froze it.
        red: CommitId,
        /// What the commit takes out of it.
        taken: Taken,
    },
    /// The commit is a `TEST-FIX:` that takes lines out of a frozen test and changes files that are not tests.
    TestFixTouchesOtherFiles {
        /// The files that are not tests, in the order of the change.
        others: Vec<RepoPath>,
    },
}

/// What is to happen to a commit that is about to be made.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    /// It is made.
    Allowed,
    /// It is not made, for these reasons.
    Refused(Vec<Refusal>),
}

/// Whether `commit` may be made, given the test files `frozen` by the round's `RED:` commits.
#[must_use]
pub fn judge_commit(rules: &Checks, frozen: &[FrozenTest], commit: &PendingCommit<'_>) -> Verdict {
    let edited: Vec<Refusal> = commit
        .files
        .iter()
        .filter(|file| file.takes_lines_out())
        .filter_map(|file| {
            let test = frozen.iter().find(|test| test.path == file.path)?;
            Some(Refusal::FrozenTestEdited {
                path: test.path.clone(),
                red: test.red.clone(),
                taken: taken_by(file),
            })
        })
        .collect();
    if edited.is_empty() {
        return Verdict::Allowed;
    }
    if !commit.subject.starts_with(TEST_FIX_PREFIX) {
        return Verdict::Refused(edited);
    }
    let others: Vec<RepoPath> = commit
        .files
        .iter()
        .filter(|file| !rules.test_paths.matches(&file.path))
        .map(|file| file.path.clone())
        .collect();
    if others.is_empty() {
        Verdict::Allowed
    } else {
        Verdict::Refused(vec![Refusal::TestFixTouchesOtherFiles { others }])
    }
}

/// Why a commit of a task whose plan ties its steps to requirements is refused for its `Step:` trailer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StepRefusal {
    /// The message has no `Step:` trailer.
    Missing {
        /// The steps of the plan, in the order written: what the trailer may name.
        steps: Vec<ItemId>,
    },
    /// The message has `Step:` trailers and some of them name no step of the plan.
    NotInPlan {
        /// What those trailers name, as written, in the order of the message.
        named: Vec<String>,
        /// The steps of the plan, in the order written.
        steps: Vec<ItemId>,
    },
}

/// What is to happen to a commit that is about to be made, as far as its `Step:` trailer goes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StepVerdict {
    /// It is made.
    Allowed,
    /// It is not made.
    Refused(StepRefusal),
}

/// Whether the commit with the message `message` may be made, given the `plan` of the task: when the plan has `For:`
/// lines, the message ends in a `Step: N.M` trailer that names a step of the plan.
#[must_use]
pub fn judge_step(plan: &Plan, message: &str) -> StepVerdict {
    if !plan.ties_requirements() {
        return StepVerdict::Allowed;
    }
    let steps = || plan.steps().map(Step::id).collect::<Vec<_>>();
    let named: Vec<&str> = trailer_block(message).filter_map(step_trailer).collect();
    if named.is_empty() {
        return StepVerdict::Refused(StepRefusal::Missing { steps: steps() });
    }
    let unknown: Vec<String> = named
        .into_iter()
        .filter(|text| !text.parse::<ItemId>().is_ok_and(|id| plan.step(&id).is_some()))
        .map(str::to_owned)
        .collect();
    if unknown.is_empty() {
        StepVerdict::Allowed
    } else {
        StepVerdict::Refused(StepRefusal::NotInPlan {
            named: unknown,
            steps: steps(),
        })
    }
}

/// The lines of the trailer block of `message`: its last paragraph, after the blank lines and the comment lines of the
/// editor at its end, and above the scissors line of `git commit -v`. A paragraph that holds a line which is no trailer
/// (and no folded line of one) is body, so a `Step:` line in it is not in the block.
fn trailer_block(message: &str) -> impl Iterator<Item = &str> {
    let mut lines: Vec<&str> = message.lines().collect();
    let comment = cut_at_scissors(&mut lines);
    let mut block: Vec<&str> = lines
        .into_iter()
        .rev()
        .skip_while(|line| line.trim().is_empty() || line.starts_with(comment))
        .take_while(|line| !line.trim().is_empty())
        .collect();
    block.reverse();
    if !block.iter().enumerate().all(|(index, line)| is_trailer_line(line, index)) {
        block.clear();
    }
    block.into_iter()
}

/// Removes from `lines` the scissors line of `git commit -v` and everything under it, and returns the comment
/// character that line starts with: the character of the editor's comment lines. Without such a line `lines` is whole
/// and the character is `#`.
fn cut_at_scissors(lines: &mut Vec<&str>) -> char {
    let found = lines.iter().enumerate().find_map(|(index, line)| {
        let mut chars = line.chars();
        let comment = chars.next()?;
        (chars.as_str() == SCISSORS).then_some((index, comment))
    });
    let Some((index, comment)) = found else {
        return DEFAULT_COMMENT;
    };
    lines.truncate(index);
    comment
}

/// Whether `line`, the `index`th of a trailer block, is a trailer (`Token: value`) or, after the first, the folded
/// continuation of one that starts with white space.
fn is_trailer_line(line: &str, index: usize) -> bool {
    if index > 0 && line.starts_with(char::is_whitespace) {
        return true;
    }
    line.split_once(':').is_some_and(|(token, _)| {
        !token.is_empty() && token.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
    })
}

/// The comment character of git's message file when the configuration does not say another.
const DEFAULT_COMMENT: char = '#';

/// What follows the comment character in the line that `git commit -v` puts above the diff it shows.
const SCISSORS: &str = " ------------------------ >8 ------------------------";

/// What a trailer line names: the text after `Step:` when `line` starts with it. A comment of the editor and a line
/// that only mentions the word are no trailer.
fn step_trailer(line: &str) -> Option<&str> {
    line.strip_prefix(STEP_TRAILER).map(str::trim)
}

/// What starts the trailer of a commit message that names the step of the plan the commit belongs to.
const STEP_TRAILER: &str = "Step:";

/// What `file`, which takes lines out of its file, takes out of it.
const fn taken_by(file: &FileChange) -> Taken {
    match (&file.change, file.lines) {
        (Change::Deleted, _) | (_, Lines::Binary) => Taken::WholeFile,
        (_, Lines::Text { removed, .. }) => Taken::Lines(removed),
    }
}

#[cfg(test)]
mod tests;
