//! A task whose session went bad goes on in a new session of the same agent, briefed with what its runs left.

use agent_core::state::{ItemPhase, MergeMode};

use super::rig::{ONE, Rig, git, investigator, normal};

/// A reviewer that approves.
const APPROVE: &str = "verify:VERDICT: APPROVE\\nSUMMARY: fine";

// Rule: after the retries in the same session, the task goes on once in a new session whose prompt is the briefing,
// with the uncommitted files kept in a snapshot first; the round goes on.
#[tokio::test]
async fn a_task_whose_session_ends_without_a_status_goes_on_in_a_new_session_with_a_briefing() {
    let mut rig = Rig::new(
        &[
            normal("claude-1", "nostatus-dirty,nostatus,nostatus,work"),
            investigator("inv-r1", APPROVE),
            investigator("inv-r2", APPROVE),
        ],
        5,
        false,
    )
    .await;
    rig.start().await;
    let (_, tasks) = rig.run(&["claude-1"], ONE, MergeMode::Autopilot, None).await;

    let briefing = rig.prompt(&tasks[0], 4).await;
    let snapshots = git(
        &rig.repo,
        &["for-each-ref", "--format=%(refname)", "refs/agents/snapshots/"],
    );
    let state = rig
        .item_where("1.1", "merged after the new session", |phase| {
            matches!(phase, ItemPhase::Done { .. })
        })
        .await;
    rig.stop().await.unwrap();

    assert!(
        briefing.starts_with("# A NEW SESSION ON A TASK THAT WAS ALREADY STARTED"),
        "{briefing}"
    );
    assert!(
        briefing.contains("its last runs ended without a `STATUS:` line"),
        "{briefing}"
    );
    assert!(briefing.contains("Do the first thing in the repository."), "{briefing}");
    assert!(briefing.contains("- draft.txt"), "{briefing}");
    let snapshot = snapshots.lines().next().expect("a snapshot before the new session");
    assert!(briefing.contains(snapshot), "{briefing}");
    assert_eq!(
        git(&rig.repo, &["show", &format!("{snapshot}:draft.txt")]),
        "draft of run 1"
    );
    let member = state.find_task(&tasks[0]).unwrap().member;
    let runs = member.runs();
    assert_eq!(runs.len(), 4);
    assert_eq!(
        runs[0].session(),
        runs[2].session(),
        "the retries resume the first session"
    );
    assert_ne!(runs[3].session(), runs[2].session(), "the fourth run is a new session");
    assert_eq!(member.renewals().map(agent_core::state::Renewals::used), Some(1));
}
