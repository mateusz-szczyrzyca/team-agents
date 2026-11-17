use super::*;

const REPO_URL: &str = "https://github.com/owner/repo";
const COMMIT: &str = "1234567";

// Helper to track which paths resolve to files
fn resolve_path(path: &str) -> bool {
    matches!(
        path,
        "src/lib.rs" | "src/main.rs" | "Cargo.toml" | "README.md" | "crates/agent-core/src/files.rs"
    )
}

#[test]
fn bare_path_is_linked() {
    let input = "see src/lib.rs";
    let result = link_files(input, REPO_URL, COMMIT, resolve_path);
    assert_eq!(
        result,
        "see [src/lib.rs](https://github.com/owner/repo/blob/1234567/src/lib.rs)"
    );
}

#[test]
fn backtick_path_is_linked() {
    let input = "see `src/lib.rs`";
    let result = link_files(input, REPO_URL, COMMIT, resolve_path);
    assert_eq!(
        result,
        "see [`src/lib.rs`](https://github.com/owner/repo/blob/1234567/src/lib.rs)"
    );
}

#[test]
fn path_with_line_number_is_linked() {
    let input = "see `src/lib.rs:42`";
    let result = link_files(input, REPO_URL, COMMIT, resolve_path);
    assert_eq!(
        result,
        "see [`src/lib.rs:42`](https://github.com/owner/repo/blob/1234567/src/lib.rs#L42)"
    );
}

#[test]
fn path_with_line_range_is_linked() {
    let input = "see `src/lib.rs:10-20`";
    let result = link_files(input, REPO_URL, COMMIT, resolve_path);
    assert_eq!(
        result,
        "see [`src/lib.rs:10-20`](https://github.com/owner/repo/blob/1234567/src/lib.rs#L10-L20)"
    );
}

#[test]
fn path_with_special_chars_is_encoded() {
    let input = "file with spaces in `src/my file.rs`";
    let result = link_files(input, REPO_URL, COMMIT, |path| path == "src/my file.rs");
    assert_eq!(
        result,
        "file with spaces in [`src/my file.rs`](https://github.com/owner/repo/blob/1234567/src/my%20file.rs)"
    );
}

#[test]
fn directory_not_linked() {
    let input = "see src/";
    let result = link_files(input, REPO_URL, COMMIT, |_| false);
    assert_eq!(result, input);
}

#[test]
fn missing_file_not_linked() {
    let input = "see src/missing.rs";
    let result = link_files(input, REPO_URL, COMMIT, |_| false);
    assert_eq!(result, input);
}

#[test]
fn absolute_path_not_linked() {
    let input = "see /src/lib.rs";
    let result = link_files(input, REPO_URL, COMMIT, resolve_path);
    assert_eq!(result, input);
}

#[test]
fn path_with_double_dot_not_linked() {
    let input = "see ../src/lib.rs";
    let result = link_files(input, REPO_URL, COMMIT, resolve_path);
    assert_eq!(result, input);
}

#[test]
fn path_in_fenced_code_not_linked() {
    let input = "```\nsrc/lib.rs\n```";
    let result = link_files(input, REPO_URL, COMMIT, resolve_path);
    assert_eq!(result, input);
}

#[test]
fn path_in_existing_link_not_relinked() {
    let input = "[link](src/lib.rs)";
    let result = link_files(input, REPO_URL, COMMIT, resolve_path);
    assert_eq!(result, input);
}

#[test]
fn path_at_start_of_text_linked() {
    let input = "src/lib.rs is important";
    let result = link_files(input, REPO_URL, COMMIT, resolve_path);
    assert_eq!(
        result,
        "[src/lib.rs](https://github.com/owner/repo/blob/1234567/src/lib.rs) is important"
    );
}

#[test]
fn path_at_end_of_text_linked() {
    let input = "see src/lib.rs";
    let result = link_files(input, REPO_URL, COMMIT, resolve_path);
    assert_eq!(
        result,
        "see [src/lib.rs](https://github.com/owner/repo/blob/1234567/src/lib.rs)"
    );
}

#[test]
fn path_followed_by_period_linked() {
    let input = "see src/lib.rs.";
    let result = link_files(input, REPO_URL, COMMIT, resolve_path);
    assert_eq!(
        result,
        "see [src/lib.rs](https://github.com/owner/repo/blob/1234567/src/lib.rs)."
    );
}

#[test]
fn path_followed_by_comma_linked() {
    let input = "see src/lib.rs, also README.md";
    let result = link_files(input, REPO_URL, COMMIT, resolve_path);
    assert_eq!(
        result,
        "see [src/lib.rs](https://github.com/owner/repo/blob/1234567/src/lib.rs), also [README.md](https://github.com/owner/repo/blob/1234567/README.md)"
    );
}

#[test]
fn multiple_paths_linked() {
    let input = "see src/lib.rs and src/main.rs";
    let result = link_files(input, REPO_URL, COMMIT, resolve_path);
    assert_eq!(
        result,
        "see [src/lib.rs](https://github.com/owner/repo/blob/1234567/src/lib.rs) and [src/main.rs](https://github.com/owner/repo/blob/1234567/src/main.rs)"
    );
}

#[test]
fn text_without_paths_unchanged() {
    let input = "this is just some prose with no paths";
    let result = link_files(input, REPO_URL, COMMIT, resolve_path);
    assert_eq!(result, input);
}

#[test]
fn url_not_linked() {
    let input = "see https://github.com/owner/repo for details";
    let result = link_files(input, REPO_URL, COMMIT, resolve_path);
    assert_eq!(result, input);
}

#[test]
fn email_not_linked() {
    let input = "contact <user@example.com>";
    let result = link_files(input, REPO_URL, COMMIT, resolve_path);
    assert_eq!(result, input);
}

#[test]
fn path_in_longer_word_not_linked() {
    let input = "my_src/lib.rs_backup";
    let result = link_files(input, REPO_URL, COMMIT, resolve_path);
    assert_eq!(result, input);
}

#[test]
fn path_with_hash_in_backticks_encoded() {
    let input = "`src/lib#2.rs`";
    let result = link_files(input, REPO_URL, COMMIT, |path| path == "src/lib#2.rs");
    assert_eq!(
        result,
        "[`src/lib#2.rs`](https://github.com/owner/repo/blob/1234567/src/lib%232.rs)"
    );
}

#[test]
fn multiline_text_with_paths() {
    let input = "line 1: src/lib.rs\nline 2: src/main.rs";
    let result = link_files(input, REPO_URL, COMMIT, resolve_path);
    assert_eq!(
        result,
        "line 1: [src/lib.rs](https://github.com/owner/repo/blob/1234567/src/lib.rs)\nline 2: [src/main.rs](https://github.com/owner/repo/blob/1234567/src/main.rs)"
    );
}

#[test]
fn cargo_toml_linked() {
    let input = "edit Cargo.toml";
    let result = link_files(input, REPO_URL, COMMIT, resolve_path);
    assert_eq!(
        result,
        "edit [Cargo.toml](https://github.com/owner/repo/blob/1234567/Cargo.toml)"
    );
}

#[test]
fn nested_path_linked() {
    let input = "`crates/agent-core/src/files.rs:1-50`";
    let result = link_files(input, REPO_URL, COMMIT, resolve_path);
    assert_eq!(
        result,
        "[`crates/agent-core/src/files.rs:1-50`](https://github.com/owner/repo/blob/1234567/crates/agent-core/src/files.rs#L1-L50)"
    );
}

#[test]
fn blockquote_content_processed() {
    let input = "> see src/lib.rs";
    let result = link_files(input, REPO_URL, COMMIT, resolve_path);
    assert_eq!(
        result,
        "> see [src/lib.rs](https://github.com/owner/repo/blob/1234567/src/lib.rs)"
    );
}

#[test]
fn list_item_content_processed() {
    let input = "- see src/lib.rs";
    let result = link_files(input, REPO_URL, COMMIT, resolve_path);
    assert_eq!(
        result,
        "- see [src/lib.rs](https://github.com/owner/repo/blob/1234567/src/lib.rs)"
    );
}

#[test]
fn tilde_fence_protects_content() {
    let input = "~~~\nsrc/lib.rs\n~~~";
    let result = link_files(input, REPO_URL, COMMIT, resolve_path);
    assert_eq!(result, input);
}

#[test]
fn reference_link_preserved() {
    let input = "[link][ref]\n\n[ref]: src/lib.rs";
    let result = link_files(input, REPO_URL, COMMIT, resolve_path);
    assert_eq!(result, input);
}

#[test]
fn root_level_readme_linked() {
    let input = "see README.md";
    let result = link_files(input, REPO_URL, COMMIT, |path| path == "README.md");
    assert_eq!(
        result,
        "see [README.md](https://github.com/owner/repo/blob/1234567/README.md)"
    );
}

#[test]
fn parentheses_in_path_encoded() {
    let input = "`src/file(1).rs`";
    let result = link_files(input, REPO_URL, COMMIT, |path| path == "src/file(1).rs");
    assert_eq!(
        result,
        "[`src/file(1).rs`](https://github.com/owner/repo/blob/1234567/src/file%281%29.rs)"
    );
}

#[test]
fn path_with_commit_like_substring_not_double_linked() {
    // Path with a1b2c3d substring should not be parsed as commit
    let input = "see src/a1b2c3d_code.rs";
    let result = link_files(input, REPO_URL, COMMIT, |path| path == "src/a1b2c3d_code.rs");
    // Should link the path, not the substring
    assert_eq!(
        result,
        "see [src/a1b2c3d_code.rs](https://github.com/owner/repo/blob/1234567/src/a1b2c3d_code.rs)"
    );
}

/// File references and commit words share Markdown protection and never see generated output.
#[test]
fn shared_walk_preserves_spelling_anchors_and_markdown_boundaries() {
    let commit: crate::state::CommitId = "0123456789012345678901234567890123456789".parse().unwrap();
    let file_url = format!("{REPO_URL}/blob/{commit}/src/lib.rs");
    let file = format!("[src/lib.rs]({file_url})");
    let id = "abcdef0";
    let full = "abcdef0123456789012345678901234567890123";
    let linked_id = format!("[`{id}`]({REPO_URL}/commit/{full})");
    let cases = [
        ("src/lib.rs:7", format!("[src/lib.rs:7]({file_url}#L7)")),
        ("src/lib.rs:7-12", format!("[src/lib.rs:7-12]({file_url}#L7-L12)")),
        ("`src/lib.rs:7-12`", format!("[`src/lib.rs:7-12`]({file_url}#L7-L12)")),
        ("src/lib.rs.", format!("{file}.")),
        ("src/lib.rs,", format!("{file},")),
        ("src/lib.rs;", format!("{file};")),
        ("(src/lib.rs)", format!("({file})")),
        ("src/lib.rs:", format!("{file}:")),
        ("abcdef0 src/lib.rs", format!("{linked_id} {file}")),
        ("src/lib.rs abcdef0", format!("{file} {linked_id}")),
        (
            "`abcdef0.rs`",
            format!("[`abcdef0.rs`]({REPO_URL}/blob/{commit}/abcdef0.rs)"),
        ),
        ("src/lib.rs_backup", "src/lib.rs_backup".into()),
        ("src/lib.rs@backup", "src/lib.rs@backup".into()),
        ("../src/lib.rs", "../src/lib.rs".into()),
        ("/src/lib.rs", "/src/lib.rs".into()),
        ("`see src/lib.rs`", "`see src/lib.rs`".into()),
        ("``src/lib.rs``", "``src/lib.rs``".into()),
        (
            "> ```text\n> src/lib.rs\n> ```",
            "> ```text\n> src/lib.rs\n> ```".into(),
        ),
        (
            "- ```text\n  src/lib.rs\n  ```",
            "- ```text\n  src/lib.rs\n  ```".into(),
        ),
        ("[src/lib.rs](elsewhere)", "[src/lib.rs](elsewhere)".into()),
        (
            "[src/lib.rs][ref]\n\n[ref]: /dest",
            "[src/lib.rs][ref]\n\n[ref]: /dest".into(),
        ),
        ("<b title='src/lib.rs'>", "<b title='src/lib.rs'>".into()),
        ("<!-- src/lib.rs -->", "<!-- src/lib.rs -->".into()),
        (
            "https://example.com/src/lib.rs",
            "https://example.com/src/lib.rs".into(),
        ),
        ("nothing here", "nothing here".into()),
        (
            "`src/café #1.rs`",
            format!("[`src/café #1.rs`]({REPO_URL}/blob/{commit}/src/caf%C3%A9%20%231.rs)"),
        ),
    ];
    for (input, expected) in cases {
        let output = crate::commits::link_references(
            input,
            REPO_URL,
            &commit,
            |word| (word == id).then(|| full.to_owned()),
            |path| matches!(path, "src/lib.rs" | "abcdef0.rs" | "src/café #1.rs"),
        );
        assert_eq!(output, expected, "input: {input}");
    }
}

/// Adding file linking preserves the existing punctuation boundaries of commit words.
#[test]
fn combined_linking_preserves_commit_word_boundaries() {
    let commit = "0123456789012345678901234567890123456789".parse().unwrap();
    for input in [
        "abcdef0:note",
        "abcdef0/unknown",
        "(abcdef0)",
        "abcdef0.rs",
        "abcdef0!",
        "éabcdef0",
        "abcdef0_backup",
    ] {
        let resolve = |word: &str| (word == "abcdef0").then(|| "abcdef0123456789012345678901234567890123".to_owned());
        assert_eq!(
            crate::commits::link_references(input, REPO_URL, &commit, resolve, |_| false),
            crate::commits::link_commits(input, REPO_URL, resolve),
            "input: {input}",
        );
    }
}

/// Interior punctuation belongs to the exact tree path, unlike sentence suffixes.
#[test]
fn bare_paths_preserve_interior_punctuation() {
    let commit = "0123456789012345678901234567890123456789".parse().unwrap();
    for (input, path, encoded, prefix, suffix) in [
        ("src/file(1).rs", "src/file(1).rs", "src/file%281%29.rs", "", ""),
        ("(src/file(1).rs).", "src/file(1).rs", "src/file%281%29.rs", "(", ")."),
        ("src/a,b;v.rs,", "src/a,b;v.rs", "src/a%2Cb%3Bv.rs", "", ","),
    ] {
        let result = crate::commits::link_references(input, REPO_URL, &commit, |_| None, |candidate| candidate == path);
        assert_eq!(
            result,
            format!("{prefix}[{path}]({REPO_URL}/blob/{commit}/{encoded}){suffix}"),
            "{input}"
        );
    }
}

/// A hexadecimal HTML tag name must not expose its attributes to file linking.
#[test]
fn hexadecimal_html_tags_preserve_file_attributes() {
    let commit = "0123456789012345678901234567890123456789".parse().unwrap();
    let input = "<deadbeef title='src/lib.rs'>src/lib.rs</deadbeef>";
    let result = crate::commits::link_references(input, REPO_URL, &commit, |_| None, |path| path == "src/lib.rs");
    assert_eq!(
        result,
        format!("<deadbeef title='src/lib.rs'>[src/lib.rs]({REPO_URL}/blob/{commit}/src/lib.rs)</deadbeef>")
    );
}

/// Markdown list markers are structural even when the tree contains their digits.
#[test]
fn ordered_list_markers_are_not_file_references() {
    let commit = "0123456789012345678901234567890123456789".parse().unwrap();
    let input = "1. item\n> 2) item\n- 3. item\n";
    assert_eq!(
        crate::commits::link_references(
            input,
            REPO_URL,
            &commit,
            |_| None,
            |path| matches!(path, "1" | "2" | "3")
        ),
        input
    );
}

/// Syntactically forbidden paths never reach even an accepting tree lookup.
#[test]
fn invalid_root_relative_paths_are_refused_before_lookup() {
    let commit = "0123456789012345678901234567890123456789".parse().unwrap();
    for input in [
        "/src/lib.rs",
        "../src/lib.rs",
        "./src/lib.rs",
        "src//lib.rs",
        "`/src/lib.rs`",
        "`../src/lib.rs`",
        "`./src/lib.rs`",
        "`src//lib.rs`",
    ] {
        assert_eq!(
            crate::commits::link_references(input, REPO_URL, &commit, |_| None, |_| true),
            input,
            "{input}"
        );
    }
}
