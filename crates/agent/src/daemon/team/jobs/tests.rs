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

#[tokio::test]
async fn the_notification_that_a_team_is_finished_names_the_team_the_repository_and_the_items() {
    let texts = texts().await;
    let team = crate::daemon::team::support::crew_of(crate::daemon::team::support::CHAIN, 1, 0);

    let notification = team_done_notification(&texts, &team).unwrap();

    assert_eq!(notification.title(), format!("Team {} is finished", team.id()));
    assert!(
        notification
            .body()
            .contains("on example has finished its work: 2 items done"),
        "{}",
        notification.body()
    );
}

/// The text of `name` in `prompts/harness/team/`, without its last line break.
fn team_text(name: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("../../prompts/harness/team/{name}.md"));
    std::fs::read_to_string(path).unwrap().trim_end().to_owned()
}

/// Every way a worker can fail a request, with the file of its sentence.
const WORKER_ENDS: [(WorkerEnd, &str); 5] = [
    (WorkerEnd::Failed, "worker-failed"),
    (WorkerEnd::NoResult, "worker-no-result"),
    (WorkerEnd::NoStatus, "worker-no-status"),
    (WorkerEnd::Stopped, "worker-stopped"),
    (WorkerEnd::Hung, "worker-hung"),
];

#[tokio::test]
async fn the_reason_of_a_failed_request_is_the_sentence_of_how_the_worker_ended() {
    let texts = texts().await;

    for (how, name) in WORKER_ENDS {
        assert_eq!(failure_reason(&texts, how, None).unwrap(), team_text(name), "{how:?}");
    }
}

#[tokio::test]
async fn what_the_worker_said_follows_the_sentence_in_a_paragraph_of_its_own() {
    let texts = texts().await;

    for (how, name) in WORKER_ENDS {
        let reason = failure_reason(&texts, how, Some("Ich konnte es nicht: 失敗。")).unwrap();

        assert_eq!(
            reason,
            format!("{}\n\nIch konnte es nicht: 失敗。", team_text(name)),
            "{how:?}"
        );
    }
}

#[tokio::test]
async fn the_requester_reads_the_reason_between_the_request_and_what_to_do_now() {
    let texts = texts().await;
    let reason = failure_reason(&texts, WorkerEnd::Hung, None).unwrap();

    let (_, _, text) = notice(
        render_notice(
            &texts,
            &BusNotice::HelperFailed {
                request: request(4),
                worker: agent("glm-1"),
                reason: reason.clone(),
            },
        )
        .unwrap(),
    );

    assert!(
        text.contains(&format!("H4 to the worker glm-1 failed.\n\n{reason}\n\n")),
        "{text}"
    );
}
