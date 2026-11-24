//! A team is not held behind a failed member whose work its lead's branch already holds; work the lead's branch lacks is
//! told to the lead once, and when the lead ends without it the human is told, with the list.

use agent_core::ids::TaskId;
use agent_core::state::{ItemPhase, MergeMode, State};
use agent_core::status::TaskStatus;

use super::rig::{ONE, Rig, git, investigator, normal};

/// A reviewer that approves.
const APPROVE: &str = "verify:VERDICT: APPROVE\\nSUMMARY: fine";

/// What the notice of a failed member that holds a round says.
const BEHIND: &str = "behind failed member";

/// A team of a lead that runs `lead` and a member that runs `member`, started with `[replacement] mode = "off"` so that
/// the member's declared failure is final, as for an installation that keeps its ladder short.
async fn team(lead: &'static str, member: &'static str) -> (Rig, Vec<TaskId>) {
    let mut rig = Rig::new(
        &[
            investigator("inv-lead", lead),
            normal("claude-1", member),
            investigator("inv-r1", APPROVE),
            investigator("inv-r2", APPROVE),
        ],
        5,
        true,
    )
    .await;
    let mut config = std::fs::read_to_string(rig.home.config_file()).unwrap();
    config.push_str("[replacement]\nmode = \"off\"\n");
    std::fs::write(rig.home.config_file(), config).unwrap();
    rig.wake_sleeping_teams();
    rig.start().await;
    let (_, tasks) = rig
        .run(&["inv-lead", "claude-1"], ONE, MergeMode::Autopilot, Some("inv-lead"))
        .await;
    (rig, tasks)
}

/// Waits until item 1.1 is in review or later: its pull request was opened.
async fn published(rig: &Rig) -> State {
    rig.item_where("1.1", "published by the lead", |phase| {
        matches!(
            phase,
            ItemPhase::InReview { .. } | ItemPhase::Approved { .. } | ItemPhase::Done { .. }
        )
    })
    .await
}

/// The tip of the branch the failed member worked on.
fn member_tip(rig: &Rig, task: &TaskId) -> String {
    git(&rig.worktree(task), &["rev-parse", "HEAD"]).trim().to_owned()
}

// Rule: the lead ended DONE with a passed gate and the failed member's commits merged in: the round is published with no
// hand, and the member's task is closed as stopped, not failed. The case of 22.11.2026.
#[tokio::test]
async fn a_lead_that_delivered_the_work_of_a_failed_member_is_published_without_a_hand() {
    let (mut rig, tasks) = team("sleep,integrate", "fail-tool-committed").await;
    rig.status_is(&tasks[1], TaskStatus::Failed).await;

    let state = published(&rig).await;
    rig.stop().await.unwrap();

    let member = state.find_task(&tasks[1]).unwrap().member;
    assert_eq!(
        member.status(),
        TaskStatus::Stopped,
        "the member is closed as taken over"
    );
    let lead = state.find_task(&tasks[0]).unwrap().member;
    assert_eq!(
        lead.runs().len(),
        2,
        "the lead slept, was woken and delivered; nobody sent it back"
    );
    let waiting: Vec<_> = rig
        .notifications()
        .into_iter()
        .filter(|text| text.contains(BEHIND))
        .collect();
    assert!(
        waiting.is_empty(),
        "the human is not told of a round that is published: {waiting:?}"
    );
}

// Rule: a failed member with a commit the lead lacks: the lead is told which, once, takes it, and the round is published.
#[tokio::test]
async fn a_commit_the_lead_lacks_is_told_to_the_lead_once_and_the_round_goes_on_when_it_takes_it() {
    let (mut rig, tasks) = team("sleep,work,integrate", "fail-tool-committed").await;
    rig.status_is(&tasks[1], TaskStatus::Failed).await;
    let commit = member_tip(&rig, &tasks[1]);

    let state = published(&rig).await;
    let told = rig.prompt(&tasks[0], 3).await;
    rig.stop().await.unwrap();

    assert!(told.contains(&commit), "the lead is told the commit it lacks: {told}");
    let lead = state.find_task(&tasks[0]).unwrap().member;
    assert_eq!(
        lead.runs().len(),
        3,
        "told once: the lead slept, delivered without it, was told and took it"
    );
    let member = state.find_task(&tasks[1]).unwrap().member;
    assert_eq!(member.status(), TaskStatus::Stopped);
    assert!(
        rig.notifications().iter().all(|text| !text.contains(BEHIND)),
        "the human is not told when the lead takes the work"
    );
}

// Rule: the lead ends DONE again without the commit it was told of: the human is asked once, with the list, and the
// member's work is not dropped.
#[tokio::test]
async fn a_lead_that_says_done_without_the_commit_it_was_told_of_has_the_human_asked_once() {
    let (mut rig, tasks) = team("sleep,work,work", "fail-tool-committed").await;
    rig.status_is(&tasks[1], TaskStatus::Failed).await;
    let commit = member_tip(&rig, &tasks[1]);

    let notice = rig.notification_with(&commit).await;
    let state = rig.state();
    rig.stop().await.unwrap();

    assert!(notice.contains("claude-1"), "the notice names the member: {notice}");
    let asked: Vec<_> = rig
        .notifications()
        .into_iter()
        .filter(|text| text.contains(&commit))
        .collect();
    assert_eq!(asked.len(), 1, "the human is asked once: {asked:?}");
    let member = state.find_task(&tasks[1]).unwrap().member;
    assert_eq!(
        member.status(),
        TaskStatus::Failed,
        "the member is not closed over work the lead lacks"
    );
    assert!(
        !matches!(
            state.find_task(&tasks[1]).unwrap().team.items()[0].phase(),
            ItemPhase::InReview { .. } | ItemPhase::Approved { .. } | ItemPhase::Done { .. }
        ),
        "the round is not published without the member's work"
    );
}

// Rule: an uncommitted file of a failed member is told to the lead with the commit that holds the rest; when the lead
// takes the commits and ends again, the round is published and the file is still in the member's worktree.
#[tokio::test]
async fn an_uncommitted_file_of_a_failed_member_is_told_to_the_lead_and_never_dropped() {
    let (mut rig, tasks) = team("sleep,integrate,integrate", "fail-tool-committed-dirty").await;
    rig.status_is(&tasks[1], TaskStatus::Failed).await;

    let state = published(&rig).await;
    let told = rig.prompt(&tasks[0], 3).await;
    rig.stop().await.unwrap();

    assert!(told.contains("draft.txt"), "the lead is told of the file: {told}");
    assert!(
        rig.worktree(&tasks[1]).join("draft.txt").is_file(),
        "the file is still in the member's worktree"
    );
    let member = state.find_task(&tasks[1]).unwrap().member;
    assert_eq!(member.status(), TaskStatus::Stopped);
}
