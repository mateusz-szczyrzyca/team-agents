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

#[test]
fn markdown_boundaries_and_unicode_are_preserved() {
    let linked = "[`a1b2c3d`](https://github.com/owner/repo/commit/a1b2c3d000000000000000000000000000000000)";
    let cases = [
        (
            "é [a1b2c3d](https://example.com) a1b2c3d",
            format!("é [a1b2c3d](https://example.com) {linked}"),
        ),
        (
            "é\n```rust\na1b2c3d\n```\na1b2c3d",
            format!("é\n```rust\na1b2c3d\n```\n{linked}"),
        ),
        ("```rust\na1b2c3d", "```rust\na1b2c3d".to_owned()),
        (
            "````rust\n```\na1b2c3d\n````\na1b2c3d",
            format!("````rust\n```\na1b2c3d\n````\n{linked}"),
        ),
        (
            "~~~rust\n```\na1b2c3d\n~~~\na1b2c3d",
            format!("~~~rust\n```\na1b2c3d\n~~~\n{linked}"),
        ),
        (
            "```rust\r\na1b2c3d\r\n```\r\na1b2c3d",
            format!("```rust\r\na1b2c3d\r\n```\r\n{linked}"),
        ),
        (
            "[a1b2c3d](https://example.com/(a1b2c3d)) a1b2c3d",
            format!("[a1b2c3d](https://example.com/(a1b2c3d)) {linked}"),
        ),
        ("[label][a1b2c3d] a1b2c3d", format!("[label][a1b2c3d] {linked}")),
        (
            "za1b2c3d a1b2c3dz _a1b2c3d a1b2c3d_ éa1b2c3d a1b2c3dé",
            "za1b2c3d a1b2c3dz _a1b2c3d a1b2c3d_ éa1b2c3d a1b2c3dé".to_owned(),
        ),
        ("plain text without commits", "plain text without commits".to_owned()),
    ];
    for (input, expected) in cases {
        assert_eq!(link_commits(input, REPO_URL, resolve_hash), expected, "{input:?}");
    }
}

#[test]
fn full_commit_ids_keep_their_original_spelling() {
    let id = "ABCDEF0123456789abcdef0123456789ABCDEF01";
    assert_eq!(id.len(), 40);
    for input in [id.to_owned(), format!("`{id}`")] {
        assert_eq!(
            link_commits(&input, REPO_URL, |candidate| Some(candidate.to_ascii_lowercase())),
            format!("[`{id}`]({REPO_URL}/commit/{})", id.to_ascii_lowercase())
        );
    }
}

#[test]
fn unresolved_text_is_preserved_for_markdown_delimiter_combinations() {
    let fragments = ["", "é", "[", "](", ")", "\\", "`", "~~~\n", "```\n", "a1b2c3d", "\r\n"];
    for before in fragments {
        for middle in fragments {
            for after in fragments {
                let text = format!("{before}{middle}{after}");
                assert_eq!(link_commits(&text, REPO_URL, |_| None), text);
            }
        }
    }
}

#[test]
fn existing_autolinks_are_preserved_alongside_linkable_prose() {
    let input = "é <https://example.com/a1b2c3d> a1b2c3d";
    let expected = "é <https://example.com/a1b2c3d> [`a1b2c3d`](https://github.com/owner/repo/commit/a1b2c3d000000000000000000000000000000000)";
    assert_eq!(link_commits(input, REPO_URL, resolve_hash), expected);
}

#[test]
fn autolink_schemes_and_unclosed_angles_keep_markdown_meaning() {
    let linked = "[`a1b2c3d`](https://github.com/owner/repo/commit/a1b2c3d000000000000000000000000000000000)";
    for scheme in ["http", "https", "mailto", "git+ssh", "CUSTOM.2"] {
        let text = format!("<{scheme}:a1b2c3d>");
        assert_eq!(link_commits(&text, REPO_URL, resolve_hash), text);
    }
    for (input, expected) in [
        ("<a1b2c3d>", format!("<{linked}>")),
        ("<https://a1b2c3d", format!("<https://{linked}")),
        ("<https:// a1b2c3d", format!("<https:// {linked}")),
    ] {
        assert_eq!(link_commits(input, REPO_URL, resolve_hash), expected);
    }
}

#[test]
fn inline_code_spans_protect_commands_and_matching_backtick_runs() {
    let linked = "[`a1b2c3d`](https://github.com/owner/repo/commit/a1b2c3d000000000000000000000000000000000)";
    for span in [
        "`git show a1b2c3d`",
        "``a1b2c3d``",
        "``git `show` a1b2c3d``",
        "`git\nshow a1b2c3d`",
        "`[a1b2c3d](target)`",
        "`é a1b2c3d`",
    ] {
        let text = format!("run {span} then a1b2c3d");
        assert_eq!(
            link_commits(&text, REPO_URL, resolve_hash),
            format!("run {span} then {linked}")
        );
    }
}
