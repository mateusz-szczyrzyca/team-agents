//! The text of one checkpoint, rendered from its parts, against its golden text.

#[cfg(test)]
mod harness_checkpoint_state_prompt {
    use std::path::Path;

    use agent_core::prompt::{HarnessPrompt, PROMPTS_DIR, Variables};

    #[test]
    fn a_checkpoint_renders_as_its_golden_text() {
        let spec = HarnessPrompt::CheckpointState.spec();
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../");
        let text = std::fs::read_to_string(root.join(PROMPTS_DIR).join(spec.path())).unwrap();
        let values = Variables::new()
            .with(
                "meant",
                "Task: Build the thing.\nItem 1.3: Say what is meant\nR1. the first thing works",
            )
            .unwrap()
            .with(
                "done",
                "Totals: 1 commits, 1 steps\nR1 met — steps 1.1 (confirmed); commits abc1234",
            )
            .unwrap()
            .with(
                "left",
                "Totals: 0 steps, 0 findings\nnext: commit what is finished, run the gate and hand the round over",
            )
            .unwrap()
            .with("cause", "commit")
            .unwrap()
            .with("time", "2026-11-27T04:00:00Z")
            .unwrap()
            .with("head", "0123abc")
            .unwrap();

        let rendered = spec.render(&text, &values).unwrap();

        let golden = std::fs::read_to_string(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/checkpoint-state/checkpoint-state.txt"),
        )
        .unwrap();
        assert_eq!(rendered, golden);
    }
}
