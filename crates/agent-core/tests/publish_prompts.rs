//! Golden tests for publication prompts and branch-switch instructions.

#[cfg(test)]
mod publish_prompts {
    use std::path::Path;

    use agent_core::prompt::{AutopilotPrompt, PROMPTS_DIR, Variables};

    const PUBLISH_FAILED_GOLDEN: &str = include_str!("fixtures/publish-failed-lead.md");
    const NOTICE_PUBLISH_QUESTION_GOLDEN: &str = include_str!("fixtures/notice-publish-question.txt");

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
    fn publish_failed_lead_prompt_matches_golden() {
        let spec = AutopilotPrompt::PublishFailed.spec();
        let values = Variables::new()
            .with("branch", "agents/gemini-1/task-1-1.1")
            .and_then(|values| values.with("reason", "rejected by remote"))
            .expect("variables");

        let template = read_prompt(spec.path());
        let rendered = spec.render(&template, &values).expect("rendered");
        assert_eq!(rendered, PUBLISH_FAILED_GOLDEN);
    }

    #[test]
    fn notice_publish_question_prompt_matches_golden() {
        let spec = AutopilotPrompt::NoticePublishQuestion.spec();
        let values = Variables::new()
            .with("task", "task-20261121-122925-b788")
            .and_then(|values| values.with("items", "1.1"))
            .and_then(|values| values.with("branch", "agents/gemini-1/task-1-1.1"))
            .and_then(|values| values.with("reason", "cannot force update branch"))
            .expect("variables");

        let template = read_prompt(spec.path());
        let rendered = spec.render(&template, &values).expect("rendered");
        assert_eq!(rendered, NOTICE_PUBLISH_QUESTION_GOLDEN);
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
