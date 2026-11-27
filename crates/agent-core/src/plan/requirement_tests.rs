//! The `For:` line of a step: the requirements of the item that it serves.

use super::*;

/// The id of a requirement written as text.
fn requirement(text: &str) -> RequirementId {
    text.parse().unwrap()
}

/// The requirements of the step `id` of the plan `text`.
fn named_by(text: &str, id: &str) -> Vec<RequirementId> {
    let plan = parse_plan(text).unwrap();
    plan.step(&id.parse().unwrap()).unwrap().requirements().unwrap()
}

const TIED: &str = "1. Work\n[ ] 1.1 First\n    For: R1, K2\n    Check: cargo test\n    Files: a.rs\n[ ] 1.2 Second\n";

#[test]
fn a_step_names_the_requirements_it_serves_in_its_for_line() {
    assert_eq!(named_by(TIED, "1.1"), vec![requirement("R1"), requirement("K2")]);
}

#[test]
fn the_for_line_leaves_the_files_and_the_check_of_the_step_as_they_are() {
    let plan = parse_plan(TIED).unwrap();
    let step = plan.step(&"1.1".parse().unwrap()).unwrap();
    assert_eq!(step.check(), Some(&Check::Command("cargo test".to_owned())));
    assert_eq!(step.files().len(), 1);
}

#[test]
fn a_step_with_no_for_line_names_no_requirement() {
    assert_eq!(named_by(TIED, "1.2"), Vec::new());
}

#[test]
fn a_plan_with_for_lines_is_written_back_and_read_again_equal() {
    let plan = parse_plan(TIED).unwrap();
    assert_eq!(parse_plan(&plan.to_text()), Ok(plan));
}

#[test]
fn the_written_plan_keeps_the_for_line() {
    let text = parse_plan(TIED).unwrap().to_text();
    assert!(text.lines().any(|line| line == "    For: R1, K2"), "{text}");
}

#[test]
fn a_plan_is_tied_to_requirements_when_a_step_has_a_for_line() {
    assert!(parse_plan(TIED).unwrap().ties_requirements());
    assert!(
        !parse_plan("1. Work\n[ ] 1.1 First\n    Check: reviewer\n")
            .unwrap()
            .ties_requirements()
    );
}

#[test]
fn an_empty_for_line_is_refused_at_its_line() {
    assert_eq!(
        parse_plan("1. Work\n[ ] 1.1 First\n    For:\n").unwrap_err(),
        PlanError::EmptyFor {
            line: NonZeroUsize::new(3).unwrap()
        }
    );
}

#[test]
fn a_for_line_that_names_no_requirement_is_refused_with_the_text() {
    let error = parse_plan("1. Work\n[ ] 1.1 First\n    For: R1, X2\n").unwrap_err();
    assert_eq!(
        error,
        PlanError::BadRequirement {
            line: NonZeroUsize::new(3).unwrap(),
            problem: InvalidRequirementId { value: "X2".to_owned() },
        }
    );
}

#[test]
fn a_second_for_line_in_a_step_is_refused() {
    assert_eq!(
        parse_plan("1. Work\n[ ] 1.1 First\n    For: R1\n    For: R2\n").unwrap_err(),
        PlanError::DuplicatePart {
            line: NonZeroUsize::new(4).unwrap(),
            part: StepPart::For
        }
    );
}
