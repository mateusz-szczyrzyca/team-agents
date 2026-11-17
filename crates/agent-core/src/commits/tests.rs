use super::*;

/// Repository used by the pure linker examples.
const REPO_URL: &str = "https://github.com/owner/repo";

// Helper to track which hashes were resolved
fn resolve_hash(hash: &str) -> Option<String> {
    match hash {
        "a1b2c3d" => Some("a1b2c3d000000000000000000000000000000000".to_string()),
        "1234567" => Some("1234567000000000000000000000000000000000".to_string()),
        _ => None,
    }
}

#[test]
fn bare_hash_is_linked() {
    let input = "see commit a1b2c3d";
    let result = link_commits(input, REPO_URL, resolve_hash);
    assert_eq!(
        result,
        "see commit [`a1b2c3d`](https://github.com/owner/repo/commit/a1b2c3d000000000000000000000000000000000)"
    );
}

#[test]
fn backtick_enclosed_hash_is_linked() {
    let input = "see commit `a1b2c3d`";
    let result = link_commits(input, REPO_URL, resolve_hash);
    assert_eq!(
        result,
        "see commit [`a1b2c3d`](https://github.com/owner/repo/commit/a1b2c3d000000000000000000000000000000000)"
    );
}

#[test]
fn too_short_hash_unchanged() {
    let input = "see commit a1b2c3";
    let result = link_commits(input, REPO_URL, resolve_hash);
    assert_eq!(result, input);
}

#[test]
fn too_long_hash_unchanged() {
    let input = "see commit a1b2c3d000000000000000000000000000000000e5f6a1b2c3d4";
    let result = link_commits(input, REPO_URL, resolve_hash);
    assert_eq!(result, input);
}

#[test]
fn unresolvable_hash_unchanged() {
    let input = "see commit deadbeef";
    let result = link_commits(input, REPO_URL, resolve_hash);
    assert_eq!(result, input);
}

#[test]
fn non_hex_sequence_unchanged() {
    let input = "see commit abcdefgh";
    let result = link_commits(input, REPO_URL, resolve_hash);
    assert_eq!(result, input);
}

#[test]
fn multiple_commits_linked() {
    let input = "fixed in a1b2c3d, see also 1234567";
    let result = link_commits(input, REPO_URL, resolve_hash);
    assert_eq!(
        result,
        "fixed in [`a1b2c3d`](https://github.com/owner/repo/commit/a1b2c3d000000000000000000000000000000000), \
         see also [`1234567`](https://github.com/owner/repo/commit/1234567000000000000000000000000000000000)"
    );
}

#[test]
fn hash_in_backticks_replaced() {
    let input = "fixed in `a1b2c3d`";
    let result = link_commits(input, REPO_URL, resolve_hash);
    assert_eq!(
        result,
        "fixed in [`a1b2c3d`](https://github.com/owner/repo/commit/a1b2c3d000000000000000000000000000000000)"
    );
}

#[test]
fn hash_in_code_fence_unchanged() {
    let input = "```\na1b2c3d\n```";
    let result = link_commits(input, REPO_URL, resolve_hash);
    assert_eq!(result, input);
}

#[test]
fn hash_in_tilde_fence_unchanged() {
    let input = "~~~\na1b2c3d\n~~~";
    let result = link_commits(input, REPO_URL, resolve_hash);
    assert_eq!(result, input);
}

#[test]
fn hash_outside_fence_linked_mixed() {
    let input = "fixed a1b2c3d\n```\na1b2c3d\n```\nalso 1234567";
    let result = link_commits(input, REPO_URL, resolve_hash);
    assert_eq!(
        result,
        "fixed [`a1b2c3d`](https://github.com/owner/repo/commit/a1b2c3d000000000000000000000000000000000)\n\
         ```\na1b2c3d\n```\nalso [`1234567`](https://github.com/owner/repo/commit/1234567000000000000000000000000000000000)"
    );
}

#[test]
fn existing_link_not_relinked() {
    let input = "[`a1b2c3d`](https://github.com/owner/repo/commit/a1b2c3d000000000000000000000000000000000)";
    let result = link_commits(input, REPO_URL, resolve_hash);
    assert_eq!(result, input);
}

#[test]
fn hash_at_boundaries() {
    // 7 chars - minimum
    let input = "a1b2c3d";
    let result = link_commits(input, REPO_URL, resolve_hash);
    assert_eq!(
        result,
        "[`a1b2c3d`](https://github.com/owner/repo/commit/a1b2c3d000000000000000000000000000000000)"
    );

    // 40 chars - maximum (all zeros, unresolvable)
    let input40 = "0000000000000000000000000000000000000000";
    let result40 = link_commits(input40, REPO_URL, |_| None);
    assert_eq!(result40, input40);
}

#[test]
fn hash_with_surrounding_punctuation() {
    let input = "commit: a1b2c3d.";
    let result = link_commits(input, REPO_URL, resolve_hash);
    assert_eq!(
        result,
        "commit: [`a1b2c3d`](https://github.com/owner/repo/commit/a1b2c3d000000000000000000000000000000000)."
    );
}
