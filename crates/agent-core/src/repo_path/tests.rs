//! The paths of a repository and the patterns that match them.

use super::*;

fn path(text: &str) -> RepoPath {
    text.parse().unwrap()
}

fn pattern(text: &str) -> PathPattern {
    text.parse().unwrap()
}

// Rule: a repository path is relative, made of non-empty segments that are not `.` or `..`, free of control characters,
// and as long as a path can be.
#[test]
fn a_repository_path_that_is_relative_and_clean_is_accepted() {
    for text in [
        "a",
        "a/b",
        "crates/agent-core/src/lib.rs",
        ".github/workflows/ci.yml",
        "dir with space/f.rs",
        "ąę/ü.rs",
    ] {
        assert_eq!(path(text).as_str(), text);
    }
}

#[test]
fn a_repository_path_that_breaks_a_rule_is_refused_with_the_rule() {
    let too_long = "a".repeat(MAX_REPO_PATH_LEN + 1);
    let cases = [
        ("", PathRule::Empty),
        (too_long.as_str(), PathRule::TooLong { max: MAX_REPO_PATH_LEN }),
        ("/etc/passwd", PathRule::Absolute),
        ("a//b", PathRule::EmptySegment),
        ("a/", PathRule::EmptySegment),
        ("a/./b", PathRule::DotSegment),
        ("../b", PathRule::DotSegment),
        ("a/..", PathRule::DotSegment),
        ("a\nb", PathRule::Control('\n')),
        ("a\0b", PathRule::Control('\0')),
    ];
    for (text, rule) in cases {
        let error = text.parse::<RepoPath>().unwrap_err();
        assert_eq!(error.rule, rule, "{text:?}");
        assert_eq!(error.value, text);
    }
}

#[test]
fn a_repository_path_of_the_longest_length_is_accepted() {
    let longest = "a".repeat(MAX_REPO_PATH_LEN);

    assert_eq!(path(&longest).as_str().len(), MAX_REPO_PATH_LEN);
}

#[test]
fn a_repository_path_is_read_from_json_through_the_same_check() {
    assert_eq!(serde_json::from_str::<RepoPath>("\"a/b.rs\"").unwrap(), path("a/b.rs"));
    assert!(serde_json::from_str::<RepoPath>("\"../b.rs\"").unwrap_err().is_data());
}

// Rule: the extension is what follows the last dot of the file's name; a name that only starts with a dot has none.
#[test]
fn the_extension_is_what_follows_the_last_dot_of_the_file_name() {
    let cases = [
        ("a.rs", Some("rs")),
        ("dir.d/a", None),
        ("dir.d/a.tar.gz", Some("gz")),
        (".gitignore", None),
        ("a/.hidden.rs", Some("rs")),
        ("Makefile", None),
        ("trailing.", None),
    ];
    for (text, extension) in cases {
        assert_eq!(path(text).extension(), extension, "{text}");
    }
}

#[test]
fn the_file_name_is_the_last_segment() {
    assert_eq!(path("a/b/c.rs").file_name(), "c.rs");
    assert_eq!(path("c.rs").file_name(), "c.rs");
}

// Rule: a pattern is relative, has no empty or `..` segment and none of the characters reserved for later syntax, and
// uses `**` only as a whole segment, a few times at most.
#[test]
fn a_pattern_that_breaks_a_rule_is_refused_with_the_rule() {
    let too_long = "a".repeat(MAX_PATTERN_LEN + 1);
    let cases = [
        ("", PatternRule::Empty),
        (too_long.as_str(), PatternRule::TooLong { max: MAX_PATTERN_LEN }),
        ("/tests/**", PatternRule::Absolute),
        ("a//b", PatternRule::EmptySegment),
        ("a/../b", PatternRule::ParentSegment),
        ("a[bc].rs", PatternRule::ForbiddenChar('[')),
        ("a{b,c}.rs", PatternRule::ForbiddenChar('{')),
        ("a\\b", PatternRule::ForbiddenChar('\\')),
        ("a\nb", PatternRule::ForbiddenChar('\n')),
        ("a**/b", PatternRule::MixedDoubleStar),
        ("a/**b", PatternRule::MixedDoubleStar),
        (
            "**/**/**/**/**/a",
            PatternRule::TooManyDoubleStars { max: MAX_DOUBLE_STARS },
        ),
    ];
    for (text, rule) in cases {
        let error = text.parse::<PathPattern>().unwrap_err();
        assert_eq!(error.rule, rule, "{text:?}");
    }
}

#[test]
fn a_pattern_that_follows_the_rules_is_accepted() {
    for text in [
        "tests/",
        "**/tests.rs",
        "*_test.go",
        "AGENTS.md",
        ".github/**",
        "crates/*/src/**",
        "a/**/b/**",
        "x?.rs",
    ] {
        assert_eq!(pattern(text).as_str(), text);
    }
}

#[test]
fn a_pattern_is_read_from_json_through_the_same_check() {
    assert_eq!(
        serde_json::from_str::<PathPattern>("\"tests/**\"").unwrap(),
        pattern("tests/**")
    );
    assert!(serde_json::from_str::<PathPattern>("\"/tests\"").unwrap_err().is_data());
}

// Rule: what a pattern matches. Each case is a pattern, a path and whether the pattern matches it; the neighbours of
// every match are in the table.
#[test]
fn patterns_match_paths_as_the_module_says() {
    let cases = [
        // a directory: everything below it, anchored at the root
        ("tests/", "tests/a.rs", true),
        ("tests/", "tests/unit/a.bats", true),
        ("tests/", "tests", false),
        ("tests/", "src/tests/a.rs", false),
        ("tests/", "tests-extra/a.rs", false),
        // `**` at the end: one segment or more
        ("tests/**", "tests/a.rs", true),
        ("tests/**", "tests/a/b/c.rs", true),
        ("tests/**", "tests", false),
        // `**` at the start and in the middle: any number of segments, none included
        ("**/tests.rs", "tests.rs", true),
        ("**/tests.rs", "a/b/tests.rs", true),
        ("**/tests.rs", "a/mytests.rs", false),
        ("a/**/b.rs", "a/b.rs", true),
        ("a/**/b.rs", "a/x/y/b.rs", true),
        ("a/**/b.rs", "x/a/b.rs", false),
        // a name: at any depth, never across a `/`
        ("AGENTS.md", "AGENTS.md", true),
        ("AGENTS.md", "docs/team/AGENTS.md", true),
        ("AGENTS.md", "AGENTS.md.bak", false),
        ("*_test.go", "agentbus/server_test.go", true),
        ("*_test.go", "server_test.go", true),
        ("*_test.go", "server_test.gox", false),
        ("*.bats", "tests/unit/watchdog.bats", true),
        // anchored patterns with a wildcard in a segment
        ("crates/*/src/**", "crates/agent-core/src/lib.rs", true),
        ("crates/*/src/**", "crates/agent-core/src/a/b.rs", true),
        ("crates/*/src/**", "crates/agent-core/tests/a.rs", false),
        ("crates/*/src/**", "crates/src/lib.rs", false),
        ("src/*.rs", "src/lib.rs", true),
        ("src/*.rs", "src/a/lib.rs", false),
        ("src/?.rs", "src/a.rs", true),
        ("src/?.rs", "src/ab.rs", false),
        ("src/?.rs", "src/.rs", false),
        // `*` matches the empty text and dotfiles, and several `*` in a segment backtrack
        ("src/*", "src/.hidden", true),
        ("a*b*c", "abc", true),
        ("a*b*c", "aXbYc", true),
        ("a*b*c", "aXbY", false),
        ("*", "any.thing", true),
        // non-ASCII text is matched by characters
        ("ä?.rs", "dir/äü.rs", true),
        ("ä?.rs", "dir/äüx.rs", false),
    ];
    for (text, candidate, expected) in cases {
        assert_eq!(
            pattern(text).matches(&path(candidate)),
            expected,
            "{text} against {candidate}"
        );
    }
}

#[test]
fn a_path_matches_a_list_when_one_pattern_matches_it() {
    let list = [pattern("tests/"), pattern("*.bats")];

    assert!(matches_any(&list, &path("tests/a.rs")));
    assert!(matches_any(&list, &path("x/y.bats")));
    assert!(!matches_any(&list, &path("src/a.rs")));
    assert!(!matches_any(&[], &path("src/a.rs")));
}
