//! The four texts that tell the requester what became of a worker's result — brought over, in conflict, given once
//! more to another worker, and returned to the requester — against their golden texts in `fixtures/worker-result/`.

#[cfg(test)]
mod worker_result_texts {
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

    /// The golden text `name` of these tests.
    fn golden(name: &str) -> String {
        std::fs::read_to_string(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/worker-result")
                .join(name),
        )
        .unwrap()
    }

    /// The text of `id` rendered with `pairs`.
    fn rendered(id: TeamPrompt, pairs: &[(&str, &str)]) -> String {
        let spec = id.spec();
        let values = pairs.iter().fold(Variables::new(), |values, (name, value)| {
            values.with(*name, *value).unwrap()
        });
        spec.render(&read_prompt(spec.path()), &values).unwrap()
    }

    #[test]
    fn the_requester_is_told_the_commit_files_and_tail_of_a_result_brought_over() {
        let text = rendered(
            TeamPrompt::WorkerResultBrought,
            &[
                ("worker", "żółw-worker"),
                ("step", "1.2"),
                ("commit", "0123456789abcdef0123456789abcdef01234567"),
                ("files", "docs/guide.md, docs/count.txt"),
                ("tail", "test -s docs/count.txt\n2 checks passed"),
            ],
        );

        assert_eq!(text, golden("worker-result-brought.txt"));
    }

    #[test]
    fn the_requester_is_told_a_commit_that_does_not_apply_and_the_files_in_conflict() {
        let text = rendered(
            TeamPrompt::WorkerResultConflict,
            &[
                ("worker", "glm-1"),
                ("step", "1.2"),
                ("commit", "0123456789abcdef0123456789abcdef01234567"),
                ("files", "docs/guide.md"),
            ],
        );

        assert_eq!(text, golden("worker-result-conflict.txt"));
    }

    #[test]
    fn the_requester_is_told_the_request_went_again_to_another_worker_and_why() {
        let text = rendered(
            TeamPrompt::WorkerGivenAgain,
            &[
                ("worker", "glm-1"),
                ("request_id", "3"),
                ("reason", "the check failed"),
                ("next", "codex-2"),
            ],
        );

        assert_eq!(text, golden("worker-given-again.txt"));
    }

    #[test]
    fn the_requester_is_told_the_step_is_back_with_every_reason_and_not_to_send_it_again() {
        let text = rendered(
            TeamPrompt::WorkerStepReturned,
            &[
                ("request_id", "3"),
                ("step", "1.2"),
                (
                    "reasons",
                    "the first result: the check failed\nthe second result: it changed a file outside the files of the request",
                ),
            ],
        );

        assert_eq!(text, golden("worker-step-returned.txt"));
    }
}
