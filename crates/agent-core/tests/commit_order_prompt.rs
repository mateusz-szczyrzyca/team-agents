//! The text of the order to commit: the file of the catalog, rendered with the two values the daemon gives it.

use std::path::Path;

use agent_core::prompt::{PROMPTS_DIR, TeamPrompt, Variables};

/// The order as the human specified it, with the values 400 and 21.
const EXPECTED: &str = "The harness stopped your run because 400 changed lines stayed uncommitted for 21 minutes after the reminder. Your files are as you left them. First commit what is finished: a failing new test as its `RED:` commit with its failing output, no debug output. Then go on with your work.";

#[test]
fn the_order_names_the_lines_and_the_minutes_and_says_to_commit_first() {
    let spec = TeamPrompt::CommitOrder.spec();
    let text = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../")
            .join(PROMPTS_DIR)
            .join(spec.path()),
    )
    .expect("the file of the order exists");
    let values = Variables::new()
        .with("lines", "400")
        .and_then(|values| values.with("minutes", "21"))
        .expect("the values of the order");

    let rendered = spec.render(&text, &values).expect("the order renders");

    assert_eq!(rendered.trim_end(), EXPECTED);
    assert_eq!(spec.variables(), ["lines", "minutes"]);
}
