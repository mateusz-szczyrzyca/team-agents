//! The summary of a CLI's search and what the rule makes of it: the path, the filters and the pattern each keep their
//! meaning.

use super::{SearchInput, search_summary};
use crate::cli::ActivityEvent;
use crate::serena::code_search;

/// The longest summary the tests allow.
const MAX: usize = 160;

/// A cut of `text` to `max` characters, with an ellipsis after it, as the runtime cuts.
fn clip(text: &str, max: usize) -> String {
    let mut chars = text.chars();
    let mut kept: String = chars.by_ref().take(max).collect();
    if chars.next().is_some() {
        kept.push('…');
    }
    kept
}

/// What the rule tells of the call of `name` with `input`, through its summary: the paths, or `None`.
fn told(name: &str, input: &SearchInput<'_>) -> Option<Vec<String>> {
    let summary = search_summary(name, input, clip, MAX).expect("a search has a summary");
    let event = ActivityEvent::ToolCall {
        name: name.to_owned(),
        summary,
    };
    code_search(&event).map(|found| found.paths)
}

/// A search for `pattern` over `path`, limited by `globs` and `types`.
fn search<'a>(pattern: &'a str, path: Option<&'a str>, globs: &[&'a str], types: &[&'a str]) -> SearchInput<'a> {
    SearchInput {
        pattern: Some(pattern),
        path,
        globs: globs.to_vec(),
        types: types.to_vec(),
    }
}

/// The paths as a list of texts, for a table.
fn paths(of: &[&str]) -> Option<Vec<String>> {
    (!of.is_empty()).then(|| of.iter().map(|path| (*path).to_owned()).collect())
}

#[test]
fn a_search_limited_to_text_by_a_glob_is_no_search_of_code() {
    for name in ["Grep", "grep"] {
        assert_eq!(
            told(name, &search("fn", None, &["*.md"], &[])),
            None,
            "{name} over the repository"
        );
        assert_eq!(
            told(name, &search("fn", Some("src"), &["*.md"], &[])),
            None,
            "{name} over src"
        );
        assert_eq!(
            told(name, &search("fn", Some("."), &["*.{md,toml}"], &[])),
            None,
            "{name} with two kinds of text"
        );
    }
}

#[test]
fn a_search_limited_to_text_by_a_type_is_no_search_of_code() {
    assert_eq!(told("Grep", &search("fn", Some("src"), &[], &["md"])), None);
    assert_eq!(told("Grep", &search("fn", None, &[], &["toml"])), None);
}

#[test]
fn a_search_limited_to_code_names_what_it_searches() {
    assert_eq!(
        told("Grep", &search("fn", Some("src"), &["*.rs"], &[])),
        paths(&["src/*.rs"])
    );
    assert_eq!(told("grep", &search("fn", None, &["*.rs"], &[])), paths(&["*.rs"]));
    assert_eq!(
        told("Grep", &search("fn", Some("src"), &[], &["rust"])),
        paths(&["src/*.rs"])
    );
    assert_eq!(
        told("Grep", &search("fn", Some("crates"), &["**/*.go"], &[])),
        paths(&["crates/**/*.go"])
    );
}

#[test]
fn a_search_limited_to_text_and_code_together_is_a_search_of_code() {
    assert_eq!(
        told("Grep", &search("fn", Some("src"), &["*.{rs,md}"], &[])),
        paths(&["src/*.rs"])
    );
    assert_eq!(
        told("Grep", &search("fn", Some("src"), &["*.md"], &["rust"])),
        paths(&["src/*.rs"])
    );
    assert_eq!(
        told("grep", &search("fn", Some("docs"), &["*.rs", "*.md"], &[])),
        paths(&["docs/*.rs"])
    );
}

#[test]
fn an_exclusion_and_an_unknown_filter_leave_the_path_as_it_was() {
    assert_eq!(
        told("Grep", &search("fn", Some("src"), &["!*.md"], &[])),
        paths(&["src"])
    );
    assert_eq!(
        told("Grep", &search("fn", Some("src"), &["Makefile"], &["nosuchtype"])),
        paths(&["src"])
    );
    assert_eq!(told("Grep", &search("fn", Some("docs"), &["!*.md"], &[])), None);
}

#[test]
fn a_search_without_a_filter_keeps_its_summary_as_the_runs_before_wrote_it() {
    let summary = |input: &SearchInput<'_>| search_summary("Grep", input, clip, MAX);

    assert_eq!(summary(&search("foo", None, &[], &[])).as_deref(), Some("foo"));
    assert_eq!(
        summary(&search("foo", Some("src"), &[], &[])).as_deref(),
        Some("src (path)")
    );
}

#[test]
fn a_pattern_that_ends_like_a_path_is_still_a_pattern_over_the_repository() {
    for pattern in [
        "docs (path)",
        "foo (path)",
        "x (path) (path)",
        "y (pattern)",
        "z (path) (pattern)",
    ] {
        for name in ["Grep", "grep"] {
            assert_eq!(
                told(name, &search(pattern, None, &[], &[])),
                paths(&["."]),
                "{name} {pattern:?}"
            );
        }
    }
}

#[test]
fn a_pattern_that_ends_like_a_path_is_marked_as_a_pattern_in_its_summary() {
    let summary = search_summary("Grep", &search("docs (path)", None, &[], &[]), clip, MAX);

    assert_eq!(summary.as_deref(), Some("docs (path) (pattern)"));
}

#[test]
fn a_long_pattern_that_ends_like_a_path_is_cut_to_make_room_for_its_mark() {
    let pattern = format!("{} (path)", "a".repeat(13));

    let summary = search_summary("Grep", &search(&pattern, None, &[], &[]), clip, 20);

    assert_eq!(summary.as_deref(), Some("aaaaaaaaaa… (pattern)"));
}

#[test]
fn a_long_path_is_cut_to_make_room_for_its_mark() {
    let path = "d".repeat(300);

    let summary = search_summary("Grep", &search("x", Some(&path), &[], &[]), clip, 40);

    assert_eq!(summary, Some(format!("{}… (path)", "d".repeat(33))));
}

#[test]
fn a_glob_names_its_pattern_under_the_path_it_was_given() {
    for name in ["Glob", "glob"] {
        let glob = |pattern: &'static str, path: Option<&'static str>| SearchInput {
            pattern: Some(pattern),
            path,
            ..SearchInput::default()
        };
        assert_eq!(told(name, &glob("*.rs", Some("src"))), paths(&["src/*.rs"]), "{name}");
        assert_eq!(told(name, &glob("**/*.go", Some("."))), paths(&["./**/*.go"]), "{name}");
        assert_eq!(told(name, &glob("*.rs", None)), paths(&["*.rs"]), "{name}");
        assert_eq!(told(name, &glob("*.md", Some("docs"))), None, "{name}");
        assert_eq!(
            told(name, &glob("/work/tree/**/lib.rs", Some("src"))),
            paths(&["/work/tree/**/lib.rs"]),
            "{name}"
        );
        assert_eq!(
            told(name, &glob("src/**/*.rs", Some("/w/"))),
            paths(&["/w/src/**/*.rs"]),
            "{name}"
        );
    }
}

#[test]
fn a_tool_that_does_not_search_has_no_search_summary() {
    let input = search("x", Some("src"), &[], &[]);

    for name in ["Read", "read", "Bash", "Task", "mcp__serena__find_symbol"] {
        assert_eq!(search_summary(name, &input, clip, MAX), None, "{name}");
    }
    assert_eq!(search_summary("Grep", &SearchInput::default(), clip, MAX), None);
}

#[test]
fn a_filter_with_many_brace_groups_is_expanded_to_a_bounded_number_of_globs() {
    let glob = "*.{rs,md,rs,md,rs,md,rs,md}".repeat(12);
    let input = search("fn", Some("src"), &[&glob], &[]);

    let summary = search_summary("Grep", &input, clip, 100_000).expect("a search has a summary");

    assert!(summary.split(' ').count() <= 65, "{} words", summary.split(' ').count());
}

#[test]
fn a_type_of_text_and_a_type_named_by_its_language_are_told_apart() {
    assert_eq!(told("Grep", &search("fn", Some("src"), &[], &["markdown"])), None);
    assert_eq!(
        told("Grep", &search("fn", Some("src"), &[], &["python"])),
        paths(&["src/*.py"])
    );
    assert_eq!(
        told("Grep", &search("fn", Some("src"), &[], &["go"])),
        paths(&["src/*.go"])
    );
}
