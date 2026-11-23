//! The note that tells a run retried after its session reported `STATUS: FAILED` without a tool call why it starts
//! again, rendered through the real text of `prompts/harness/run/no-tool-call.md` and held to its golden file in
//! `fixtures/run/`: it names the agent and points the retry at the bus tools in the tool catalog under the server
//! `agentbus`.

#[cfg(test)]
mod harness_no_tool_call {
    use std::path::Path;

    use agent_core::prompt::{HarnessPrompt, PROMPTS_DIR, Variables};

    /// The note rendered through the real text of `prompts/harness/run/no-tool-call.md`.
    fn render(agent: &str) -> String {
        let spec = HarnessPrompt::NoToolCall.spec();
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../")
            .join(PROMPTS_DIR)
            .join(spec.path());
        let text = std::fs::read_to_string(path).unwrap();
        let values = Variables::new().with("agent", agent).unwrap();
        spec.render(&text, &values).unwrap()
    }

    /// The golden text of the note.
    fn golden() -> String {
        std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/run/no-tool-call.txt"))
            .unwrap()
    }

    #[test]
    fn a_retried_run_is_pointed_at_the_bus_tools_in_the_catalog_under_the_server_agentbus() {
        let text = render("deepseek-public");

        assert_eq!(text, golden());
        assert!(text.contains("`agentbus`"), "{text}");
        assert!(text.contains("tool catalog"), "{text}");
    }
}
