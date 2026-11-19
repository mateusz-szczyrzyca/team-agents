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
