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
        ("<https://a1b2c3d", "<https://a1b2c3d".to_owned()),
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

#[test]
fn bare_urls_and_html_remain_unchanged_alongside_linkable_prose() {
    let linked = "[`a1b2c3d`](https://github.com/owner/repo/commit/a1b2c3d000000000000000000000000000000000)";
    for protected in [
        "https://github.com/owner/repo/commit/a1b2c3d",
        "git+ssh://example.com/a1b2c3d?ref=a1b2c3d",
        "[ref]: https://example.com/a1b2c3d",
        "<a href=\"https://example.com/a1b2c3d\">",
        "<span title=\"a1b2c3d > a1b2c3d\">",
        "<span title='a1b2c3d > a1b2c3d'>",
        "<!-- a1b2c3d >\na1b2c3d -->",
        "</span data-ref=a1b2c3d>",
    ] {
        let text = format!("{protected} a1b2c3d");
        assert_eq!(
            link_commits(&text, REPO_URL, resolve_hash),
            format!("{protected} {linked}")
        );
    }
}

#[test]
fn backtick_in_fence_info_keeps_following_commit_in_prose() {
    let text = "```foo`bar\na1b2c3d";
    let expected =
        "```foo`bar\n[`a1b2c3d`](https://github.com/owner/repo/commit/a1b2c3d000000000000000000000000000000000)";
    assert_eq!(link_commits(text, REPO_URL, resolve_hash), expected);
}

#[test]
fn brackets_without_link_targets_preserve_protected_markdown() {
    let linked = "[`a1b2c3d`](https://github.com/owner/repo/commit/a1b2c3d000000000000000000000000000000000)";
    for (input, expected) in [
        (
            "the range [0, 5) then `git show a1b2c3d` and https://example.com/a1b2c3d a1b2c3d",
            format!("the range [0, 5) then `git show a1b2c3d` and https://example.com/a1b2c3d {linked}"),
        ),
        (
            "[see `git show a1b2c3d`] now a1b2c3d",
            format!("[see `git show a1b2c3d`] now {linked}"),
        ),
        ("[see ``a1b2c3d``] a1b2c3d", format!("[see ``a1b2c3d``] {linked}")),
        (
            "[see https://example.com/a1b2c3d ] a1b2c3d",
            format!("[see https://example.com/a1b2c3d ] {linked}"),
        ),
        (
            "[see <span title='a1b2c3d'>] a1b2c3d",
            format!("[see <span title='a1b2c3d'>] {linked}"),
        ),
        (
            "[see <!-- a1b2c3d --> a1b2c3d",
            format!("[see <!-- a1b2c3d --> {linked}"),
        ),
        (
            "[é [see `git show a1b2c3d`]] a1b2c3d",
            format!("[é [see `git show a1b2c3d`]] {linked}"),
        ),
        ("[a1b2c3d] a1b2c3d", format!("[{linked}] {linked}")),
        ("[see `a1b2c3d`]", format!("[see {linked}]")),
        (
            "[see](unclosed `git show a1b2c3d` a1b2c3d",
            format!("[see](unclosed `git show a1b2c3d` {linked}"),
        ),
    ] {
        assert_eq!(link_commits(input, REPO_URL, resolve_hash), expected, "{input:?}");
    }
}

#[test]
fn complete_links_preserve_email_titles_and_shortcut_references() {
    let linked = "[`a1b2c3d`](https://github.com/owner/repo/commit/a1b2c3d000000000000000000000000000000000)";
    for protected in [
        "<a1b2c3d@example.com>",
        "<person@a1b2c3d.example.com>",
        "[a1b2c3d](target \"title ) a1b2c3d\")",
        "[a1b2c3d](target 'title ) a1b2c3d')",
        "[a1b2c3d]\n\n[a1b2c3d]: https://example.com",
        "[A1B2C3D]\n\n[a1b2c3d]: https://example.com",
        "[a1b2c3d][]\n\n[a1b2c3d]: https://example.com",
    ] {
        let input = format!("{protected}\n\na1b2c3d");
        assert_eq!(
            link_commits(&input, REPO_URL, resolve_hash),
            format!("{protected}\n\n{linked}"),
            "{input:?}"
        );
    }
}

#[test]
fn container_fences_protect_commits_while_following_prose_is_linked() {
    let linked = "[`a1b2c3d`](https://github.com/owner/repo/commit/a1b2c3d000000000000000000000000000000000)";
    for protected in [
        "> ```rust\n> a1b2c3d\n> ```",
        "> ~~~text\n> a1b2c3d\n> ~~~",
        " > > ```rust\n > > a1b2c3d\n > > ```",
        "- ```rust\n  a1b2c3d\n  ```",
        "1. ~~~text\n   a1b2c3d\n   ~~~",
    ] {
        assert_eq!(
            link_commits(&format!("{protected}\n\na1b2c3d"), REPO_URL, resolve_hash),
            format!("{protected}\n\n{linked}"),
            "{protected:?}"
        );
    }
}

#[test]
fn reference_definitions_inside_fences_do_not_create_shortcut_links() {
    let input = "~~~text\n[a1b2c3d]: https://example.com\n~~~\n\n[a1b2c3d]";
    assert_eq!(
        link_commits(input, REPO_URL, resolve_hash),
        "~~~text\n[a1b2c3d]: https://example.com\n~~~\n\n[[`a1b2c3d`](https://github.com/owner/repo/commit/a1b2c3d000000000000000000000000000000000)]"
    );
}

#[test]
fn complete_links_take_precedence_over_shortcut_references() {
    for link in ["[a1b2c3d](path/a1b2c3d)", "[a1b2c3d][other-a1b2c3d]", "[a1b2c3d][]"] {
        let text = format!("{link}\n\n[a1b2c3d]: https://example.com");
        assert_eq!(link_commits(&text, REPO_URL, resolve_hash), text);
    }
}

#[test]
fn reference_definitions_preserve_relative_destinations_and_titles() {
    let linked = "[`a1b2c3d`](https://github.com/owner/repo/commit/a1b2c3d000000000000000000000000000000000)";
    for definition in [
        "[ref]: path/a1b2c3d",
        r#"[ref]: path/a1b2c3d "title a1b2c3d""#,
        "[ref]: <path/a1b2c3d> 'title a1b2c3d'",
        "   [ref]: path/a1b2c3d (title a1b2c3d)",
        r#"[ref]: path/a1b2c3d "escaped \" a1b2c3d""#,
    ] {
        let input = format!("[ref]\n\n{definition}\n\na1b2c3d");
        assert_eq!(
            link_commits(&input, REPO_URL, resolve_hash),
            format!("[ref]\n\n{definition}\n\n{linked}")
        );
    }
}

#[test]
fn fences_keep_the_opening_container_context_and_list_indentation() {
    let linked = "[`a1b2c3d`](https://github.com/owner/repo/commit/a1b2c3d000000000000000000000000000000000)";
    for (code, prose) in [
        ("~~~text\n> ~~~\na1b2c3d\n~~~\n", "a1b2c3d"),
        ("```text\n- ```\na1b2c3d\n```\n", "a1b2c3d"),
        ("- item\n\n    ~~~text\n    a1b2c3d\n    ~~~\n\n", "a1b2c3d"),
        ("1. item\n\n    ```rust\n    a1b2c3d\n    ```\n\n", "a1b2c3d"),
        ("> ~~~text\n> a1b2c3d\n", "a1b2c3d"),
        ("- ~~~text\n  a1b2c3d\n", "a1b2c3d"),
        ("- > ~~~text\n  > a1b2c3d\n", "a1b2c3d"),
        ("> - item\n>\n>     ~~~text\n>     a1b2c3d\n>     ~~~\n", "a1b2c3d"),
    ] {
        assert_eq!(
            link_commits(&format!("{code}{prose}"), REPO_URL, resolve_hash),
            format!("{code}{linked}"),
            "{code:?}"
        );
    }
}

#[test]
fn unindented_fences_after_terminated_containers_preserve_later_body_lines() {
    for container in ["1. finding", "- finding", "> finding"] {
        for fence in ["```", "~~~"] {
            let input = format!("{container}\n\n{fence}rust\nfirst body line\na1b2c3d\n{fence}\n");
            assert_eq!(
                link_commits(&input, REPO_URL, resolve_hash),
                input,
                "{container:?}, {fence:?}"
            );
        }
    }
}

#[test]
fn container_reference_definitions_and_their_global_shortcuts_are_preserved() {
    let linked = "[`1234567`](https://github.com/owner/repo/commit/1234567000000000000000000000000000000000)";
    for definition in [
        "> [a1b2c3d]: /url",
        "- [a1b2c3d]: path/1234567 \"title 1234567\"",
        "1. [a1b2c3d]: /url",
        "> - [a1b2c3d]: /url",
        "- item\n\n  [a1b2c3d]: /url",
        "> item\n>\n> [a1b2c3d]: /url",
    ] {
        let input = format!("[a1b2c3d]\n\n{definition}\n\n[a1b2c3d]\n\n1234567");
        assert_eq!(
            link_commits(&input, REPO_URL, resolve_hash),
            format!("[a1b2c3d]\n\n{definition}\n\n[a1b2c3d]\n\n{linked}"),
            "{definition:?}"
        );
    }
}
