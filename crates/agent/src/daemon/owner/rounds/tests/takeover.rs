//! A task whose new session went bad too is given to another agent — its work, its worktree and its branch — and
//! carries on there.

use agent_core::state::{ItemPhase, MergeMode};

use super::rig::{ONE, Rig, investigator, normal};

/// A reviewer that approves.
const APPROVE: &str = "verify:VERDICT: APPROVE\\nSUMMARY: fine";

/// `table` added to the configuration of `rig`, before the daemon starts.
fn append(rig: &Rig, table: &str) {
    let mut config = std::fs::read_to_string(rig.home.config_file()).unwrap();
    config.push_str(table);
    std::fs::write(rig.home.config_file(), config).unwrap();
}

// Rule: a member that fails in both sessions of its agent is taken over by the first fitting agent of
// `[replacement] agents`, which works in the same worktree and branch and ends DONE; the state keeps who held it.
#[tokio::test]
async fn a_member_that_fails_in_both_sessions_is_taken_over_by_the_first_fitting_agent() {
    let mut rig = Rig::new(
        &[
            normal("claude-1", "nostatus,nostatus,nostatus,nostatus,nostatus,nostatus,work"),
            normal("claude-2", "work"),
            investigator("inv-r1", APPROVE),
            investigator("inv-r2", APPROVE),
        ],
        5,
        false,
    )
    .await;
    append(&rig, "[replacement]\nagents = [\"claude-2\"]\n");
    rig.start().await;
    let (_, tasks) = rig.run(&["claude-1"], ONE, MergeMode::Autopilot, None).await;

    // Read the state right after the fourth run ended: the ladder's answer is what the fifth run follows.
    let state = rig
        .item_where("1.1", "merged after the takeover", |phase| {
            matches!(phase, ItemPhase::Done { .. })
        })
        .await;
    let takeover = rig.prompt(&tasks[0], 7).await;
    // It works on the task's own branch — the one the agent it replaced worked on, an item's branch named after that
    // agent — as the fake CLI logged each run.
    let log = std::fs::read_to_string(rig.worktree(&tasks[0]).join(".fake-log")).unwrap_or_default();
    let branches: Vec<&str> = log.lines().filter_map(|line| line.split_whitespace().nth(2)).collect();
    assert!(
        !branches.is_empty() && branches.iter().all(|branch| *branch == branches[0]),
        "every run of the task on one branch: {branches:?}"
    );
    rig.stop().await.unwrap();

    let member = state.find_task(&tasks[0]).unwrap().member;
    assert_eq!(
        member.agent().name(),
        "claude-2",
        "the first fitting agent holds it: status {:?}, renewals {:?}, past {:?}, runs {:?}",
        member.status(),
        member.renewals(),
        member.past_holders().len(),
        member.runs().iter().map(|run| run.state().clone()).collect::<Vec<_>>()
    );
    assert_eq!(member.past_holders().len(), 1, "who held it is recorded");
    assert_eq!(member.past_holders()[0].agent.name(), "claude-1");
    // The agent that takes the task over has no session to resume: it is given the whole task, in the worktree and on
    // the branch of the task, as any first run of it.
    assert!(takeover.starts_with("# Working context"), "{takeover}");
    assert!(takeover.contains("You are agent **claude-2**"), "{takeover}");
    assert!(takeover.contains("Do the first thing in the repository."), "{takeover}");
    assert!(
        takeover.contains(&format!("worktrees/example/{}`", tasks[0])),
        "the same worktree of the task: {takeover}"
    );
}

// Rule: with no fitting agent — here the only listed one is in the team already — the team's lead gets the work, with
// its own text naming the task, the agent and the branch; the member's task stops.
#[tokio::test]
async fn the_lead_takes_the_work_when_no_listed_agent_fits() {
    let mut rig = Rig::new(
        &[
            normal(
                "claude-1",
                "nostatus,nostatus,nostatus,nostatus,nostatus,nostatus,nostatus",
            ),
            investigator("inv-lead", "sleep,sleep,work"),
            investigator("inv-r1", APPROVE),
            investigator("inv-r2", APPROVE),
        ],
        5,
        false,
    )
    .await;
    append(&rig, "[replacement]\nagents = []\n");
    rig.start().await;
    let (_, tasks) = rig
        .run(&["inv-lead", "claude-1"], ONE, MergeMode::Autopilot, Some("inv-lead"))
        .await;

    rig.wait_for("the lead's second run", || rig.runs(&tasks[0]) >= 2).await;
    let lead = rig.prompt(&tasks[0], 2).await;
    let state = rig.state();
    rig.stop().await.unwrap();

    assert!(
        lead.contains("# WORK OF A MEMBER THAT IS GONE"),
        "the lead is told whose work it has: {lead}"
    );
    assert!(lead.contains(&tasks[1].to_string()), "the task is named: {lead}");
    assert!(lead.contains("claude-1"), "the agent it was taken from: {lead}");
    assert!(lead.contains("agents/claude-1/"), "the branch: {lead}");
    let member = state.find_task(&tasks[1]).unwrap().member;
    assert_ne!(
        member.status(),
        agent_core::status::TaskStatus::Running,
        "the member's task no longer runs"
    );
}

// Rule: the first fitting agent of the team itself is left out; the next listed agent takes the task.
#[tokio::test]
async fn an_agent_of_the_team_is_left_out_of_the_takeover() {
    let mut rig = Rig::new(
        &[
            normal(
                "claude-1",
                "nostatus,nostatus,nostatus,nostatus,nostatus,nostatus,nostatus,work",
            ),
            normal("claude-2", "work"),
            normal("claude-3", "work"),
            investigator("inv-lead", "sleep"),
            investigator("inv-r1", APPROVE),
            investigator("inv-r2", APPROVE),
        ],
        5,
        false,
    )
    .await;
    // claude-2 is a member of the same team; claude-3 is not.
    append(&rig, "[replacement]\nagents = [\"claude-2\", \"claude-3\"]\n");
    rig.start().await;
    let (_, tasks) = rig
        .run(&["inv-lead", "claude-1"], ONE, MergeMode::Autopilot, Some("inv-lead"))
        .await;

    rig.wait_for("the takeover run", || rig.runs(&tasks[1]) >= 8).await;
    let state = rig.state();
    rig.stop().await.unwrap();

    let member = state.find_task(&tasks[1]).unwrap().member;
    assert_eq!(member.agent().name(), "claude-3", "the next listed agent takes it");
    assert_eq!(member.past_holders().len(), 1);
}

// Rule: mode off with a fitting agent listed fails the task as today: no hand-over, no record of a past holder.
#[tokio::test]
async fn the_off_mode_fails_the_task_and_hands_it_to_nobody() {
    let mut rig = Rig::new(
        &[
            normal(
                "claude-1",
                "nostatus,nostatus,nostatus,nostatus,nostatus,nostatus,nostatus",
            ),
            normal("claude-2", "work"),
            investigator("inv-r1", APPROVE),
            investigator("inv-r2", APPROVE),
        ],
        5,
        false,
    )
    .await;
    append(&rig, "[replacement]\nagents = [\"claude-2\"]\nmode = \"off\"\n");
    rig.start().await;
    let (_, tasks) = rig.run(&["claude-1"], ONE, MergeMode::Autopilot, None).await;

    rig.status_is(&tasks[0], agent_core::status::TaskStatus::Failed).await;
    let state = rig.state();
    rig.stop().await.unwrap();

    let member = state.find_task(&tasks[0]).unwrap().member;
    assert_eq!(member.agent().name(), "claude-1");
    assert_eq!(member.past_holders().len(), 0);
}

// Rule: without a [replacement] table the task fails after its new session, as today.
#[tokio::test]
async fn without_a_replacement_table_the_task_fails_as_before() {
    let mut rig = Rig::new(
        &[
            normal(
                "claude-1",
                "nostatus,nostatus,nostatus,nostatus,nostatus,nostatus,nostatus",
            ),
            normal("claude-2", "work"),
            investigator("inv-r1", APPROVE),
            investigator("inv-r2", APPROVE),
        ],
        5,
        false,
    )
    .await;
    rig.start().await;
    let (_, tasks) = rig.run(&["claude-1"], ONE, MergeMode::Autopilot, None).await;

    rig.status_is(&tasks[0], agent_core::status::TaskStatus::Failed).await;
    let state = rig.state();
    rig.stop().await.unwrap();

    assert_eq!(state.find_task(&tasks[0]).unwrap().member.past_holders().len(), 0);
}

// Rule: the agent that took a task over starts the ladder afresh: one run without a status of its own is resumed in its
// session, not failed and not handed over again.
#[tokio::test]
async fn the_agent_that_took_over_gets_its_own_retries() {
    let retries = "nostatus,nostatus,nostatus,nostatus,nostatus,nostatus,work";
    let mut rig = Rig::new(
        &[
            normal("claude-1", retries),
            normal("claude-2", "nostatus,work"),
            investigator("inv-r1", APPROVE),
            investigator("inv-r2", APPROVE),
        ],
        5,
        false,
    )
    .await;
    append(&rig, "[replacement]\nagents = [\"claude-2\"]\n");
    rig.start().await;
    let (_, tasks) = rig.run(&["claude-1"], ONE, MergeMode::Autopilot, None).await;

    let state = rig
        .item_where("1.1", "merged after the takeover", |phase| {
            matches!(phase, ItemPhase::Done { .. })
        })
        .await;
    rig.stop().await.unwrap();

    let member = state.find_task(&tasks[0]).unwrap().member;
    assert_eq!(member.agent().name(), "claude-2");
    assert_eq!(
        member.past_holders().len(),
        1,
        "the taking-over agent is handed nothing"
    );
    let after_handover: Vec<_> = member.runs().iter().skip(7).map(|run| run.session().cloned()).collect();
    assert!(
        after_handover.len() >= 2,
        "its failed run and the retry: {after_handover:?}"
    );
    assert_eq!(
        after_handover[0], after_handover[1],
        "the retry is in the same session of the taking-over agent"
    );
}
