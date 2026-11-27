//! The text a run is sent back with when its plan names no requirement of its item, against its golden text.

#[cfg(test)]
mod harness_plan_requirement_prompts {
    use std::path::Path;

    use agent_core::prompt::{HarnessPrompt, PROMPTS_DIR, Variables};

    fn root() -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../")
    }

    #[test]
    fn the_correction_for_a_plan_with_no_requirements_renders_as_its_golden_text() {
        let spec = HarnessPrompt::PlanNoRequirement.spec();
        let text = std::fs::read_to_string(root().join(PROMPTS_DIR).join(spec.path())).unwrap();
        let values = Variables::new()
            .with("without", "1.2")
            .unwrap()
            .with("unknown", "none")
            .unwrap()
            .with("known", "R1, R2, K1")
            .unwrap();

        let rendered = spec.render(&text, &values).unwrap();

        let golden = std::fs::read_to_string(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/plan-prompts/plan-no-requirement.txt"),
        )
        .unwrap();
        assert_eq!(rendered, golden);
    }
}
