//! What a lead given a member's work is told the member's runs left, against its golden text in
//! `fixtures/lead-takes-work/`.

#[cfg(test)]
mod harness_lead_briefing {
    use std::path::Path;

    use agent_core::prompt::{HarnessPrompt, PROMPTS_DIR, Variables};

    /// The text of the catalog's file `path`.
    fn read_prompt(path: &str) -> String {
        std::fs::read_to_string(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../")
                .join(PROMPTS_DIR)
                .join(path),
        )
        .unwrap()
    }

    /// The golden text `name`.
    fn golden(name: &str) -> String {
        std::fs::read_to_string(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/lead-takes-work")
                .join(name),
        )
        .unwrap()
    }

    #[test]
    fn a_lead_is_briefed_on_the_commits_files_plan_and_unread_messages_of_a_members_work() {
        let spec = HarnessPrompt::LeadBriefing.spec();
        let values = [
            ("reason", "no_status"),
            ("task", "task-20261121-101500-k2m4"),
            ("base", "main"),
            ("branch", "agents/claude-1/task-20261121-101500-k2m4-1.1"),
            ("commits", "- 1a2b3c4d Add the parser"),
            ("uncommitted", "- draft.txt\n- notes/żółw.md"),
            (
                "snapshot",
                "refs/agents/snapshots/round-20261121-101400-a1b2/task-20261121-101500-k2m4/1",
            ),
            ("plan", "- [x] 1 Read the code\n- [>] 2 Write the parser"),
            ("left_out", ""),
            ("messages", "- #12 inv-opus:\n  Look at the cache first."),
        ]
        .iter()
        .try_fold(Variables::new(), |values, (name, value)| values.with(name, *value))
        .unwrap();

        let text = spec.render(&read_prompt(spec.path()), &values).unwrap();

        assert_eq!(text, golden("lead-briefing.txt"));
    }
}
