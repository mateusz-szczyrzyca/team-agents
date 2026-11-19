//! Root anchoring, supported globs and the rules that cannot be allowed.
use super::*;

#[test]
fn allowances_match_root_paths_and_supported_globs() {
    for (pattern, yes, no) in [
        ("Cargo.toml", "Cargo.toml", "sub/Cargo.toml"),
        ("tests/*.golden", "tests/żółć.golden", "tests/sub/a.golden"),
        ("tests/**/case?.txt", "tests/sub/case1.txt", "tests/sub/case12.txt"),
        ("tests/**/*.bin", "tests/a/b.bin", "src/a.bin"),
    ] {
        let allowance = TaskAllowance::parse(pattern).unwrap();
        assert_eq!(allowance.as_str(), pattern);
        assert!(allowance.matches(&yes.parse().unwrap()), "{pattern}: {yes}");
        assert!(!allowance.matches(&no.parse().unwrap()), "{pattern}: {no}");
    }
}

#[test]
fn allowances_refuse_invalid_forms_and_every_glob_reaching_rules() {
    for pattern in ["", "/tmp/a", "../a", "x/../y", "a[bc]", "a{b}", "a\\b", "a/**b"] {
        assert!(
            matches!(TaskAllowance::parse(pattern), Err(AllowanceError::Pattern(_))),
            "{pattern}"
        );
    }
    for pattern in [
        "*",
        "**",
        "**/*",
        "AGENTS.md",
        "CLAUDE.md",
        ".github/workflows/x.yml",
        "*.md",
        ".github/**",
        "**/CLAUDE.md",
        ".git*/work*/job?.yml",
        "**/workflows/*.yml",
        "sub/AGENTS.md",
        "**/.github/**",
        ".github/workflows/**/file.txt",
    ] {
        assert_eq!(TaskAllowance::parse(pattern), Err(AllowanceError::Rules), "{pattern}");
    }
}
