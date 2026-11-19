//! Correction instructions are loaded from the harness catalog rather than supplied by the parser.
use agent_core::config::Language;
use agent_core::prompt::{AutopilotPrompt, Variables};
use agent_runtime::autopilot_prompts::AutopilotPrompts;
use std::path::Path;

#[tokio::test]
async fn every_typed_fix_response_rule_has_a_catalog_instruction() {
    let cases = [
        (
            "harness/review/response-numbers.md",
            "one answer for each supplied blocking number\n",
        ),
        ("harness/review/response-explain.md", "FIXED or EXPLAINED — why\n"),
        (
            "harness/review/response-fixed.md",
            "FIXED (explanation and second opinion are forbidden)\n",
        ),
    ];
    for (path, _) in &cases {
        assert!(
            AutopilotPrompt::ALL.iter().any(|id| id.spec().path() == *path),
            "missing {path}"
        );
    }
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../prompts");
    let texts = AutopilotPrompts::load(&root, &Language::default()).await.unwrap();
    for (path, expected) in cases {
        let id = AutopilotPrompt::ALL.iter().find(|id| id.spec().path() == path).unwrap();
        assert_eq!(texts.render(*id, &Variables::new()).unwrap(), expected);
    }
}
