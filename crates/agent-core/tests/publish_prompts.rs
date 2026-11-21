//! Golden tests for publication prompts and branch-switch instructions.

#[cfg(test)]
mod publish_prompts {
    use std::path::Path;

    use agent_core::prompt::PROMPTS_DIR;

    fn read_prompt(path: &str) -> String {
        std::fs::read_to_string(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../")
                .join(PROMPTS_DIR)
                .join(path),
        )
        .unwrap_or_else(|error| panic!("{path}: {error}"))
    }

    #[test]
    fn agent_prompts_instruct_not_to_switch_branches() {
        for prompt_path in [
            "harness/pr/gate-not-passed.md",
            "harness/pr/branch-not-joined.md",
            "harness/review/fix-pass.md",
            "harness/review/fix-response.md",
        ] {
            let text = read_prompt(prompt_path);
            assert!(
                text.contains("do not switch your worktree to another branch"),
                "{prompt_path} must instruct not to switch branches"
            );
            assert!(
                !text.contains("switch to it first"),
                "{prompt_path} must not instruct to switch to branch first"
            );
            assert!(
                !text.contains("Switch to your branch"),
                "{prompt_path} must not instruct to switch to branch"
            );
        }
    }
}
