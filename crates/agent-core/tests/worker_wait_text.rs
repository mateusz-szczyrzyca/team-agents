//! The text that returns to the requester a request no worker took, through the catalog.

#[cfg(test)]
mod worker_wait_text {
    use std::path::Path;

    use agent_core::prompt::{PROMPTS_DIR, TeamPrompt, Variables};

    /// The text of the entry rendered with `pairs`.
    fn rendered(pairs: &[(&str, &str)]) -> String {
        let spec = TeamPrompt::WorkerWaitReturned.spec();
        let source = std::fs::read_to_string(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../")
                .join(PROMPTS_DIR)
                .join(spec.path()),
        )
        .unwrap();
        let values = pairs.iter().fold(Variables::new(), |values, &(name, value)| {
            values.with(name, value).unwrap()
        });
        spec.render(&source, &values).unwrap()
    }

    #[test]
    fn the_requester_is_told_to_do_the_step_itself() {
        let text = rendered(&[("request", "H3"), ("step", "step 2"), ("minutes", "10")]);

        assert_eq!(
            text.trim_end(),
            "Nobody took your request H3 (step 2) for 10 minutes: no worker of the pool was free. Do the step yourself now."
        );
    }

    #[test]
    fn a_request_without_a_step_says_so() {
        let text = rendered(&[("request", "H4"), ("step", "no step"), ("minutes", "1")]);

        assert_eq!(
            text.trim_end(),
            "Nobody took your request H4 (no step) for 1 minutes: no worker of the pool was free. Do the step yourself now."
        );
    }
}
