//! The cause of a refused feature number is the `source()` of the task list error, so a reporter that walks the chain
//! (`Error::source`) can say why: `NumberProblem`, and for a number out of range std's `ParseIntError` below it. The
//! tests walk the chain through the public API only: a field of the error being equal to what is expected does not show
//! that the link exists.

use std::error::Error;
use std::num::{IntErrorKind, ParseIntError};

use agent_core::task_list::{NumberProblem, TaskList, TaskListError};

/// The `NumberProblem` that `err` has as its source, if it has one. A helper may not panic (only a test may), so it
/// answers with an `Option`.
fn number_problem(err: &TaskListError) -> Option<&NumberProblem> {
    err.source()?.downcast_ref::<NumberProblem>()
}

/// The `ParseIntError` below a `NumberProblem`, if it has one.
fn parse_int_error(problem: &NumberProblem) -> Option<&ParseIntError> {
    problem.source()?.downcast_ref::<ParseIntError>()
}

#[test]
fn a_feature_number_of_the_wrong_shape_has_the_shape_problem_as_its_source() {
    let err = TaskList::parse("01. Title\n").unwrap_err();

    assert!(matches!(err, TaskListError::FeatureNumber { .. }));
    let problem = number_problem(&err).expect("the source of the error is a NumberProblem");
    assert_eq!(problem, &NumberProblem::Shape);
    assert!(problem.source().is_none(), "a wrong shape has no cause below it");
}

#[test]
fn a_feature_number_out_of_range_has_the_range_problem_and_the_parse_error_in_its_chain() {
    for (text, kind) in [
        ("0. Title\n", IntErrorKind::Zero),
        ("65536. Title\n", IntErrorKind::PosOverflow),
    ] {
        let err = TaskList::parse(text).unwrap_err();

        assert!(matches!(err, TaskListError::FeatureNumber { .. }), "{text:?}");
        let problem = number_problem(&err).expect("the source of the error is a NumberProblem");
        assert!(matches!(problem, NumberProblem::Range(_)), "{text:?}");
        assert_eq!(
            parse_int_error(problem)
                .expect("the source of a number out of range is a ParseIntError")
                .kind(),
            &kind,
            "{text:?}"
        );
    }
}

#[test]
fn a_dependency_of_the_wrong_shape_has_the_shape_problem_as_its_source() {
    let err = TaskList::parse("1. First\n2. Second\nDepends on: 1, x\n").unwrap_err();

    assert!(matches!(err, TaskListError::MalformedDependsOn { .. }));
    let problem = number_problem(&err).expect("the source of the error is a NumberProblem");
    assert_eq!(problem, &NumberProblem::Shape);
    assert!(problem.source().is_none(), "a wrong shape has no cause below it");
}

#[test]
fn a_dependency_out_of_range_has_the_range_problem_and_the_parse_error_in_its_chain() {
    for (text, kind) in [("0", IntErrorKind::Zero), ("65536", IntErrorKind::PosOverflow)] {
        let err = TaskList::parse(&format!("1. First\n2. Second\nDepends on: {text}\n")).unwrap_err();

        assert!(matches!(err, TaskListError::MalformedDependsOn { .. }), "{text:?}");
        let problem = number_problem(&err).expect("the source of the error is a NumberProblem");
        assert!(matches!(problem, NumberProblem::Range(_)), "{text:?}");
        assert_eq!(
            parse_int_error(problem)
                .expect("the source of a number out of range is a ParseIntError")
                .kind(),
            &kind,
            "{text:?}"
        );
    }
}

#[test]
fn an_empty_depends_on_has_no_source() {
    let err = TaskList::parse("1. First\n2. Second\nDepends on: ,\n").unwrap_err();

    assert!(matches!(err, TaskListError::MalformedDependsOn { .. }));
    assert!(
        err.source().is_none(),
        "there is no word to blame, so there is no cause"
    );
}
