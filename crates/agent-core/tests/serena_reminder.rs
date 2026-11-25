//! The reminder of Serena is a value of the catalog with the count of the session (see `prompt_tests.rs`).

use agent_core::prompt::HarnessPrompt;

#[test]
fn the_reminder_is_an_entry_of_the_catalog_of_the_model() {
    let spec = HarnessPrompt::SerenaRemind.spec();

    assert_eq!(spec.path(), "harness/run/serena-remind.md");
    assert_eq!(spec.variables(), ["count"]);
}
