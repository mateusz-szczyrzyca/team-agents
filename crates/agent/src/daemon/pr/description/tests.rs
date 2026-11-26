//! Description ownership and gate provenance contracts.
use super::*;
use agent_core::config::Language;
use agent_core::state::{GateOutcome, GateResult, GateTree};
use agent_runtime::agentbus::DelegationStatus;
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

#[test]
fn lead_cannot_replace_the_recorded_split_provenance() {
    assert_eq!(
        lead_sections("## What and why\n\nUseful work.\n\n## Split\n\nInvented approval.\n\n## Tests\n\nPassed."),
        "## What and why\n\nUseful work.\n\n## Tests\n\nPassed."
    );
}

#[tokio::test]
async fn measured_sections_in_formats_gate_finished_time_in_the_daemon_zone() {
    use agent_core::local_time::Zone;
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
    let east = Zone::Fixed(time::UtcOffset::from_hms(1, 0, 0).expect("valid"));

    let rendered = measured_sections_in(&texts, 42, &gate, None, east).unwrap();

    assert!(rendered.contains("Finished: 2026-11-19 10:30:00."), "{rendered}");
}

/// A request of a task as the bus lists it.
fn request(id: u64, step: Option<&str>, worker: Option<&str>, status: DelegationStatus) -> TaskRequest {
    TaskRequest {
        id: agent_runtime::agentbus::RequestId::new(std::num::NonZeroU64::new(id).unwrap()),
        step: step.map(|step| step.parse().unwrap()),
        worker: worker.map(|worker| worker.parse().unwrap()),
        status,
    }
}

#[tokio::test]
async fn the_steps_a_worker_did_are_listed_by_the_harness_and_nothing_else_is() {
    let texts = AutopilotPrompts::load(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("../../prompts"),
        &Language::default(),
    )
    .await
    .unwrap();
    let requests = [
        (
            "claude-1".parse().unwrap(),
            vec![
                request(3, Some("1.2"), Some("glm-1"), DelegationStatus::Done),
                request(4, None, Some("glm-1"), DelegationStatus::Done),
                request(5, Some("1.3"), Some("glm-2"), DelegationStatus::Failed),
                request(6, Some("1.4"), Some("glm-2"), DelegationStatus::Running),
            ],
        ),
        (
            "inv-opus".parse().unwrap(),
            vec![request(7, Some("2.1"), Some("glm-2"), DelegationStatus::Done)],
        ),
    ];

    let section = worker_steps_section(&texts, &requests).unwrap();
    let none = worker_steps_section(&texts, &[("claude-1".parse().unwrap(), vec![])]).unwrap();

    assert_eq!(
        section.as_deref(),
        Some(
            "## Steps done by a worker\n\n- step 1.2 of claude-1's plan — done by glm-1 (H3)\n- step 2.1 of inv-opus's plan — done by glm-2 (H7)\n"
        )
    );
    assert_eq!(none, None);
}

#[test]
fn the_lead_cannot_supply_the_steps_done_by_a_worker() {
    let input = "## What and why\n\nWhy.\n\n## Steps done by a worker\n\n- step 9.9 — done by nobody\n";

    assert_eq!(lead_sections(input), "## What and why\n\nWhy.");
}

/// The prompts of the repository, as the daemon loads them.
async fn repository_texts() -> AutopilotPrompts {
    AutopilotPrompts::load(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("../../prompts"),
        &Language::default(),
    )
    .await
    .unwrap()
}

/// A request that came back to its requester, with the summary the bus holds for it.
fn returned(id: u64, step: Option<&str>, summary: &str) -> (TaskRequest, String) {
    (
        request(id, step, Some("glm-1"), DelegationStatus::Failed),
        summary.to_owned(),
    )
}

#[tokio::test]
async fn a_returned_step_is_listed_with_the_first_line_of_its_summary() {
    let texts = repository_texts().await;
    let requests = [(
        "claude-1".parse().unwrap(),
        vec![returned(
            5,
            Some("1.3"),
            "Result: the file is not written.\nThe worker ran out of turns.",
        )],
    )];

    let section = returned_steps_section(&texts, &requests).unwrap();

    assert_eq!(
        section.as_deref(),
        Some(
            "## Steps returned to their requesters\n\n- step 1.3 of claude-1's plan — returned to claude-1 (H5): Result: the file is not written.\n"
        )
    );
}

#[tokio::test]
async fn a_returned_step_with_an_empty_summary_says_no_reason_was_given() {
    let texts = repository_texts().await;
    let requests = [("claude-1".parse().unwrap(), vec![returned(5, Some("1.3"), "")])];

    let section = returned_steps_section(&texts, &requests).unwrap();

    assert_eq!(
        section.as_deref(),
        Some(
            "## Steps returned to their requesters\n\n- step 1.3 of claude-1's plan — returned to claude-1 (H5): no reason given\n"
        )
    );
}

#[tokio::test]
async fn a_reason_of_two_hundred_characters_is_cut_to_a_hundred_and_twenty() {
    let texts = repository_texts().await;
    let long = "ż".repeat(200);
    let requests = [("claude-1".parse().unwrap(), vec![returned(5, Some("1.3"), &long)])];

    let section = returned_steps_section(&texts, &requests).unwrap();

    let expected = format!(
        "## Steps returned to their requesters\n\n- step 1.3 of claude-1's plan — returned to claude-1 (H5): {}\n",
        "ż".repeat(120)
    );
    assert_eq!(section, Some(expected));
}

#[tokio::test]
async fn only_the_failed_requests_that_name_a_step_are_returned_steps() {
    let texts = repository_texts().await;
    let requests = [
        (
            "claude-1".parse().unwrap(),
            vec![
                returned(3, Some("1.2"), "Result: failed."),
                returned(4, None, "Result: failed, and no step."),
                (
                    request(5, Some("1.3"), Some("glm-2"), DelegationStatus::Done),
                    "Result: done.".to_owned(),
                ),
                (
                    request(6, Some("1.4"), Some("glm-2"), DelegationStatus::Queued),
                    String::new(),
                ),
            ],
        ),
        (
            "inv-opus".parse().unwrap(),
            vec![returned(7, Some("2.1"), "Result: failed too.")],
        ),
    ];

    let section = returned_steps_section(&texts, &requests).unwrap();
    let done = worker_steps_section(
        &texts,
        &requests
            .iter()
            .map(|(requester, of_task)| {
                (
                    requester.clone(),
                    of_task.iter().map(|(request, _)| request.clone()).collect(),
                )
            })
            .collect::<Vec<_>>(),
    )
    .unwrap();

    assert_eq!(
        section.as_deref(),
        Some(
            "## Steps returned to their requesters\n\n- step 1.2 of claude-1's plan — returned to claude-1 (H3): Result: failed.\n- step 2.1 of inv-opus's plan — returned to inv-opus (H7): Result: failed too.\n"
        )
    );
    assert_eq!(
        done.as_deref(),
        Some("## Steps done by a worker\n\n- step 1.3 of claude-1's plan — done by glm-2 (H5)\n")
    );
}

#[tokio::test]
async fn without_a_returned_step_there_is_no_section() {
    let texts = repository_texts().await;
    let requests = [(
        "claude-1".parse().unwrap(),
        vec![(
            request(3, Some("1.2"), Some("glm-1"), DelegationStatus::Done),
            "Result: done.".to_owned(),
        )],
    )];

    assert_eq!(returned_steps_section(&texts, &requests).unwrap(), None);
    assert_eq!(returned_steps_section(&texts, &[]).unwrap(), None);
}

#[test]
fn the_lead_cannot_supply_the_steps_returned_to_their_requesters() {
    let input = "## What and why\n\nWhy.\n\n## Steps returned to their requesters\n\n- step 9.9 — returned to nobody\n";

    assert_eq!(lead_sections(input), "## What and why\n\nWhy.");
}
