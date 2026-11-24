//! A team is not held behind a failed member whose work its lead's branch already holds; work the lead's branch lacks is
//! told to the lead once, and when the lead ends without it the human is told, with the list.

use agent_core::ids::TaskId;
use agent_core::protocol::{Client, Command, ControlRequest, Reply, SayRequest, Target};
use agent_core::state::{ItemPhase, MergeMode, State};
use agent_core::status::TaskStatus;

use super::rig::{ONE, PARALLEL, Rig, git, investigator, normal};
use crate::client::Connection;

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

// Rule: a member that failed without a single tool call did no work the lead could have taken: it is not released
// although the lead delivered, and the human is told, as before.
#[tokio::test]
async fn a_failed_member_that_called_no_tool_keeps_holding_the_round_when_the_lead_delivered() {
    let (mut rig, tasks) = team("sleep,work", "fail").await;
    rig.status_is(&tasks[1], TaskStatus::Failed).await;

    let notice = rig.notification_with("behind failed member").await;
    let state = rig.state();
    rig.stop().await.unwrap();

    assert!(notice.contains("claude-1"), "the notice names the member: {notice}");
    let member = state.find_task(&tasks[1]).unwrap().member;
    assert_eq!(member.status(), TaskStatus::Failed, "the member is not closed");
    assert!(
        !matches!(
            state.find_task(&tasks[1]).unwrap().team.items()[0].phase(),
            ItemPhase::InReview { .. } | ItemPhase::Approved { .. } | ItemPhase::Done { .. }
        ),
        "the round is not published behind a member that did nothing"
    );
}

// Rule: the release is a change of the state, not of the daemon's memory: a daemon restarted while the lead sleeps behind
// the failed member still publishes the round when the lead delivers, and the member stays closed as taken over.
#[tokio::test]
async fn a_release_of_a_failed_member_survives_a_restart_of_the_daemon() {
    let (mut rig, tasks) = team("sleep,integrate", "fail-tool-committed").await;
    rig.status_is(&tasks[1], TaskStatus::Failed).await;
    rig.stop().await.unwrap();
    rig.start().await;

    let state = published(&rig).await;
    let after_restart = rig.state();
    rig.stop().await.unwrap();

    let member = state.find_task(&tasks[1]).unwrap().member;
    assert_eq!(member.status(), TaskStatus::Stopped);
    assert_eq!(
        after_restart.find_task(&tasks[1]).unwrap().member.status(),
        TaskStatus::Stopped
    );
    assert!(
        rig.notifications().iter().all(|text| !text.contains(BEHIND)),
        "the human is not told of a round that is published"
    );
}

/// A team of a lead that runs `lead` and two members that run `member`, each with an item of its own, started with
/// `[replacement] mode = "off"`.
async fn team_of_two(lead: &'static str, member: &'static str) -> (Rig, Vec<TaskId>) {
    let mut rig = Rig::new(
        &[
            investigator("inv-lead", lead),
            normal("claude-1", member),
            normal("claude-2", member),
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
        .run(
            &["inv-lead", "claude-1", "claude-2"],
            PARALLEL,
            MergeMode::Autopilot,
            Some("inv-lead"),
        )
        .await;
    (rig, tasks)
}

/// Lets the lead, which holds in its first run, go on.
fn release_lead(rig: &Rig, lead: &TaskId) {
    std::fs::write(rig.worktree(lead).join(".fake-continue"), "").unwrap();
}

/// Asks the daemon something a few times, so that it looks at its teams, and lets what it started get going.
async fn look(rig: &Rig) {
    for _ in 0..10 {
        Connection::connect(&rig.home.socket_file())
            .await
            .unwrap()
            .request(
                Client::Cli,
                Command::Status(agent_core::protocol::StatusRequest { target: None }),
            )
            .await
            .unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
}

// Rule: two failed members whose work the lead lacks are told to it in one briefing: the second does not replace the
// first, and nothing is published until the lead has been told of both.
#[tokio::test]
async fn two_failed_members_with_uncommitted_files_are_both_told_to_the_lead_in_one_briefing() {
    let (mut rig, tasks) = team_of_two("hold,integrate,integrate", "fail-tool-committed-dirty-named").await;
    rig.status_is(&tasks[1], TaskStatus::Failed).await;
    rig.status_is(&tasks[2], TaskStatus::Failed).await;
    release_lead(&rig, &tasks[0]);

    let told = rig.prompt(&tasks[0], 3).await;
    rig.item_where("1.1", "published by the lead", |phase| {
        matches!(
            phase,
            ItemPhase::InReview { .. } | ItemPhase::Approved { .. } | ItemPhase::Done { .. }
        )
    })
    .await;
    let state = rig
        .item_where("2.1", "published by the lead", |phase| {
            matches!(
                phase,
                ItemPhase::InReview { .. } | ItemPhase::Approved { .. } | ItemPhase::Done { .. }
            )
        })
        .await;
    rig.stop().await.unwrap();

    for member in [&tasks[1], &tasks[2]] {
        let worktree = rig.worktree(member);
        assert!(
            told.contains(worktree.to_str().unwrap()),
            "the lead is told of the work in {}: {told}",
            worktree.display()
        );
        assert_eq!(state.find_task(member).unwrap().member.status(), TaskStatus::Stopped);
    }
    assert_eq!(
        state.find_task(&tasks[0]).unwrap().member.runs().len(),
        3,
        "one briefing for both: the lead held, integrated, and was told once"
    );
}

// Rule: a member the human stopped has its uncommitted file told to the lead before the round is published: stopping a
// member does not drop what it left in its worktree.
#[tokio::test]
async fn a_stopped_member_with_an_uncommitted_file_is_told_to_the_lead_before_the_round_is_published() {
    let (mut rig, tasks) = team("hold,integrate,integrate", "fail-tool-committed-dirty").await;
    rig.status_is(&tasks[1], TaskStatus::Failed).await;
    Connection::connect(&rig.home.socket_file())
        .await
        .unwrap()
        .request(
            Client::Cli,
            Command::Stop(ControlRequest {
                target: Target::Task(tasks[1].clone()),
            }),
        )
        .await
        .unwrap();
    rig.status_is(&tasks[1], TaskStatus::Stopped).await;
    release_lead(&rig, &tasks[0]);

    let state = published(&rig).await;
    assert_eq!(
        state.find_task(&tasks[0]).unwrap().member.runs().len(),
        3,
        "the lead was told once after it delivered, and took it"
    );
    let told = rig.prompt(&tasks[0], 3).await;
    rig.stop().await.unwrap();

    assert!(told.contains("draft.txt"), "the lead is told of the file: {told}");
    assert!(rig.worktree(&tasks[1]).join("draft.txt").is_file());
}

// Rule: a member that was started in a new session once and is then stopped by the human has its uncommitted file told to
// the lead like any other stopped member: only a member the ladder gave to the lead is not looked at again.
#[tokio::test]
async fn a_renewed_member_the_human_stopped_has_its_uncommitted_file_told_to_the_lead() {
    let (mut rig, tasks) = team(
        "hold,integrate,integrate",
        "nostatus,nostatus,nostatus,fail-tool-committed-dirty",
    )
    .await;
    rig.status_is(&tasks[1], TaskStatus::Failed).await;
    Connection::connect(&rig.home.socket_file())
        .await
        .unwrap()
        .request(
            Client::Cli,
            Command::Stop(ControlRequest {
                target: Target::Task(tasks[1].clone()),
            }),
        )
        .await
        .unwrap();
    rig.status_is(&tasks[1], TaskStatus::Stopped).await;
    release_lead(&rig, &tasks[0]);

    let state = published(&rig).await;
    assert!(
        state.find_task(&tasks[1]).unwrap().member.renewals().is_some(),
        "the member was started in a new session before it failed"
    );
    assert_eq!(
        state.find_task(&tasks[0]).unwrap().member.runs().len(),
        3,
        "the lead was told once after it delivered, and took it"
    );
    let told = rig.prompt(&tasks[0], 3).await;
    rig.stop().await.unwrap();

    assert!(told.contains("draft.txt"), "the lead is told of the file: {told}");
}

// Rule: a file the lead was told of that changed afterwards is another file: the lead is told again, and the round is
// published only when the lead ended after that.
#[tokio::test]
async fn a_draft_that_changed_after_the_lead_was_told_is_told_again() {
    let (mut rig, tasks) = team(
        "sleep,integrate,integrate-after-changing-drafts,integrate",
        "fail-tool-committed-dirty",
    )
    .await;
    rig.status_is(&tasks[1], TaskStatus::Failed).await;

    let state = published(&rig).await;
    assert_eq!(
        state.find_task(&tasks[0]).unwrap().member.runs().len(),
        4,
        "told of the draft, changed it, told of the change, took it"
    );
    let told = rig.prompt(&tasks[0], 4).await;
    rig.stop().await.unwrap();

    assert!(
        told.contains("draft.txt"),
        "the lead is told of the changed file: {told}"
    );
    assert_eq!(state.find_task(&tasks[1]).unwrap().member.status(), TaskStatus::Stopped);
}

// Rule: the human is asked once what the lead left, also across a restart of the daemon: the notice is found among the
// saved ones and not made again when the lead ends once more.
#[tokio::test]
async fn the_human_is_asked_once_also_when_the_daemon_restarts_and_the_lead_ends_again() {
    let (mut rig, tasks) = team("sleep,work,work,work,work,work", "fail-tool-committed").await;
    rig.status_is(&tasks[1], TaskStatus::Failed).await;
    let commit = member_tip(&rig, &tasks[1]);
    rig.notification_with(&commit).await;
    rig.stop().await.unwrap();
    rig.start().await;
    let lead = tasks[0].clone();
    let asked = Connection::connect(&rig.home.socket_file())
        .await
        .unwrap()
        .request(
            Client::Cli,
            Command::Say(SayRequest {
                target: Target::Task(lead.clone()),
                text: "Look at it again.".to_owned().try_into().unwrap(),
            }),
        )
        .await
        .unwrap();
    assert!(matches!(asked, Reply::Applied { .. }), "{asked:?}");
    rig.wait_for("the lead ended its fourth run", || rig.runs(&lead) >= 4)
        .await;
    rig.status_is(&lead, TaskStatus::Waiting).await;
    look(&rig).await;
    let runs = rig.runs(&lead);
    let asked: Vec<_> = rig
        .notifications()
        .into_iter()
        .filter(|text| text.contains(&commit))
        .collect();
    rig.stop().await.unwrap();

    assert_eq!(runs, 4, "the lead is not woken again for what the human was told");
    assert_eq!(asked.len(), 1, "the human is asked once: {asked:?}");
}
