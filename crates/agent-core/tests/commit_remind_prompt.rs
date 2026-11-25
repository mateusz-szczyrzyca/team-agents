//! The text of the reminder to commit: the file of the catalog, rendered with the two values the daemon gives it.

use std::path::Path;

use agent_core::prompt::{PROMPTS_DIR, TeamPrompt, Variables};

/// The reminder as the human specified it, with the values 412 and 35.
const EXPECTED: &str = "You have 412 changed lines uncommitted and no commit for 35 minutes. Commit what is finished now, in small commits. A new test that fails goes in as its `RED:` commit with its failing output. Never commit debug output (`eprintln!`, `dbg!`, probes). Do not commit only to silence this message. Then go on with your work.";

#[test]
fn the_reminder_names_the_lines_and_the_minutes_and_says_what_to_commit() {
    let spec = TeamPrompt::CommitRemind.spec();
    let text = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../")
            .join(PROMPTS_DIR)
            .join(spec.path()),
    )
    .expect("the file of the reminder exists");
    let values = Variables::new()
        .with("lines", "412")
        .and_then(|values| values.with("minutes", "35"))
        .expect("the values of the reminder");

    let rendered = spec.render(&text, &values).expect("the reminder renders");

    assert_eq!(rendered.trim_end(), EXPECTED);
    assert_eq!(spec.variables(), ["lines", "minutes"]);
}
