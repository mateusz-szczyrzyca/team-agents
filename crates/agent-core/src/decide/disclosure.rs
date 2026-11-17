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
    // The stub of the RED commit: it knows what it will write and writes nothing.
    let _ = (commits, OPENING, CLOSING, HEADING, INTRODUCTION, TEST_FIX_PREFIX);
    None
}

/// `description` with `section` at its end, in the place of a section the harness wrote before; without one when
/// `section` is `None`. Everything else of the description stays as it is.
#[must_use]
pub fn with_section(description: &str, section: Option<&str>) -> String {
    let _ = section;
    description.to_owned()
}

#[cfg(test)]
mod tests;
