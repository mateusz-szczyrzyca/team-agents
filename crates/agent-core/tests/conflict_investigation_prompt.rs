//! The additional conflict pass extends the existing instructions with an investigator's answer.

use agent_core::prompt::{AutopilotPrompt, Variables};

#[test]
fn the_ordinary_passes_render_no_investigator_text() {
    let values = Variables::new()
        .with("pr", "150")
        .unwrap()
        .with("base", "main")
        .unwrap()
        .with("branch", "agents/claude-1/task-1-1.1")
        .unwrap()
        .with("pass", "1")
        .unwrap()
        .with("passes", "3")
        .unwrap()
        .with("investigator_answer", "")
        .unwrap();
    let template = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../prompts/harness/pr/merge-conflict.md"),
    )
    .unwrap();
    let rendered = AutopilotPrompt::PrMergeConflict.spec().render(&template, &values);
    assert!(
        matches!(&rendered, Ok(text) if text == include_str!("fixtures/merge-conflict-lead.md")),
        "{rendered:?}"
    );
}
