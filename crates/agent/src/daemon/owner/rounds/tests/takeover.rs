//! A task whose new session went bad too is given to another agent of `[replacement] agents` — in its worktree, on its
//! branch, with the briefing — or, with nobody fitting, its work goes to the team's lead; mode `off` and a file without
//! `[replacement] agents` fail it as before.

use agent_core::state::{ItemPhase, MergeMode, State};
use agent_core::status::TaskStatus;

use super::rig::{ONE, Rig, investigator, normal};

/// A reviewer that approves.
const APPROVE: &str = "verify:VERDICT: APPROVE\\nSUMMARY: fine";

/// Six runs without a status: the two retries and the third run of the first session, then the same of the new one.
const BAD_IN_BOTH_SESSIONS: &str = "nostatus";

/// Adds `table` to the configuration of `rig`, before the daemon starts.
fn configure(rig: &Rig, table: &str) {
    let mut config = std::fs::read_to_string(rig.home.config_file()).unwrap();
    config.push_str(table);
    std::fs::write(rig.home.config_file(), config).unwrap();
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

/// The agent that holds `task` in `state`.
fn holder(state: &State, task: &agent_core::ids::TaskId) -> String {
    state.find_task(task).unwrap().member.agent().name().to_owned()
}

// Rule: a task that fails in both sessions of its agent is taken over by the first fitting agent of
// `[replacement] agents`, which is briefed, works in the same worktree and on the same branch and ends DONE; the state
// keeps who held it.
#[tokio::test]
async fn a_task_that_fails_in_both_sessions_is_taken_over_by_the_first_fitting_agent_with_the_briefing() {
    let mut rig = Rig::new(
        &[
            normal("claude-1", BAD_IN_BOTH_SESSIONS),
            normal("claude-2", "work"),
            investigator("inv-r1", APPROVE),
            investigator("inv-r2", APPROVE),
        ],
        5,
        false,
    )
    .await;
    configure(&rig, "[replacement]\nagents = [\"claude-2\"]\n");
    rig.start().await;
    let (_, tasks) = rig.run(&["claude-1"], ONE, MergeMode::Autopilot, None).await;

    let state = rig
        .item_where("1.1", "merged after the takeover", |phase| {
            matches!(phase, ItemPhase::Done { .. })
        })
        .await;
    let takeover = rig.prompt(&tasks[0], 7).await;
    let log = std::fs::read_to_string(rig.worktree(&tasks[0]).join(".fake-log")).unwrap();
    rig.stop().await.unwrap();

    let member = state.find_task(&tasks[0]).unwrap().member;
    assert_eq!(
        member.agent().name(),
        "claude-2",
        "the first fitting agent holds the task"
    );
    assert_eq!(member.past_holders().len(), 1, "who held it is recorded");
    assert_eq!(member.past_holders()[0].agent.name(), "claude-1");
    assert_eq!(
        member.runs().len(),
        7,
        "six bad runs of claude-1, one good run of claude-2"
    );
    // The agent that takes the task over is briefed on where the work stands, and its task is its own first prompt.
    assert!(
        takeover.starts_with("# A NEW SESSION ON A TASK THAT WAS ALREADY STARTED"),
        "{takeover}"
    );
    assert!(takeover.contains("You are agent **claude-2**"), "{takeover}");
    assert!(takeover.contains("Do the first thing in the repository."), "{takeover}");
    // Every run, of either agent, was on one branch, in the task's worktree (the fake CLI logs `N <session> <branch>`).
    let branches: Vec<&str> = log.lines().filter_map(|line| line.split_whitespace().nth(2)).collect();
    assert_eq!(branches.len(), 7, "{log}");
    assert!(
        branches.iter().all(|branch| *branch == branches[0]) && branches[0].starts_with("agents/claude-1/"),
        "every run on the task's branch: {branches:?}"
    );
}

// Rule: the agent that took a task over starts the ladder afresh: its own run without a status is resumed in its own
// session, not failed and not handed over again.
#[tokio::test]
async fn the_agent_that_took_a_task_over_has_retries_of_its_own() {
    let mut rig = Rig::new(
        &[
            normal("claude-1", BAD_IN_BOTH_SESSIONS),
            // The fake CLI takes the action of the task's run number: claude-2's first run is the task's seventh.
            normal("claude-2", "work,work,work,work,work,work,nostatus,work"),
            investigator("inv-r1", APPROVE),
            investigator("inv-r2", APPROVE),
        ],
        5,
        false,
    )
    .await;
    configure(&rig, "[replacement]\nagents = [\"claude-2\"]\n");
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
    assert_eq!(member.past_holders().len(), 1, "taken over once, not twice");
    let sessions: Vec<_> = member.runs().iter().skip(6).map(|run| run.session().cloned()).collect();
    assert_eq!(
        sessions.len(),
        2,
        "claude-2's run without a status and its retry: {sessions:?}"
    );
    assert!(sessions[0].is_some(), "{sessions:?}");
    assert_eq!(sessions[0], sessions[1], "the retry resumes claude-2's own session");
    let before = member.runs()[5].session().cloned();
    assert_ne!(sessions[0], before, "claude-2 does not resume the session of claude-1");
}

// Rule: an agent already in the task's team is left out of the takeover; the next listed agent takes the task.
#[tokio::test]
async fn an_agent_of_the_team_is_left_out_of_the_takeover() {
    let mut rig = Rig::new(
        &[
            investigator("inv-lead", "sleep"),
            normal("claude-1", BAD_IN_BOTH_SESSIONS),
            normal("claude-3", "work"),
            investigator("inv-r1", APPROVE),
            investigator("inv-r2", APPROVE),
        ],
        5,
        true,
    )
    .await;
    // The team's lead is listed first: it fits by its tier and sleeps, but it is in the team.
    configure(&rig, "[replacement]\nagents = [\"inv-lead\", \"claude-3\"]\n");
    rig.start().await;
    let (_, tasks) = rig
        .run(&["inv-lead", "claude-1"], ONE, MergeMode::Autopilot, Some("inv-lead"))
        .await;

    let member_task = tasks[1].clone();
    let state = rig
        .state_where("claude-3 ended a run of the task", |state| {
            let member = state.find_task(&member_task).unwrap().member;
            member.agent().name() == "claude-3" && member.runs().len() == 7 && member.current_run().is_none()
        })
        .await;
    rig.stop().await.unwrap();

    let member = state.find_task(&tasks[1]).unwrap().member;
    assert_eq!(member.past_holders().len(), 1);
    assert_eq!(member.past_holders()[0].agent.name(), "claude-1");
    assert_eq!(holder(&state, &tasks[0]), "inv-lead", "the lead keeps its own task");
}

// Rule: with nobody fitting — the only listed agent is the team's lead — the lead gets the member's work with its own
// text, naming the task, the agent and the branch; the member's task stops, and the lead that delivers the work
// publishes the item.
#[tokio::test]
async fn with_no_fitting_agent_the_lead_gets_the_members_work_with_its_own_text() {
    let mut rig = Rig::new(
        &[
            investigator("inv-lead", "sleep,work"),
            normal("claude-1", BAD_IN_BOTH_SESSIONS),
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

    let member = state.find_task(&tasks[1]).unwrap().member;
    assert_eq!(member.status(), TaskStatus::Stopped, "the member's task stops");
    assert_eq!(member.runs().len(), 6);
    assert_eq!(member.past_holders().len(), 0, "nobody took the task itself over");
    let lead = state.find_task(&tasks[0]).unwrap().member;
    assert_eq!(
        lead.runs().len(),
        2,
        "the lead slept, was woken by its text, and delivered"
    );
    assert!(
        bus.contains("WORK OF A MEMBER THAT IS GONE"),
        "the lead's own text: {bus}"
    );
    assert!(bus.contains(tasks[1].as_str()), "the text names the task: {bus}");
    assert!(bus.contains("agents/claude-1/"), "the text names the branch: {bus}");
}

// Rule: mode `off` fails the task as before, even with a fitting agent listed: nothing is handed over.
#[tokio::test]
async fn mode_off_fails_the_task_as_before() {
    let mut rig = Rig::new(
        &[
            normal("claude-1", BAD_IN_BOTH_SESSIONS),
            normal("claude-2", "work"),
            investigator("inv-r1", APPROVE),
            investigator("inv-r2", APPROVE),
        ],
        5,
        false,
    )
    .await;
    configure(&rig, "[replacement]\nagents = [\"claude-2\"]\nmode = \"off\"\n");
    rig.start().await;
    let (_, tasks) = rig.run(&["claude-1"], ONE, MergeMode::Autopilot, None).await;

    let state = rig.status_is(&tasks[0], TaskStatus::Failed).await;
    rig.stop().await.unwrap();

    let member = state.find_task(&tasks[0]).unwrap().member;
    assert_eq!(member.agent().name(), "claude-1");
    assert_eq!(member.past_holders().len(), 0, "nothing was handed over");
    assert_eq!(member.runs().len(), 6);
}

// Rule: a file without `[replacement] agents` fails the task after its new session, as before.
#[tokio::test]
async fn without_replacement_agents_the_task_fails_as_before() {
    let mut rig = Rig::new(
        &[
            investigator("inv-lead", "sleep"),
            normal("claude-1", BAD_IN_BOTH_SESSIONS),
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

    let state = rig.status_is(&tasks[1], TaskStatus::Failed).await;
    let bus = bus_texts(&rig);
    rig.stop().await.unwrap();

    let member = state.find_task(&tasks[1]).unwrap().member;
    assert_eq!(member.agent().name(), "claude-1");
    assert_eq!(member.past_holders().len(), 0, "nothing was handed over");
    assert!(
        !bus.contains("WORK OF A MEMBER THAT IS GONE"),
        "the lead is not given the work: {bus}"
    );
}
