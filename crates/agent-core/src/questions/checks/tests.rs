//! Strict checks verdicts survive a questions-file round trip; legacy questions keep their grammar.
use super::*;
use crate::questions::{QuestionsFile, Verdict};
use std::num::NonZeroU32;

fn subjects() -> Vec<Subject> {
    ["tests/żółw.rs", "tests/with space.rs"]
        .map(|p| Subject::Path(p.parse().unwrap()))
        .to_vec()
}

fn file() -> QuestionsFile {
    QuestionsFile::new(
        "Checks [checks:v1]",
        vec![
            Entry::new(NonZeroU32::MIN, "Edited tests")
                .unwrap()
                .with_check_subjects(&subjects())
                .recommend("refuse")
                .unwrap(),
        ],
    )
    .unwrap()
}

#[test]
fn group_membership_and_all_three_verdicts_round_trip() {
    for (answer, expected) in [
        ("allow", ChecksVerdict::Allow),
        ("refuse", ChecksVerdict::Refuse),
        (
            "allow except tests/żółw.rs tests/with space.rs",
            ChecksVerdict::Except(vec!["tests/żółw.rs".into(), "tests/with space.rs".into()]),
        ),
    ] {
        let mut file = file();
        file.set_verdict(NonZeroU32::MIN, Verdict::from_text(answer).unwrap())
            .unwrap();
        let parsed = QuestionsFile::parse(&file.render()).unwrap();
        let entry = parsed.entry(NonZeroU32::MIN).unwrap();
        for subject in subjects() {
            assert!(entry.checks_contains(&subject));
        }
        assert!(!entry.checks_contains(&Subject::NoTests));
        assert_eq!(entry.checks_verdict().unwrap(), Some(expected));
        assert_eq!(parsed.answers().unwrap().len(), 1);
    }
}

#[test]
fn invalid_checks_verdicts_are_rejected_by_the_shared_answer_boundary() {
    for answer in [
        "ok",
        "",
        "yes",
        "allow it",
        "allow except",
        "allow except tests/unknown.rs",
        "refuse now",
    ] {
        let mut file = file();
        file.set_verdict(NonZeroU32::MIN, Verdict::from_text(answer).unwrap())
            .unwrap();
        let parsed = QuestionsFile::parse(&file.render()).unwrap();
        assert_eq!(
            parsed.entries()[0].checks_verdict(),
            Err(InvalidChecksVerdict),
            "{answer}"
        );
        let error = parsed.answers().expect_err(answer);
        assert_eq!(error.problems[0].0, NonZeroU32::MIN);
        assert!(error.problems[0].1.to_string().contains("allow except"));
    }
}

#[test]
fn legacy_ok_and_own_answers_keep_their_existing_meaning() {
    let mut file = QuestionsFile::new(
        "Legacy",
        vec![
            Entry::new(NonZeroU32::MIN, "Old question")
                .unwrap()
                .recommend("refuse")
                .unwrap(),
        ],
    )
    .unwrap();
    for answer in ["ok", "ALLOW it, please"] {
        file.set_verdict(NonZeroU32::MIN, Verdict::from_text(answer).unwrap())
            .unwrap();
        let parsed = QuestionsFile::parse(&file.render()).unwrap();
        assert_eq!(parsed.entries()[0].checks_verdict().unwrap(), None);
        assert_eq!(parsed.answers().unwrap().len(), 1);
    }
}

#[test]
fn no_tests_cannot_be_named_as_a_path_exception() {
    let entry = Entry::new(NonZeroU32::MIN, "No tests")
        .unwrap()
        .with_check_subjects(&[Subject::NoTests]);
    let mut file = QuestionsFile::new("Checks [checks:v1]", vec![entry]).unwrap();
    file.set_verdict(NonZeroU32::MIN, Verdict::from_text("allow except no-tests").unwrap())
        .unwrap();
    assert_eq!(file.entries()[0].checks_verdict(), Err(InvalidChecksVerdict));
}

#[test]
fn a_marked_checks_file_cannot_silently_treat_an_unlabelled_question_as_legacy() {
    let mut file = QuestionsFile::new(
        "Checks [checks:v1]",
        vec![
            Entry::new(NonZeroU32::MIN, "Missing membership")
                .unwrap()
                .recommend("refuse")
                .unwrap(),
        ],
    )
    .unwrap();
    file.set_verdict(NonZeroU32::MIN, Verdict::from_text("allow").unwrap())
        .unwrap();
    let error = file.answers().unwrap_err();
    assert_eq!(
        error.problems,
        [(
            NonZeroU32::MIN,
            crate::questions::AnswerProblem::ChecksVerdict(InvalidChecksVerdict)
        )]
    );
}

fn commit_subject(digit: char, path: &str) -> Subject {
    Subject::Commit {
        commit: digit.to_string().repeat(40).parse().unwrap(),
        path: path.parse().unwrap(),
    }
}

#[test]
fn a_commit_subject_is_told_apart_from_other_commits_and_from_the_same_path_in_a_group() {
    let (first, second) = (commit_subject('a', "tests/a.rs"), commit_subject('b', "tests/a.rs"));
    let mut file = QuestionsFile::new(
        "Checks [checks:v1]",
        vec![
            Entry::new(NonZeroU32::MIN, "First commit")
                .unwrap()
                .with_check_subjects(std::slice::from_ref(&first)),
            Entry::new(NonZeroU32::new(2).unwrap(), "Second commit")
                .unwrap()
                .with_check_subjects(std::slice::from_ref(&second)),
        ],
    )
    .unwrap();
    file.set_verdict(NonZeroU32::MIN, Verdict::from_text("allow").unwrap())
        .unwrap();
    file.set_verdict(NonZeroU32::new(2).unwrap(), Verdict::from_text("refuse").unwrap())
        .unwrap();
    let parsed = QuestionsFile::parse(&file.render()).unwrap();

    let allowed = parsed
        .allowed_check_subjects(&[
            first.clone(),
            second.clone(),
            Subject::Path("tests/a.rs".parse().unwrap()),
        ])
        .unwrap();

    assert_eq!(allowed, HashSet::from([first]));
}

#[test]
fn a_commit_subject_in_a_path_with_spaces_survives_a_round_trip_of_the_file() {
    let subject = commit_subject('c', "tests/with space.rs");
    let mut file = QuestionsFile::new(
        "Checks [checks:v1]",
        vec![
            Entry::new(NonZeroU32::MIN, "A commit")
                .unwrap()
                .with_check_subjects(std::slice::from_ref(&subject)),
        ],
    )
    .unwrap();
    file.set_verdict(NonZeroU32::MIN, Verdict::from_text("allow").unwrap())
        .unwrap();

    let parsed = QuestionsFile::parse(&file.render()).unwrap();

    assert!(parsed.entries()[0].checks_contains(&subject));
    assert_eq!(
        parsed
            .allowed_check_subjects(&[subject.clone(), Subject::NoTests])
            .unwrap(),
        HashSet::from([subject])
    );
}
