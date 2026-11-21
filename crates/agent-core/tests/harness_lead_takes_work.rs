//! The text a lead is given when a member's task is handed to it, against its golden text in `fixtures/lead-takes-work/`.

#[cfg(test)]
mod harness_lead_takes_work {
    use std::path::Path;

    use agent_core::prompt::{HarnessPrompt, PROMPTS_DIR};

    fn read_prompt(path: &str) -> String {
        std::fs::read_to_string(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../")
                .join(PROMPTS_DIR)
                .join(path),
        )
        .unwrap()
    }

    fn golden(name: &str) -> String {
        std::fs::read_to_string(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/lead-takes-work")
                .join(name),
        )
        .unwrap()
    }

    #[test]
    fn a_lead_is_told_whose_work_it_is_given_and_where_it_is() {
        let spec = HarnessPrompt::LeadTakesWork.spec();
        let values = agent_core::prompt::Variables::new()
            .with("task", "task-20261120-144101-r7q2")
            .unwrap()
            .with("agent", "deepinfra-deepseek")
            .unwrap()
            .with("branch", "agents/deepinfra-deepseek/task-20261120-191109-qesn-1.1")
            .unwrap()
            .with(
                "worktree",
                "/home/dev/agents-rs-trial/worktrees/team-agents/task-20261120-191109-qesn",
            )
            .unwrap();

        let text = spec.render(&read_prompt(spec.path()), &values).unwrap();

        assert_eq!(text, golden("lead-takes-work.txt"));
    }
}
