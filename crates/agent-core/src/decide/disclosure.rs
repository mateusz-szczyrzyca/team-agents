//! What the harness itself writes into the description of a pull request about the commits of the round: the list of
//! the `TEST-FIX:` commits, under the heading the checks and the reviewers look for.
//!
//! A test is frozen from its `RED:` commit on; a commit that corrects one is a `TEST-FIX:` commit, and the pull request
//! says so under `## Tests changed after RED`. The agents may not edit a pull request, so the harness writes that
//! section from the commits: [`test_fix_section`] makes it, and [`with_section`] puts it at the end of a description, in
//! the place of the one the harness wrote before. Two comment lines mark the section as the harness's, so that it is
//! replaced as a whole and nothing a person wrote is touched.

use super::checks::{CommitFacts, TEST_FIX_PREFIX};

/// The line before the section the harness writes.
const OPENING: &str = "<!-- harness: tests changed after RED -->";

/// The line after the section the harness writes.
const CLOSING: &str = "<!-- /harness: tests changed after RED -->";

/// The heading of the section: the one the checks before a review and the reviewers look for.
const HEADING: &str = "## Tests changed after RED";

/// What the section says before its list.
const INTRODUCTION: &str = "The harness lists here every commit of the branch whose subject starts with `TEST-FIX:`, \
                            with what its message says.";

/// The section that lists the `TEST-FIX:` commits among `commits`, oldest first as they are given, each with its ID,
/// its subject and its message as a quotation; `None` when there is none.
#[must_use]
pub fn test_fix_section(commits: &[CommitFacts]) -> Option<String> {
    let fixes: Vec<&CommitFacts> = commits
        .iter()
        .filter(|commit| commit.subject.starts_with(TEST_FIX_PREFIX))
        .collect();
    if fixes.is_empty() {
        return None;
    }
    let mut lines = vec![
        OPENING.to_owned(),
        HEADING.to_owned(),
        String::new(),
        INTRODUCTION.to_owned(),
        String::new(),
    ];
    for commit in fixes {
        lines.push(format!("- {} — {}", commit.id, commit.subject));
        // A quotation: whatever the message holds, a heading or a comment line in it is none of the description.
        for line in commit.body.trim().lines() {
            let line = line.trim_end();
            lines.push(if line.is_empty() {
                "  >".to_owned()
            } else {
                format!("  > {line}")
            });
        }
    }
    lines.push(CLOSING.to_owned());
    Some(lines.join("\n"))
}

/// `description` with `section` at its end, in the place of a section the harness wrote before; without one when
/// `section` is `None`. Everything else of the description stays as it is.
#[must_use]
pub fn with_section(description: &str, section: Option<&str>) -> String {
    let (kept, had) = without_section(description);
    match section {
        // Nothing to take out and nothing to put in: the description is not the harness's to tidy.
        None if !had => description.to_owned(),
        None => kept,
        Some(section) => {
            let before = kept.trim_end();
            if before.is_empty() {
                format!("{section}\n")
            } else {
                format!("{before}\n\n{section}\n")
            }
        }
    }
}

/// `description` without the section between the harness's two comment lines, and whether it had one. What stood
/// before the section keeps one line break at its end; a section that was opened and never closed runs to the end.
fn without_section(description: &str) -> (String, bool) {
    let mut kept: Vec<&str> = Vec::new();
    let (mut inside, mut had) = (false, false);
    for line in description.lines() {
        if inside {
            inside = line.trim() != CLOSING;
        } else if line.trim() == OPENING {
            inside = true;
            had = true;
        } else {
            kept.push(line);
        }
    }
    if !had {
        return (description.to_owned(), false);
    }
    let text = kept.join("\n");
    let text = text.trim_end();
    if text.is_empty() {
        (String::new(), true)
    } else {
        (format!("{text}\n"), true)
    }
}

#[cfg(test)]
mod tests;
