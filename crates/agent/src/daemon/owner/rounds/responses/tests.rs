//! Corrections without a located finding must not invent one.
use super::*;

#[tokio::test]
async fn an_unlocated_error_requests_all_numbers_without_offering_a_finding_specific_answer() {
    let texts = agent_runtime::autopilot_prompts::AutopilotPrompts::load(
        &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../prompts"),
        &agent_core::config::Language::default(),
    )
    .await
    .unwrap();

    assert_eq!(
        unlocated_correction(&texts).unwrap(),
        texts
            .render(AutopilotPrompt::ReviewResponseNumbers, &Variables::new())
            .unwrap()
    );
}
