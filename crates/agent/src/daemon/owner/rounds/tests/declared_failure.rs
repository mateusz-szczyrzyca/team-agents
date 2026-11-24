//! A run that reports `STATUS: FAILED` after it used a tool is not the end of its task: the ladder starts the task in a
//! new session before anything fails it.

use agent_core::ids::TaskId;
use agent_core::state::{ItemPhase, MergeMode, State};
use agent_core::status::TaskStatus;

use super::rig::{ONE, Rig, investigator, normal};

/// A reviewer that approves.
const APPROVE: &str = "verify:VERDICT: APPROVE\\nSUMMARY: fine";

// Rule: a member that reports FAILED after a tool call is started once more in a new session; its task is not failed.
#[tokio::test]
async fn a_declared_failure_after_a_tool_call_goes_on_in_a_new_session() {
    let mut rig = Rig::new(
        &[
            investigator("inv-lead", "sleep"),
            normal("claude-1", "fail-first-tool:the second session says done"),
            investigator("inv-r1", APPROVE),
            investigator("inv-r2", APPROVE),
        ],
        5,
        true,
    )
    .await;
    rig.start().await;
    let (_, tasks) = rig
        .run(&["inv-lead", "claude-1"], ONE, MergeMode::Autopilot, Some("inv-lead"))
        .await;
    rig.wait_for("the second run to start", || rig.runs(&tasks[1]) >= 2)
        .await;
    let state = rig.state();
    rig.stop().await.unwrap();

    let member = state.find_task(&tasks[1]).unwrap().member;
    assert_ne!(
        member.status(),
        TaskStatus::Failed,
        "a declared failure is not the end of the task"
    );
    assert_ne!(
        member.runs()[1].session(),
        member.runs()[0].session(),
        "the second run is a new session"
    );
    assert_eq!(member.renewals().map(agent_core::state::Renewals::used), Some(1));
}

/// Adds `table` to the configuration of `rig`, before the daemon starts.
fn configure(rig: &Rig, table: &str) {
    let mut config = std::fs::read_to_string(rig.home.config_file()).unwrap();
    config.push_str(table);
    std::fs::write(rig.home.config_file(), config).unwrap();
}

/// Waits until `claude-2`, the replacement, has taken `task` over and ended its run: the third of the task, after the
/// two of `claude-1`.
async fn replacement_ended_a_run(rig: &Rig, task: &TaskId) -> State {
    rig.state_where("the replacement ended a run of the task", |state| {
        state.find_task(task).is_some_and(|found| {
            found.member.agent().name() == "claude-2"
                && found.member.runs().len() == 3
                && found.member.current_run().is_none()
        })
    })
    .await
}

/// The texts the bus of `rig` keeps, every file of its directory read as text.
fn bus_texts(rig: &Rig) -> String {
    let mut texts = String::new();
    let mut dirs = vec![rig.home.root().join("bus")];
    while let Some(dir) = dirs.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries {
            let path = entry.unwrap().path();
            if path.is_dir() {
                dirs.push(path);
            } else if let Ok(text) = std::fs::read_to_string(&path) {
                texts.push_str(&text);
            }
        }
    }
    texts
}

// Rule: a member that reports FAILED after a tool call in its new session too is taken over by the first fitting agent
// of `[replacement] agents`, which is briefed with the reason `failed` and what the failed run said, and works in the
// same worktree and on the same branch; the item is finished by it.
#[tokio::test]
async fn a_declared_failure_that_fails_again_is_taken_over_by_a_fitting_agent_in_the_same_worktree() {
    let mut rig = Rig::new(
        &[
            investigator("inv-lead", "sleep"),
            normal("claude-1", "fail-tool"),
            normal("claude-2", "work"),
            investigator("inv-r1", APPROVE),
            investigator("inv-r2", APPROVE),
        ],
        5,
        true,
    )
    .await;
    configure(&rig, "[replacement]\nagents = [\"claude-2\"]\n");
    rig.start().await;
    let (_, tasks) = rig
        .run(&["inv-lead", "claude-1"], ONE, MergeMode::Autopilot, Some("inv-lead"))
        .await;

    let state = replacement_ended_a_run(&rig, &tasks[1]).await;
    let takeover = rig.prompt(&tasks[1], 3).await;
    let log = std::fs::read_to_string(rig.worktree(&tasks[1]).join(".fake-log")).unwrap();
    let told = rig.notifications();
    rig.stop().await.unwrap();

    let member = state.find_task(&tasks[1]).unwrap().member;
    assert_eq!(
        member.agent().name(),
        "claude-2",
        "the first fitting agent holds the task"
    );
    assert_eq!(member.past_holders().len(), 1, "who held it is recorded");
    assert_eq!(member.past_holders()[0].agent.name(), "claude-1");
    assert_eq!(
        member.runs().len(),
        3,
        "two failed runs of claude-1, one run of claude-2"
    );
    assert!(
        takeover.contains("You are agent **claude-2**"),
        "the replacement is briefed: {takeover}"
    );
    assert!(
        takeover.contains("`failed`"),
        "the briefing gives the reason of the failure: {takeover}"
    );
    assert!(
        takeover.contains("I could not do it."),
        "the briefing holds what the failed run said: {takeover}"
    );
    assert!(
        told.iter().all(|text| !text.contains("ended with STATUS: FAILED")),
        "the human is not told while a rung is untried: {told:?}"
    );
    let branches: Vec<&str> = log.lines().filter_map(|line| line.split_whitespace().nth(2)).collect();
    assert_eq!(branches.len(), 3, "{log}");
    assert!(
        branches.iter().all(|branch| *branch == branches[0]) && branches[0].starts_with("agents/claude-1/"),
        "every run on the task's branch: {branches:?}"
    );
}

// Rule: with no fitting agent and the team's lead to take it, a declared failure that fails again gives the lead the
// member's work with its own text, and the member's task stops rather than failing.
#[tokio::test]
async fn a_declared_failure_that_fails_again_gives_the_lead_the_members_work() {
    let mut rig = Rig::new(
        &[
            investigator("inv-lead", "sleep,work"),
            normal("claude-1", "fail-tool"),
            investigator("inv-r1", APPROVE),
            investigator("inv-r2", APPROVE),
        ],
        5,
        true,
    )
    .await;
    configure(&rig, "[replacement]\nagents = [\"inv-lead\"]\n");
    rig.start().await;
    let (_, tasks) = rig
        .run(&["inv-lead", "claude-1"], ONE, MergeMode::Autopilot, Some("inv-lead"))
        .await;

    let member_task = tasks[1].clone();
    let state = rig
        .item_where("1.1", "published by the lead", |phase| {
            matches!(
                phase,
                ItemPhase::InReview { .. } | ItemPhase::Approved { .. } | ItemPhase::Done { .. }
            )
        })
        .await;
    let bus = bus_texts(&rig);
    rig.stop().await.unwrap();

    let member = state.find_task(&member_task).unwrap().member;
    assert_eq!(
        member.status(),
        TaskStatus::Stopped,
        "the member's task stops, it does not fail"
    );
    assert_eq!(member.runs().len(), 2, "the failed run and its new session");
    assert_eq!(member.past_holders().len(), 0, "the lead, not an agent, takes the work");
    assert!(
        bus.contains("WORK OF A MEMBER THAT IS GONE"),
        "the lead's own text: {bus}"
    );
    assert!(
        bus.contains("commit the files the member left uncommitted"),
        "the lead is told to commit the member's files first: {bus}"
    );
    assert!(bus.contains("agents/claude-1/"), "the text names the branch: {bus}");
}

// Rule: `[replacement] mode = "off"` keeps a declared failure final: the task fails at its first failed run, and nothing
// is handed over; the human is told once.
#[tokio::test]
async fn mode_off_keeps_a_declared_failure_final() {
    let mut rig = Rig::new(
        &[
            investigator("inv-lead", "sleep"),
            normal("claude-1", "fail-tool"),
            normal("claude-2", "work"),
            investigator("inv-r1", APPROVE),
            investigator("inv-r2", APPROVE),
        ],
        5,
        true,
    )
    .await;
    configure(&rig, "[replacement]\nmode = \"off\"\nagents = [\"claude-2\"]\n");
    rig.start().await;
    let (_, tasks) = rig
        .run(&["inv-lead", "claude-1"], ONE, MergeMode::Autopilot, Some("inv-lead"))
        .await;

    let state = rig
        .state_where("the task failed", |state| {
            state
                .find_task(&tasks[1])
                .is_some_and(|found| found.member.status() == TaskStatus::Failed)
        })
        .await;
    rig.stop().await.unwrap();

    let member = state.find_task(&tasks[1]).unwrap().member;
    assert_eq!(member.runs().len(), 1, "mode off: the failed run is not started again");
    assert_eq!(member.past_holders().len(), 0, "mode off: nothing is handed over");
    assert_eq!(member.agent().name(), "claude-1");
    assert_eq!(
        rig.notifications()
            .iter()
            .filter(|text| text.contains("The run of claude-1 ended with STATUS: FAILED"))
            .count(),
        1,
        "the human is told once, when the ladder has no rung left"
    );
}

// Rule: no file a failed member left uncommitted is lost on the way up the ladder: the new session and the agent that
// takes the task over are both briefed with the file, and the worktree still holds it.
#[tokio::test]
async fn every_holder_after_a_declared_failure_is_told_of_the_uncommitted_file() {
    let mut rig = Rig::new(
        &[
            investigator("inv-lead", "sleep"),
            normal("claude-1", "fail-tool-dirty"),
            normal("claude-2", "work"),
            investigator("inv-r1", APPROVE),
            investigator("inv-r2", APPROVE),
        ],
        5,
        true,
    )
    .await;
    configure(&rig, "[replacement]\nagents = [\"claude-2\"]\n");
    rig.start().await;
    let (_, tasks) = rig
        .run(&["inv-lead", "claude-1"], ONE, MergeMode::Autopilot, Some("inv-lead"))
        .await;
    let worktree = rig.worktree(&tasks[1]);
    rig.wait_for("the first run to start", || worktree.join(".fake-args-1").exists())
        .await;
    std::fs::write(worktree.join(".fake-continue"), b"").unwrap();

    replacement_ended_a_run(&rig, &tasks[1]).await;
    let new_session = rig.prompt(&tasks[1], 2).await;
    let takeover = rig.prompt(&tasks[1], 3).await;
    let kept = worktree.join("draft.txt").exists();
    rig.stop().await.unwrap();

    for (who, briefing) in [("the new session", &new_session), ("the replacement", &takeover)] {
        assert!(briefing.contains("`failed`"), "{who} is told the reason: {briefing}");
        assert!(
            briefing.contains("- draft.txt"),
            "{who} is told of the uncommitted file: {briefing}"
        );
        assert!(
            briefing.contains("I could not do it."),
            "{who} gets what the failed run said: {briefing}"
        );
    }
    assert!(kept, "the uncommitted file is still in the worktree");
}

// Rule: the rung a task is on survives a restart of the daemon: a restart in the middle of the new session of a declared
// failure does not give the task back its rungs, and it still ends in the hands of the replacement without a word to
// the human.
#[tokio::test]
async fn a_restart_in_the_new_session_of_a_declared_failure_keeps_the_ladder() {
    let mut rig = Rig::new(
        &[
            investigator("inv-lead", "sleep"),
            normal("claude-1", "fail-tool-dirty"),
            normal("claude-2", "work"),
            investigator("inv-r1", APPROVE),
            investigator("inv-r2", APPROVE),
        ],
        5,
        true,
    )
    .await;
    configure(&rig, "[replacement]\nagents = [\"claude-2\"]\n");
    rig.start().await;
    let (_, tasks) = rig
        .run(&["inv-lead", "claude-1"], ONE, MergeMode::Autopilot, Some("inv-lead"))
        .await;
    let worktree = rig.worktree(&tasks[1]);
    rig.wait_for("the new session to start", || worktree.join(".fake-args-2").exists())
        .await;
    let before = rig.state();
    rig.stop().await.unwrap();
    std::fs::write(worktree.join(".fake-continue"), b"").unwrap();
    rig.start().await;

    let state = replacement_ended_a_run(&rig, &tasks[1]).await;
    let told = rig.notifications();
    rig.stop().await.unwrap();

    assert_eq!(
        before
            .find_task(&tasks[1])
            .unwrap()
            .member
            .renewals()
            .map(agent_core::state::Renewals::used),
        Some(1),
        "the new session was recorded before the restart"
    );
    let member = state.find_task(&tasks[1]).unwrap().member;
    assert_eq!(member.agent().name(), "claude-2");
    assert_eq!(member.past_holders().len(), 1);
    assert!(
        told.iter().all(|text| !text.contains("ended with STATUS: FAILED")),
        "the human is not told while a rung is untried: {told:?}"
    );
}

/// The output of `git` with `args` in `dir`.
fn git_out(dir: &std::path::Path, args: &[&str]) -> String {
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .unwrap();
    assert!(output.status.success(), "git {args:?} failed in {dir:?}");
    String::from_utf8(output.stdout).unwrap()
}

// Rule: the lead that is given the work of a failed member, which committed one file and left another uncommitted, is told
// what the failed run said, whole, and where the worktree and the branch are; it commits the uncommitted file there as it
// is, takes the branch into its own and finishes — the member's task is stopped, not failed, and the human is not asked.
#[tokio::test]
async fn the_lead_commits_the_failed_members_file_in_its_worktree_and_finishes_without_asking() {
    let mut rig = Rig::new(
        &[
            investigator("inv-lead", "sleep,take-gone-members-work"),
            normal("claude-1", "fail-tool-committed-dirty"),
            investigator("inv-r1", APPROVE),
            investigator("inv-r2", APPROVE),
        ],
        5,
        true,
    )
    .await;
    rig.start().await;
    let (_, tasks) = rig
        .run(&["inv-lead", "claude-1"], ONE, MergeMode::Autopilot, Some("inv-lead"))
        .await;

    let state = rig
        .item_where("1.1", "published by the lead", |phase| {
            matches!(
                phase,
                ItemPhase::InReview { .. } | ItemPhase::Approved { .. } | ItemPhase::Done { .. }
            )
        })
        .await;
    let bus = bus_texts(&rig);
    let member_tree = rig.worktree(&tasks[1]);
    let lead_tree = rig.worktree(&tasks[0]);
    let member_log = git_out(&member_tree, &["log", "--format=%s", "-n", "3"]);
    let member_dirty = git_out(&member_tree, &["status", "--porcelain"]);
    let lead_files = git_out(&lead_tree, &["ls-files"]);
    let asked = rig.home.questions_file(&tasks[0], std::num::NonZeroU32::MIN).exists();
    rig.stop().await.unwrap();

    let member = state.find_task(&tasks[1]).unwrap().member;
    assert_eq!(
        member.status(),
        TaskStatus::Stopped,
        "the member's task is taken over, not failed"
    );
    assert!(
        bus.contains("I could not do it."),
        "the lead is given what the failed run said: {bus}"
    );
    assert!(
        member_log.contains("Commit the files the failed member left, as they are"),
        "the lead committed the member's file on the member's branch: {member_log}"
    );
    assert_eq!(member_dirty, "", "nothing of the member is left uncommitted");
    assert!(
        lead_files.lines().any(|file| file == "draft.txt"),
        "the file reached the lead's branch: {lead_files}"
    );
    assert!(!asked, "the lead did not ask the human for leave");
}
