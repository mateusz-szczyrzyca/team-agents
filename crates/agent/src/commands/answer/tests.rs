//! File validation completes before any answer can be sent.
use super::*;
use crate::fixtures::task;
use agent_core::decide::checks::Subject;
use agent_core::questions::{Entry, Verdict};

fn grouped() -> QuestionsFile {
    QuestionsFile::new(
        "Checks [checks:v1]",
        (1..=2)
            .map(|number| {
                Entry::new(NonZeroU32::new(number).unwrap(), "Edited tests")
                    .unwrap()
                    .with_check_subjects(&[Subject::Path("tests/żółw.rs".parse().unwrap())])
                    .recommend("refuse")
                    .unwrap()
            })
            .collect(),
    )
    .unwrap()
}

#[tokio::test]
async fn checks_file_requires_every_explicit_verdict_before_delivery() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("answers.md");
    for invalid in ["ok", "", "yes", "allow except tests/unknown.rs"] {
        let mut file = grouped();
        file.set_verdict(NonZeroU32::MIN, Verdict::from_text("allow").unwrap())
            .unwrap();
        file.set_verdict(NonZeroU32::new(2).unwrap(), Verdict::from_text(invalid).unwrap())
            .unwrap();
        std::fs::write(&path, file.render()).unwrap();
        let error = answers_in(&path, &task(1)).await.expect_err(invalid);
        match &error {
            AnswerError::InvalidChecks { source, .. } => {
                assert_eq!(source.problems.len(), 1);
                assert_eq!(source.problems[0].0.get(), 2);
            }
            other => panic!("unexpected error: {other:?}"),
        }
        let message = error.to_string();
        for required in ["question 2", "allow", "refuse", "allow except"] {
            assert!(message.contains(required), "{message}");
        }
    }
    for verdict in ["allow", "refuse", "allow except tests/żółw.rs"] {
        let mut file = grouped();
        for number in [NonZeroU32::MIN, NonZeroU32::new(2).unwrap()] {
            file.set_verdict(number, Verdict::from_text(verdict).unwrap()).unwrap();
        }
        std::fs::write(&path, file.render()).unwrap();
        assert_eq!(answers_in(&path, &task(1)).await.unwrap().as_slice().len(), 2);
    }
}

#[tokio::test]
async fn legacy_questions_still_send_ok_and_partial_own_answers() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("old.md");
    let mut file = QuestionsFile::new(
        "Old checks",
        vec![
            Entry::new(NonZeroU32::MIN, "Old question")
                .unwrap()
                .recommend("refuse")
                .unwrap(),
        ],
    )
    .unwrap();
    file.set_verdict(NonZeroU32::MIN, Verdict::from_text("ok").unwrap())
        .unwrap();
    std::fs::write(&path, file.render()).unwrap();
    let list = answers_in(&path, &task(1)).await.unwrap();
    assert_eq!(list.as_slice()[0].verdict, Verdict::Accept);
}
