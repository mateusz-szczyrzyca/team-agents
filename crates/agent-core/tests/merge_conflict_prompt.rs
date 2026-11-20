//! The lead's instructions after GitHub refused an approved pull request for a conflict, rendered with the values of a
//! real round and compared with the text the lead is given.

#[cfg(test)]
mod merge_conflict_prompt {
    use agent_core::prompt::{AutopilotPrompt, Variables};

    /// The text the lead is given, rendered, as a fixture.
    const GOLDEN: &str = include_str!("fixtures/merge-conflict-lead.md");

    #[test]
    fn the_lead_is_told_to_merge_the_base_and_not_to_rebase_it_rendered_as_written() {
        let spec = AutopilotPrompt::PrMergeConflict.spec();
        let values = Variables::new()
            .with("pr", "150")
            .and_then(|values| values.with("base", "main"))
            .and_then(|values| values.with("branch", "agents/claude-1/task-1-1.1"))
            .and_then(|values| values.with("pass", "1"))
            .and_then(|values| values.with("passes", "3"))
            .expect("distinct variables");
        let template = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../")
                .join(agent_core::prompt::PROMPTS_DIR)
                .join(spec.path()),
        )
        .expect("the text of the lead exists");

        let rendered = spec.render(&template, &values).expect("every variable is given");

        assert_eq!(rendered, GOLDEN);
    }
}
