//! The question to the human after the last step of the ladder, against its golden text in `fixtures/last-step/`.

#[cfg(test)]
mod harness_last_step_question {
    use std::path::Path;

    use agent_core::prompt::{HarnessPrompt, PROMPTS_DIR, Variables};
    use agent_core::questions::QuestionsFile;

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

    /// The question rendered with the values of these tests.
    fn rendered() -> String {
        let spec = HarnessPrompt::NoticeLastStep.spec();
        let values = Variables::new()
            .with("task", "task-20261121-101500-k2m4")
            .unwrap()
            .with("agent", "claude-2")
            .unwrap()
            .with("held", "claude-1, żółw-agent")
            .unwrap()
            .with("reason", "no_status")
            .unwrap();
        spec.render(&read_prompt(spec.path()), &values).unwrap()
    }

    #[test]
    fn the_human_is_asked_whether_to_start_the_task_once_more_or_stop_it() {
        let golden = std::fs::read_to_string(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/last-step/notice-last-step.txt"),
        )
        .unwrap();

        assert_eq!(rendered(), golden);
    }

    #[test]
    fn the_question_is_a_questions_file_the_harness_reads() {
        let file = QuestionsFile::parse(&rendered()).expect("a questions file of one question");

        assert_eq!(file.entries().len(), 1);
    }
}
