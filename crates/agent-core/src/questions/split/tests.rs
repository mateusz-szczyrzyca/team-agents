use super::*;
use crate::questions::{MAX_QUESTIONS_BYTES, QuestionsFile};
use crate::state::split::PartField;

fn verdict(text: &str) -> Verdict {
    Verdict::from_text(text).expect("valid verdict").expect("answer")
}

fn approval(parts: serde_json::Value) -> Verdict {
    verdict(&format!(
        "split\n```json\n{}\n```",
        serde_json::json!({"parent":"1.1","parts":parts})
    ))
}

fn part(title: &str, dependencies: &[&str]) -> serde_json::Value {
    serde_json::json!({"title":title,"delivers":"A separately reviewable document","checks":"Reviewer verifies the examples","depends_on":dependencies})
}

#[test]
fn explicit_split_verdict_reads_two_and_three_parts_and_survives_questions_rendering() {
    for parts in [
        serde_json::json!([part("Résumé", &[]), part("Publish", &["a"])]),
        serde_json::json!([part("Draft", &[]), part("Review", &["a"]), part("Publish", &["a", "b"])]),
    ] {
        let answer = approval(parts.clone());
        let plan = read_split(&answer).expect("valid plan").expect("explicit approval");
        assert_eq!(
            serde_json::to_value(&plan).expect("serialize"),
            serde_json::json!({"parent":"1.1","parts":parts})
        );
        let text = format!(
            "# Questions\n\n## 1. Split?\n\n**Recommended:** a)\n\n**Verdict:** {}\n\n**ANSWERED**\n",
            match &answer {
                Verdict::Own(own) => own.as_str(),
                Verdict::Accept => "ok",
            }
        );
        let file = QuestionsFile::parse(&text).expect("questions parse");
        assert_eq!(
            QuestionsFile::parse(&file.render()).expect("rendered questions parse"),
            file
        );
        assert!(file.render().contains("Résumé") || file.render().contains("Publish"));
    }
}

#[test]
fn free_split_answers_and_accepting_the_recommendation_do_not_record_parts() {
    for answer in [
        Verdict::Accept,
        verdict("split"),
        verdict("Please split into two parts"),
        verdict("split\nThe first part is the document"),
    ] {
        assert_eq!(read_split(&answer).expect("unstructured answer"), None);
    }
}

#[test]
fn explicit_split_faults_keep_the_validation_cause_and_actionable_message() {
    let mut empty_checks = part("Draft", &[]);
    empty_checks["checks"] = serde_json::json!(" ");
    let cases = [
        (
            serde_json::json!([empty_checks, part("Publish", &[])]),
            "part a has no checks",
        ),
        (
            serde_json::json!([part("Draft", &["c"]), part("Publish", &[])]),
            "part a depends on unknown part c",
        ),
        (
            serde_json::json!([part("Draft", &["b"]), part("Publish", &["a"])]),
            "split dependencies contain a cycle",
        ),
        (
            serde_json::json!([part("Only", &[])]),
            "a split needs 2 through 26 parts, got 1",
        ),
    ];
    for (parts, message) in cases {
        let error = read_split(&approval(parts)).expect_err("invalid plan");
        let ReadSplitError::Invalid { source } = error else {
            panic!("expected plan validation, got {error:?}");
        };
        assert_eq!(source.to_string(), message);
    }
    assert!(matches!(
        read_split(&approval(
            serde_json::json!([{"title":"Draft","delivers":"Doc","checks":"","depends_on":[]},part("Publish", &[])])
        )),
        Err(ReadSplitError::Invalid {
            source: SplitError::Empty {
                field: PartField::Checks,
                ..
            }
        })
    ));
}

#[test]
fn explicit_split_rejects_bad_fences_invalid_json_extra_fields_and_oversized_answers() {
    for text in ["split\n```json\n{}", "split\n```json\n{}\n```\nextra"] {
        assert!(matches!(read_split(&verdict(text)), Err(ReadSplitError::Shape)));
    }
    for text in [
        "split\n```json\nnot json\n```",
        "split\n```json\n{\"parent\":\"1.1\",\"parts\":[],\"extra\":1}\n```",
    ] {
        assert!(matches!(read_split(&verdict(text)), Err(ReadSplitError::Json { .. })));
    }
    let huge = format!("split\n```json\n{}\n```", " ".repeat(MAX_QUESTIONS_BYTES));
    assert!(matches!(read_split(&verdict(&huge)), Err(ReadSplitError::TooLarge)));
}
