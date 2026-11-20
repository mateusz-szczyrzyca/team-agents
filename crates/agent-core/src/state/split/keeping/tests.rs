use super::*;

#[test]
fn proposal_input_corrections_and_one_pending_question_survive_restarts() {
    let mut keeping = ProposalKeeping::new("malformed json".into()).expect("bounded input");
    assert_eq!(keeping.text(), "malformed json");
    assert_eq!(keeping.corrections(), 0);
    assert_eq!(keeping.stage(), &ProposalStage::Pending);
    let fault = ProposalFault::MissingChecks {
        part: "b".to_owned().try_into().expect("key"),
    };
    keeping.send_back(fault.clone());
    let mut restored: ProposalKeeping =
        serde_json::from_value(serde_json::to_value(&keeping).expect("write")).expect("restart");
    assert_eq!(restored, keeping);
    assert_eq!(restored.corrections(), 1);
    assert_eq!(restored.stage(), &ProposalStage::Correction { fault });
    restored
        .resubmit("corrected proposal — 日本語".into())
        .expect("replacement");
    assert_eq!(restored.text(), "corrected proposal — 日本語");
    assert_eq!(restored.corrections(), 1);
    assert_eq!(restored.stage(), &ProposalStage::Pending);
    let question = NonZeroU32::new(1).expect("question");
    restored.asked(question);
    let restarted: ProposalKeeping =
        serde_json::from_value(serde_json::to_value(&restored).expect("write")).expect("restart");
    assert_eq!(restarted, restored);
    assert_eq!(restarted.stage(), &ProposalStage::Asked { question });
    let mut saturated: ProposalKeeping = serde_json::from_value(serde_json::json!({
        "text":"input", "corrections":u32::MAX, "stage":"pending"
    }))
    .expect("max count");
    saturated.send_back(ProposalFault::Unreadable { part: None });
    assert_eq!(saturated.corrections(), u32::MAX);
}

#[test]
fn proposal_storage_bounds_new_replaced_and_restored_input_without_losing_old_state() {
    let bound = crate::questions::MAX_QUESTIONS_BYTES;
    let mut keeping = ProposalKeeping::new("x".repeat(bound)).expect("exact bound");
    let snapshot = keeping.clone();
    assert!(matches!(
        ProposalKeeping::new("x".repeat(bound + 1)),
        Err(ReadProposalError::TooLarge)
    ));
    assert!(matches!(
        keeping.resubmit("x".repeat(bound + 1)),
        Err(ReadProposalError::TooLarge)
    ));
    assert_eq!(keeping, snapshot);
    for value in [
        serde_json::json!({"text":"x".repeat(bound + 1),"corrections":0,"stage":"pending"}),
        serde_json::json!({"text":"x","corrections":0,"stage":{"asked":{"question":0}}}),
        serde_json::json!({"text":"x","corrections":0,"stage":"pending","extra":1}),
    ] {
        assert_eq!(
            serde_json::from_value::<ProposalKeeping>(value)
                .expect_err("invalid record")
                .classify(),
            serde_json::error::Category::Data
        );
    }
}
