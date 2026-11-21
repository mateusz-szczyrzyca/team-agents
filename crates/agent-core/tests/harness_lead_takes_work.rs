//! The message a lead is given when the work of a member is its own now, against its golden text in
//! `fixtures/lead-takes-work/`.

#[cfg(test)]
mod harness_lead_takes_work {
    use std::path::Path;

    use agent_core::prompt::{PROMPTS_DIR, TeamPrompt, Variables};

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
    fn a_lead_is_told_whose_work_it_has_where_it_is_and_what_to_do_with_it() {
        let spec = TeamPrompt::LeadTakesWork.spec();
        let values = Variables::new()
            .with("member", "claude-1")
            .unwrap()
            .with("task", "task-20261121-101500-k2m4")
            .unwrap()
            .with("branch", "agents/claude-1/task-20261121-101500-k2m4-1.1")
            .unwrap()
            .with("worktree", "/srv/agents/worktrees/example/task-20261121-101500-k2m4")
            .unwrap();

        let text = spec.render(&read_prompt(spec.path()), &values).unwrap();

        assert_eq!(text, golden("bus-lead-takes-work.txt"));
    }
}
