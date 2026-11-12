use std::num::NonZeroU64;
use std::path::Path;

use agent_core::config::Language;

use super::*;

async fn texts() -> TeamPrompts {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../prompts");
    TeamPrompts::load(&dir, &Language::default()).await.unwrap()
}

fn agent(name: &str) -> AgentName {
    name.parse().unwrap()
}

fn request(number: u64) -> RequestId {
    RequestId::new(NonZeroU64::new(number).unwrap())
}

fn notice(outgoing: Outgoing) -> (NoticeKind, Vec<(String, String)>, String) {
    let Outgoing::Notice { kind, fields, text } = outgoing else {
        panic!("a notice was expected: {outgoing:?}")
    };
    let fields = fields
        .into_iter()
        .map(|(name, value)| (name.as_str().to_owned(), value))
        .collect();
    (kind, fields, text)
}

fn pair(name: &str, value: &str) -> (String, String) {
    (name.to_owned(), value.to_owned())
}

#[tokio::test]
async fn a_member_that_finished_an_item_is_a_member_finished_notice_with_the_item_and_the_branch() {
    let texts = texts().await;

    let (kind, fields, text) = notice(
        render_notice(
            &texts,
            &BusNotice::MemberFinished {
                member: agent("claude-1"),
                item: Some("1.1".parse().unwrap()),
                branch: "agents/claude-1/task-x-1.1".parse().unwrap(),
            },
        )
        .unwrap(),
    );

    assert_eq!(kind, NoticeKind::MemberFinished);
    assert_eq!(
        fields,
        [
            pair("branch", "agents/claude-1/task-x-1.1"),
            pair("item", "1.1"),
            pair("member", "claude-1")
        ]
    );
    assert!(text.contains("claude-1 reported DONE on item 1.1"), "{text}");
    assert!(text.contains("`agents/claude-1/task-x-1.1`"), "{text}");
}

#[tokio::test]
async fn a_member_that_finished_work_without_an_item_has_the_plain_text_and_no_item_detail() {
    let texts = texts().await;

    let (kind, fields, text) = notice(
        render_notice(
            &texts,
            &BusNotice::MemberFinished {
                member: agent("claude-1"),
                item: None,
                branch: "agents/claude-1/task-x".parse().unwrap(),
            },
        )
        .unwrap(),
    );

    assert_eq!(kind, NoticeKind::MemberFinished);
    assert_eq!(
        fields,
        [pair("branch", "agents/claude-1/task-x"), pair("member", "claude-1")]
    );
    assert!(text.contains("reported DONE with the work you gave it"), "{text}");
}

#[tokio::test]
async fn a_member_that_failed_is_a_member_failed_notice_with_what_it_said() {
    let texts = texts().await;

    let (kind, fields, text) = notice(
        render_notice(
            &texts,
            &BusNotice::MemberFailed {
                member: agent("claude-1"),
                message: "I could not do it.".to_owned(),
            },
        )
        .unwrap(),
    );

    assert_eq!(kind, NoticeKind::MemberFailed);
    assert_eq!(fields, [pair("member", "claude-1")]);
    assert!(text.contains("I could not do it."), "{text}");
}

#[tokio::test]
async fn a_member_without_a_status_is_a_member_no_status_notice() {
    let texts = texts().await;

    let (kind, fields, text) = notice(
        render_notice(
            &texts,
            &BusNotice::MemberNoStatus {
                member: agent("claude-1"),
            },
        )
        .unwrap(),
    );

    assert_eq!(kind, NoticeKind::MemberNoStatus);
    assert_eq!(fields, [pair("member", "claude-1")]);
    assert!(
        text.contains("claude-1 ended its turn without a `STATUS:` line"),
        "{text}"
    );
}

#[tokio::test]
async fn the_result_of_a_worker_is_a_helper_result_notice_that_names_the_request_and_says_done() {
    let texts = texts().await;

    let (kind, fields, text) = notice(
        render_notice(
            &texts,
            &BusNotice::HelperResult {
                request: request(3),
                worker: agent("glm-1"),
                summary: "Result: three files.".to_owned(),
            },
        )
        .unwrap(),
    );

    assert_eq!(kind, NoticeKind::HelperResult);
    assert_eq!(
        fields,
        [pair("request", "H3"), pair("status", "done"), pair("worker", "glm-1")]
    );
    assert!(text.contains("request H3"), "{text}");
    assert!(text.contains("Result: three files."), "{text}");
}

#[tokio::test]
async fn a_request_a_worker_could_not_do_is_a_helper_result_notice_that_says_failed() {
    let texts = texts().await;

    let (kind, fields, text) = notice(
        render_notice(
            &texts,
            &BusNotice::HelperFailed {
                request: request(3),
                worker: agent("glm-1"),
                reason: "the worker ended without a status".to_owned(),
            },
        )
        .unwrap(),
    );

    assert_eq!(kind, NoticeKind::HelperResult);
    assert_eq!(
        fields,
        [pair("request", "H3"), pair("status", "failed"), pair("worker", "glm-1")]
    );
    assert!(text.contains("the worker ended without a status"), "{text}");
}
