//! Golden human question for absent crew capabilities.
use agent_core::prompt::{PROMPTS_DIR, TeamPrompt, Variables};
use std::path::Path;

#[test]
fn tier_question_names_the_wait_and_keeps_the_requirement() {
    let render = |id: TeamPrompt, values: Variables| {
        let spec = id.spec();
        let text = std::fs::read_to_string(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../")
                .join(PROMPTS_DIR)
                .join(spec.path()),
        )
        .unwrap();
        spec.render(&text, &values).unwrap()
    };
    let waiting = render(
        TeamPrompt::TierWaiting,
        Variables::new()
            .with("item", "1.1a")
            .unwrap()
            .with("tier", "investigator")
            .unwrap(),
    );
    assert_eq!(waiting, "- Item 1.1a requires investigator.\n");
    let text = render(
        TeamPrompt::NoticeTierQuestion,
        Variables::new()
            .with("task", "task-20261121-100000-abcd")
            .unwrap()
            .with("waiting", waiting.trim_end())
            .unwrap(),
    );
    assert_eq!(text, include_str!("fixtures/tier-question.md"));
}
