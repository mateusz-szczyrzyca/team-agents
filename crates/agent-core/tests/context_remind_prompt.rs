//! The text of the reminder of a full context: the file of the catalog, rendered with the three values the daemon gives it.

use std::path::Path;

use agent_core::prompt::{PROMPTS_DIR, TeamPrompt, Variables};

/// The reminder as the human specified it, with the values 130000, 150000 and 80.
const EXPECTED: &str = "Your session holds 130000 tokens of context, 80% of its bound of 150000. Commit what is finished now: a failing new test as its `RED:` commit, no debug output. Then write in your report to the bus where you are and what is next: the next run may start a new session that sees only your commits, the list of uncommitted files and what you wrote.";

#[test]
fn the_reminder_names_the_context_the_percent_and_the_bound_and_says_what_to_write() {
    let spec = TeamPrompt::ContextRemind.spec();
    let text = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../")
            .join(PROMPTS_DIR)
            .join(spec.path()),
    )
    .expect("the file of the reminder exists");
    let values = Variables::new()
        .with("context", "130000")
        .and_then(|values| values.with("bound", "150000"))
        .and_then(|values| values.with("percent", "80"))
        .expect("the values of the reminder");

    let rendered = spec.render(&text, &values).expect("the reminder renders");

    assert_eq!(rendered.trim_end(), EXPECTED);
    assert_eq!(spec.variables(), ["context", "bound", "percent"]);
}
