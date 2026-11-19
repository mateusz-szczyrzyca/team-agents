//! Allow directives belong to the header and preserve original line numbers.
use super::*;

#[test]
fn allowance_header_preserves_patterns_and_excludes_them_from_brief() {
    let list = TaskList::parse("\u{feff}\nCoordinator: inv-opus\n\nAllow: Cargo.toml\nAllow: tests/**/*.golden\n\nŻółć.\n\n1. Feature\n[ ] 1.1 Work\n").unwrap();
    assert_eq!(
        list.allowances()
            .iter()
            .map(crate::task_allowance::TaskAllowance::as_str)
            .collect::<Vec<_>>(),
        ["Cargo.toml", "tests/**/*.golden"]
    );
    assert_eq!(list.brief(), "Żółć.");
    assert_eq!(list.coordinator().unwrap().as_str(), "inv-opus");
    assert_eq!(TaskList::parse("Plain brief").unwrap().allowances(), []);
    assert!(matches!(
        TaskList::parse("Allow: tests/**/*.golden"),
        Err(TaskListError::Empty)
    ));
}

#[test]
fn invalid_allowance_names_its_original_line_and_pattern() {
    for pattern in [
        "",
        "/absolute",
        "../parent",
        "*",
        "**",
        "**/*",
        "AGENTS.md",
        "CLAUDE.md",
        ".github/workflows/x.yml",
        "*.md",
        ".github/**",
        "**/CLAUDE.md",
        "tests/[ab]",
    ] {
        let result = TaskList::parse(&format!("\nCoordinator: inv-opus\nAllow: {pattern}\nBrief"));
        assert!(
            matches!(&result, Err(TaskListError::InvalidAllowance { line, text, .. }) if line.get() == 3 && text == pattern),
            "{pattern}: {result:?}"
        );
        assert!(result.unwrap_err().to_string().contains("line 3: invalid Allow:"));
    }
}

#[test]
fn allow_line_after_brief_is_text_and_after_feature_is_unexpected() {
    assert_eq!(
        TaskList::parse("Brief\nAllow: tests/**").unwrap().brief(),
        "Brief\nAllow: tests/**"
    );
    assert!(
        matches!(TaskList::parse("1. Feature\nAllow: tests/**"), Err(TaskListError::UnexpectedLine { line }) if line.get() == 2)
    );
}
