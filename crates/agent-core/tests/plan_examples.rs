//! The three examples of `docs/plan.md` are fixtures, and the page shows each of them as it is.
//!
//! The page is the form's documentation; the fixtures are what its readers paste, so they cannot drift apart without
//! this test saying so.
use agent_core::plan::{Plan, StepState, parse_plan};

/// The page that documents the form of a plan.
const PAGE: &str = include_str!("../../../docs/plan.md");

/// A fixture of one example of the page.
type Fixture = (&'static str, &'static str);

/// The fixtures of the three examples: a code change, a document and a set of images.
const FIXTURES: [Fixture; 3] = [
    ("plan-code.md", include_str!("fixtures/plan-code.md")),
    ("plan-document.md", include_str!("fixtures/plan-document.md")),
    ("plan-images.md", include_str!("fixtures/plan-images.md")),
];

/// Every example parses, and each is shown on the page as the fixture has it.
#[test]
fn the_three_examples_of_the_page_parse_and_are_their_fixtures() {
    for (name, fixture) in FIXTURES {
        let plan = parse_plan(fixture).expect("the example of the page parses");
        assert!(!plan.stages().is_empty(), "{name}");
        assert!(
            plan.stages()
                .iter()
                .all(|stage| stage.steps().iter().all(|step| step.check().is_some())),
            "{name}: every step of every example names its check"
        );
        let fence = format!("```text\n{fixture}```");
        assert!(PAGE.contains(&fence), "{name} is not on the page as it is");
    }
}

/// The examples cover what a plan of any work looks like: the states, and all three kinds of check.
#[test]
fn the_examples_show_every_state_and_every_kind_of_check() {
    let mut states = Vec::new();
    let mut kinds = Vec::new();
    for (_, fixture) in FIXTURES {
        let plan: Plan = parse_plan(fixture).expect("the example of the page parses");
        for step in plan.steps() {
            if !states.contains(&step.state()) {
                states.push(step.state());
            }
            let kind = match step.check() {
                Some(agent_core::plan::Check::Command(_)) => "command",
                Some(agent_core::plan::Check::Exists(_)) => "exists",
                Some(agent_core::plan::Check::Reviewer) => "reviewer",
                None => "none",
            };
            if !kinds.contains(&kind) {
                kinds.push(kind);
            }
        }
    }
    for state in [StepState::Todo, StepState::InHand, StepState::Done, StepState::GivenUp] {
        assert!(states.contains(&state), "{state:?} is in no example");
    }
    assert!(kinds.contains(&"command") && kinds.contains(&"exists") && kinds.contains(&"reviewer"));
}
