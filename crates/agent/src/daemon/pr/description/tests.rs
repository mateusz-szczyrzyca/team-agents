//! Description ownership and gate provenance contracts.
use super::*;
use agent_core::config::Language;
use agent_core::state::{GateOutcome, GateResult, GateTree};
use std::num::NonZeroI32;
use std::path::Path;

#[test]
fn lead_keeps_other_sections_and_fenced_headings_but_cannot_supply_size_or_gate() {
    let input = "## What and why\n\nŻółć.\n\n```markdown\n## Size\nexample\n```\n\n## Size\n\nwrong\n### Detail\nwrong too\n\n## Tests\n\nSingle tests.\n\n## Gate\n\nwrong\n\n## Out of scope\n\nNone.\n";
    assert_eq!(
        lead_sections(input),
        "## What and why\n\nŻółć.\n\n```markdown\n## Size\nexample\n```\n\n## Tests\n\nSingle tests.\n\n## Out of scope\n\nNone."
    );
    assert_eq!(lead_sections("\n  \n"), "");
}

#[tokio::test]
async fn measured_sections_are_written_from_records_and_only_a_same_head_failure_marks_a_retry() {
    let texts = AutopilotPrompts::load(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("../../prompts"),
        &Language::default(),
    )
    .await
    .unwrap();
    let gate = GateRecord {
        tree: GateTree::Clean,
        result: GateResult {
            head: "26b65de7e4076596768ca96b48a7f94799c93874".parse().unwrap(),
            finished_at: "2026-11-19T09:30:00Z".parse().unwrap(),
            outcome: GateOutcome::Passed,
            log: "/tmp/tasks/task-1/gate-2.log".parse().unwrap(),
        },
    };
    let mut previous = gate.clone();
    previous.result.outcome = GateOutcome::Failed {
        exit_code: NonZeroI32::new(2).unwrap(),
    };
    let plain = include_str!("plain.md").trim_end();
    let retry = include_str!("retry.md").trim_end();
    assert_eq!(measured_sections(&texts, 42, &gate, None).unwrap(), plain);
    assert_eq!(measured_sections(&texts, 42, &gate, Some(&previous)).unwrap(), retry);
    previous.result.head = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".parse().unwrap();
    assert_eq!(measured_sections(&texts, 42, &gate, Some(&previous)).unwrap(), plain);
    assert_eq!(measured_sections(&texts, 42, &gate, Some(&gate)).unwrap(), plain);
}

#[tokio::test]
async fn allowance_section_names_changed_paths_and_the_covering_line_only() {
    use agent_core::decide::checks::{Change, FileChange, Lines};
    use agent_core::task_allowance::TaskAllowance;
    let texts = AutopilotPrompts::load(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("../../prompts"),
        &Language::default(),
    )
    .await
    .unwrap();
    let files = [
        FileChange {
            path: "Cargo.toml".parse().unwrap(),
            change: Change::Modified,
            lines: Lines::Text { added: 1, removed: 1 },
        },
        FileChange {
            path: "tests/café.golden".parse().unwrap(),
            change: Change::Renamed {
                from: "tests/old.golden".parse().unwrap(),
            },
            lines: Lines::Binary,
        },
        FileChange {
            path: "src/code.rs".parse().unwrap(),
            change: Change::Added,
            lines: Lines::Text { added: 1, removed: 0 },
        },
    ];
    let allowances = [
        TaskAllowance::parse("Cargo.toml").unwrap(),
        TaskAllowance::parse("tests/*.golden").unwrap(),
    ];
    assert_eq!(allowed_section(&texts, &files, &allowances).unwrap(), Some("## Allowed beforehand\n\n- `Cargo.toml` — `Allow: Cargo.toml`\n- `tests/café.golden` — `Allow: tests/*.golden`\n- `tests/old.golden` — `Allow: tests/*.golden`\n".to_owned()));
    assert_eq!(allowed_section(&texts, &files, &[]).unwrap(), None);
    assert_eq!(allowed_section(&texts, &[], &allowances).unwrap(), None);
    assert_eq!(
        lead_sections("## Allowed beforehand\n\nforged\n\n## Tests\n\nreal"),
        "## Tests\n\nreal"
    );
}

#[test]
fn the_lead_cannot_replace_the_unpushed_base_evidence() {
    assert_eq!(
        lead_sections("## What and why\n\nWork.\n\n## Unpushed base\n\nClaim.\n\n## Tests\n\nTests."),
        "## What and why\n\nWork.\n\n## Tests\n\nTests."
    );
}
