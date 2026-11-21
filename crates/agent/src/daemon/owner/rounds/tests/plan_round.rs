//! The plan of a solo agent through a round: a run that ends done with a step to do goes back, one that then loses a
//! step goes back again, and the run that settles its plan goes on; every version is kept.

use agent_core::protocol::{Client, Command, Reply, StatusRequest};
use agent_core::state::{ItemPhase, MergeMode, PlanKeeping, PlanStage};
use agent_core::status::TaskStatus;
use agent_runtime::history::{PlanRead, Reader};

use super::rig::{ONE, Rig, investigator, normal};
use crate::client::Connection;

/// Writes the script `name` of the stand-in CLI, which writes `plan` as the plan of the worktree it runs in.
fn plan_script(rig: &Rig, name: &str, plan: &str) {
    let dir = rig.home.root().join("fake-scripts");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join(format!("{name}.sh")),
        format!("mkdir -p .agent\ncat > .agent/plan.md <<'PLAN'\n{plan}PLAN\n"),
    )
    .unwrap();
}

#[tokio::test]
async fn a_run_goes_back_for_a_step_to_do_then_for_a_lost_step_and_goes_on_when_its_plan_is_settled() {
    let mut rig = Rig::new(
        &[
            normal("claude-1", "script:unfinished,script:lost,script:settled"),
            investigator("inv-r1", "sleep"),
            investigator("inv-r2", "sleep"),
        ],
        5,
        false,
    )
    .await;
    let config = std::fs::read_to_string(rig.home.config_file()).unwrap();
    std::fs::write(
        rig.home.config_file(),
        config.replace("[plan]\nbounces = 0\n", "[plan]\nbounces = 3\n"),
    )
    .unwrap();
    plan_script(
        &rig,
        "unfinished",
        "Fix it — zażółć.\n\n1. Work\n[x] 1.1 Find it\n[ ] 1.2 Fix it\n",
    );
    plan_script(&rig, "lost", "Fix it — zażółć.\n\n1. Work\n[x] 1.1 Find it\n");
    plan_script(
        &rig,
        "settled",
        "Fix it — zażółć.\n\n1. Work\n[x] 1.1 Find it\n[-] 1.2 Fix it\n    It was fixed by 1.1.\n",
    );
    rig.start().await;

    let (_, tasks) = rig.run(&["claude-1"], "Fix the typo.", MergeMode::NoMerge, None).await;
    let second = rig.prompt(&tasks[0], 2).await;
    let third = rig.prompt(&tasks[0], 3).await;
    let state = rig
        .state_where("the plan settled and the task past its runs", |state| {
            state.find_task(&tasks[0]).is_some_and(|found| {
                found.member.status() != TaskStatus::Running
                    && found.member.status() != TaskStatus::Queued
                    && found.member.runs().len() == 3
            })
        })
        .await;
    rig.stop().await.unwrap();

    assert!(
        second.contains("are still to do or in hand: 1.2. Finish each"),
        "{second}"
    );
    assert!(
        third.contains("another step now: 1.2. A step is never deleted"),
        "{third}"
    );
    let keeping = state.find_task(&tasks[0]).unwrap().member.plan().unwrap().clone();
    assert_eq!(
        (keeping.sent_back(), keeping.stage(), keeping.pending()),
        (2, PlanStage::Open, None)
    );
    let versions = Reader::open(&rig.home.history_file())
        .unwrap()
        .plan_versions(&tasks[0])
        .unwrap();
    let texts: Vec<_> = versions
        .iter()
        .map(|record| match &record.read {
            PlanRead::Version { text } => text.lines().last().unwrap_or_default().to_owned(),
            other => panic!("a version was expected: {other:?}"),
        })
        .collect();
    assert_eq!(texts, ["[ ] 1.2 Fix it", "[x] 1.1 Find it", "    It was fixed by 1.1."]);
}

#[tokio::test]
async fn the_same_done_after_a_send_back_for_a_lost_step_goes_back_again() {
    let mut rig = Rig::new(
        &[
            normal("claude-1", "script:unfinished,script:lost,script:lost,script:settled"),
            investigator("inv-r1", "sleep"),
            investigator("inv-r2", "sleep"),
        ],
        5,
        false,
    )
    .await;
    let config = std::fs::read_to_string(rig.home.config_file()).unwrap();
    std::fs::write(
        rig.home.config_file(),
        config.replace("[plan]\nbounces = 0\n", "[plan]\nbounces = 3\n"),
    )
    .unwrap();
    plan_script(
        &rig,
        "unfinished",
        "Fix it.\n\n1. Work\n[x] 1.1 Find it\n[ ] 1.2 Fix it\n",
    );
    plan_script(&rig, "lost", "Fix it.\n\n1. Work\n[x] 1.1 Find it\n");
    plan_script(
        &rig,
        "settled",
        "Fix it.\n\n1. Work\n[x] 1.1 Find it\n[-] 1.2 Fix it\n    It was fixed by 1.1.\n",
    );
    rig.start().await;

    let (_, tasks) = rig.run(&["claude-1"], "Fix the typo.", MergeMode::NoMerge, None).await;
    let fourth = rig.prompt(&tasks[0], 4).await;
    let state = rig
        .state_where("the plan settled after four runs", |state| {
            state.find_task(&tasks[0]).is_some_and(|found| {
                found.member.status() != TaskStatus::Running
                    && found.member.status() != TaskStatus::Queued
                    && found.member.runs().len() == 4
            })
        })
        .await;
    rig.stop().await.unwrap();

    assert!(
        fourth.contains("another step now: 1.2. A step is never deleted"),
        "{fourth}"
    );
    let keeping = state.find_task(&tasks[0]).unwrap().member.plan().unwrap().clone();
    assert_eq!((keeping.sent_back(), keeping.pending()), (3, None));
}

/// A reviewer that asks for changes the first time and approves afterwards.
const CHANGES_ONCE: &str = "verify:VERDICT: REQUEST_CHANGES\\nFINDINGS\\n1. [MAJOR] TDD-2 src/lib.rs:7 — the case is untested → add coverage\\n   CHECK: read src/lib.rs:7,verify:VERDICT: APPROVE\\nSUMMARY: fixed";

/// A reviewer that approves.
const APPROVING: &str = "verify:VERDICT: APPROVE\\nSUMMARY: fine";

/// Writes the script `name` of the stand-in CLI, which writes `plan` as the plan of the worktree and commits `file`.
fn committing_plan_script(rig: &Rig, name: &str, plan: &str, file: &str) {
    let dir = rig.home.root().join("fake-scripts");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join(format!("{name}.sh")),
        format!(
            "set -eu\nmkdir -p .agent\ncat > .agent/plan.md <<'PLAN'\n{plan}PLAN\necho {name} > {file}\ngit add {file}\n\
             git -c user.name=Fake -c user.email=fake@example.com -c commit.gpgsign=false commit -q -m {name}\n"
        ),
    )
    .unwrap();
}

#[tokio::test]
async fn a_fix_pass_held_for_its_plan_is_not_taken_as_the_fix_response() {
    let mut rig = Rig::new(
        &[
            normal("claude-1", "script:first,fix-script:open,fix-held:1"),
            investigator("inv-r1", CHANGES_ONCE),
            investigator("inv-r2", APPROVING),
        ],
        5,
        false,
    )
    .await;
    let config = std::fs::read_to_string(rig.home.config_file()).unwrap();
    std::fs::write(
        rig.home.config_file(),
        config.replace("[plan]\nbounces = 0\n", "[plan]\nbounces = 3\n"),
    )
    .unwrap();
    let finished = "Do one.\n\n1. Work\n[x] 1.1 Write it\n";
    committing_plan_script(&rig, "first", finished, "one.txt");
    committing_plan_script(
        &rig,
        "open",
        "Do one.\n\n1. Work\n[x] 1.1 Write it\n[ ] 1.2 Cover the case\n",
        "two.txt",
    );
    rig.start().await;

    let (_, tasks) = rig.run(&["claude-1"], ONE, MergeMode::Autopilot, None).await;
    let worktree = rig.worktree(&tasks[0]);
    rig.wait_for("the third run to start", || worktree.join(".fake-started-3").exists())
        .await;
    let held = rig.state();
    std::fs::write(
        worktree.join(".agent/plan.md"),
        "Do one.\n\n1. Work\n[x] 1.1 Write it\n[x] 1.2 Cover the case\n",
    )
    .unwrap();
    std::fs::write(worktree.join(".fake-release-3"), "").unwrap();
    rig.state_where("the third run to end", |state| {
        state
            .find_task(&tasks[0])
            .is_some_and(|found| found.member.runs().len() == 3 && found.member.current_run().is_none())
    })
    .await;
    rig.stop().await.unwrap();

    let team = held.teams().first().unwrap();
    let phase = team.items()[0].phase().clone();
    assert!(matches!(phase, ItemPhase::Fixing { .. }), "{phase:?}");
    let keeping = held.find_task(&tasks[0]).unwrap().member.plan().unwrap().clone();
    assert_eq!(keeping.sent_back(), 1);
}

#[tokio::test]
async fn a_fix_pass_held_for_its_plan_is_not_taken_as_the_fix_response_of_a_rotating_verification() {
    let mut rig = Rig::new(
        &[
            normal("claude-1", "script:first,fix-script:open,fix-held:1"),
            investigator("inv-r1", "rotating-verify"),
            investigator("inv-r2", "rotating-verify"),
        ],
        5,
        false,
    )
    .await;
    let config = std::fs::read_to_string(rig.home.config_file()).unwrap();
    std::fs::write(
        rig.home.config_file(),
        config.replace("[plan]\nbounces = 0\n", "[plan]\nbounces = 3\n"),
    )
    .unwrap();
    let finished = "Do one.\n\n1. Work\n[x] 1.1 Write it\n";
    committing_plan_script(&rig, "first", finished, "one.txt");
    committing_plan_script(
        &rig,
        "open",
        "Do one.\n\n1. Work\n[x] 1.1 Write it\n[ ] 1.2 Cover the case\n",
        "two.txt",
    );
    rig.start().await;

    let (_, tasks) = rig.run(&["claude-1"], ONE, MergeMode::Autopilot, None).await;
    let worktree = rig.worktree(&tasks[0]);
    rig.wait_for("the third run to start", || worktree.join(".fake-started-3").exists())
        .await;
    let held = rig.state();
    std::fs::write(
        worktree.join(".agent/plan.md"),
        "Do one.\n\n1. Work\n[x] 1.1 Write it\n[x] 1.2 Cover the case\n",
    )
    .unwrap();
    std::fs::write(worktree.join(".fake-release-3"), "").unwrap();
    rig.state_where("the third run to end", |state| {
        state
            .find_task(&tasks[0])
            .is_some_and(|found| found.member.runs().len() == 3 && found.member.current_run().is_none())
    })
    .await;
    rig.stop().await.unwrap();

    let team = held.teams().first().unwrap();
    let phase = team.items()[0].phase().clone();
    assert!(matches!(phase, ItemPhase::Fixing { .. }), "{phase:?}");
    let keeping = held.find_task(&tasks[0]).unwrap().member.plan().unwrap().clone();
    assert_eq!(keeping.sent_back(), 1);
}

#[tokio::test]
async fn a_coordinators_done_after_it_integrated_the_last_member_is_sent_back_for_its_open_steps() {
    let mut rig = Rig::new(
        &[
            investigator("inv-lead", "sleep,integrate,integrate"),
            normal("claude-1", "fix-held:1"),
            investigator("inv-r1", APPROVING),
            investigator("inv-r2", APPROVING),
        ],
        5,
        true,
    )
    .await;
    let config = std::fs::read_to_string(rig.home.config_file()).unwrap();
    std::fs::write(
        rig.home.config_file(),
        config.replace("[plan]\nbounces = 0\n", "[plan]\nbounces = 3\n"),
    )
    .unwrap();
    rig.start().await;

    let (_, tasks) = rig
        .run(&["inv-lead", "claude-1"], ONE, MergeMode::Autopilot, Some("inv-lead"))
        .await;
    let lead = rig.worktree(&tasks[0]);
    std::fs::create_dir_all(lead.join(".agent")).unwrap();
    std::fs::write(
        lead.join(".agent/plan.md"),
        "Lead it — zażółć.\n\n1. Integrate\n[x] 1.1 Merge one\n[ ] 1.2 Check the merge\n",
    )
    .unwrap();
    let member = rig.worktree(&tasks[1]);
    rig.wait_for("the member's run to start", || member.join(".fake-started-1").exists())
        .await;
    std::fs::create_dir_all(member.join(".agent")).unwrap();
    std::fs::write(member.join(".agent/plan.md"), "Do one.\n\n1. Work\n[x] 1.1 Write it\n").unwrap();
    std::fs::write(member.join(".fake-release-1"), "").unwrap();
    let third = rig.prompt(&tasks[0], 3).await;
    rig.stop().await.unwrap();

    assert!(
        third.contains("are still to do or in hand: 1.2. Finish each"),
        "{third}"
    );
}

/// A rig whose solo agent runs `lead` and whose two reviewers approve, holding runs to their plan three times.
async fn approved_round(lead: &'static str) -> Rig {
    let rig = Rig::new(
        &[
            normal("claude-1", lead),
            investigator("inv-r1", APPROVING),
            investigator("inv-r2", APPROVING),
        ],
        5,
        false,
    )
    .await;
    let config = std::fs::read_to_string(rig.home.config_file()).unwrap();
    std::fs::write(
        rig.home.config_file(),
        config.replace("[plan]\nbounces = 0\n", "[plan]\nbounces = 3\n"),
    )
    .unwrap();
    rig
}

#[tokio::test]
async fn reviewers_that_write_no_plan_are_not_sent_back() {
    let mut rig = approved_round("script:first").await;
    committing_plan_script(&rig, "first", "Do one.\n\n1. Work\n[x] 1.1 Write it\n", "one.txt");
    rig.start().await;

    let (team, _) = rig.run(&["claude-1"], ONE, MergeMode::Autopilot, None).await;
    let state = rig
        .item_where("1.1", "done", |phase| matches!(phase, ItemPhase::Done { .. }))
        .await;
    rig.stop().await.unwrap();

    let reviewers = state.team(&team).unwrap().reviewers();
    assert_eq!(reviewers.len(), 2);
    for reviewer in reviewers {
        assert_eq!(reviewer.runs().len(), 1, "{:?}", reviewer.task());
        assert_eq!(reviewer.plan().map(PlanKeeping::sent_back).unwrap_or_default(), 0);
    }
}

#[tokio::test]
async fn a_plan_committed_by_mistake_is_not_in_the_published_branch() {
    let mut rig = approved_round("script:mistake,work").await;
    let dir = rig.home.root().join("fake-scripts");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("mistake.sh"),
        "set -eu\nmkdir -p .agent\nprintf 'Do one.\\n\\n1. Work\\n[x] 1.1 Write it\\n' > .agent/plan.md\n\
         echo one > one.txt\ngit add one.txt\ngit add -f .agent/plan.md\n\
         git -c user.name=Fake -c user.email=fake@example.com -c commit.gpgsign=false commit -q -m 'Work, and the plan by mistake'\n",
    )
    .unwrap();
    rig.start().await;

    let (_, tasks) = rig.run(&["claude-1"], ONE, MergeMode::Autopilot, None).await;
    rig.item_where("1.1", "done", |phase| matches!(phase, ItemPhase::Done { .. }))
        .await;
    rig.stop().await.unwrap();

    let branch = format!("agents/claude-1/{}-1.1", tasks[0]);
    let published = super::rig::git(
        &rig.origin,
        &["ls-tree", "-r", "--name-only", &format!("refs/heads/{branch}")],
    );
    assert!(published.lines().any(|path| path == "one.txt"), "{published}");
    assert!(!published.contains(".agent"), "{published}");
}

async fn round_with_changes(lead: &'static str) -> Rig {
    let rig = Rig::new(
        &[
            normal("claude-1", lead),
            investigator("inv-r1", CHANGES_ONCE),
            investigator("inv-r2", APPROVING),
        ],
        5,
        false,
    )
    .await;
    let config = std::fs::read_to_string(rig.home.config_file()).unwrap();
    std::fs::write(
        rig.home.config_file(),
        config.replace("[plan]\nbounces = 0\n", "[plan]\nbounces = 3\n"),
    )
    .unwrap();
    rig
}

fn committing_plan_script_waiting(rig: &Rig, name: &str, plan: &str, file_to_create: &str, wait_file: &str) {
    let dir = rig.home.root().join("fake-scripts");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join(format!("{name}.sh")),
        format!(
            "set -eu\nwhile [ ! -f {wait_file} ]; do sleep 0.05; done\n\
             mkdir -p .agent\ncat > .agent/plan.md <<'PLAN'\n{plan}PLAN\n\
             echo ok > {file_to_create}\ngit add {file_to_create}\n\
             git -c user.name=Fake -c user.email=fake@example.com -c commit.gpgsign=false commit -q -m 'Work'\n"
        ),
    )
    .unwrap();
}

fn example_repo() -> agent_core::config::RepoName {
    "example".parse().unwrap()
}

#[tokio::test]
async fn plan_steps_are_reported_in_status_summaries() {
    let mut rig = approved_round("script:first").await;
    committing_plan_script(
        &rig,
        "first",
        "Do one.\n\n1. Work\n[x] 1.1 Write it\n    Check: exists one.txt\n",
        "one.txt",
    );
    rig.start().await;

    let (team, tasks) = rig.run(&["claude-1"], ONE, MergeMode::Autopilot, None).await;
    rig.item_where("1.1", "done", |phase| matches!(phase, ItemPhase::Done { .. }))
        .await;

    let mut connection = Connection::connect(&rig.home.socket_file()).await.unwrap();
    let reply = connection
        .request(Client::Cli, Command::Status(StatusRequest { target: None }))
        .await
        .unwrap();

    let (Reply::Teams { teams } | Reply::Overview { teams, .. }) = reply else {
        panic!("expected Reply::Teams, got {reply:?}");
    };

    let team_summary = teams.iter().find(|t| t.id == team).unwrap();
    let steps = team_summary.task_plans.get(&tasks[0]).unwrap();
    assert_eq!(steps.len(), 1);
    assert_eq!(steps[0].id, "1.1".parse().unwrap());
    assert_eq!(serde_json::to_value(steps[0].mark).unwrap(), serde_json::json!("x"));
    assert_eq!(steps[0].state, Some(agent_core::plan::judge::JudgementState::Confirmed));

    rig.stop().await.unwrap();
}

#[tokio::test]
async fn plan_steps_are_preserved_in_status_after_daemon_restart() {
    let mut rig = approved_round("script:first").await;
    committing_plan_script(
        &rig,
        "first",
        "Do one.\n\n1. Work\n[x] 1.1 Write it\n    Check: exists one.txt\n",
        "one.txt",
    );
    rig.start().await;

    let (team, tasks) = rig.run(&["claude-1"], ONE, MergeMode::Autopilot, None).await;
    rig.item_where("1.1", "done", |phase| matches!(phase, ItemPhase::Done { .. }))
        .await;

    // Verify status before restart
    let mut connection = Connection::connect(&rig.home.socket_file()).await.unwrap();
    let reply = connection
        .request(Client::Cli, Command::Status(StatusRequest { target: None }))
        .await
        .unwrap();
    let (Reply::Teams { teams } | Reply::Overview { teams, .. }) = reply else {
        panic!("expected Reply::Teams, got {reply:?}");
    };
    let team_summary = teams.iter().find(|t| t.id == team).unwrap();
    let steps = team_summary.task_plans.get(&tasks[0]).unwrap();
    assert_eq!(steps[0].state, Some(agent_core::plan::judge::JudgementState::Confirmed));

    // Restart the daemon
    rig.stop().await.unwrap();
    rig.start().await;

    // Verify status after restart: reads from in-memory snapshot recovered at startup
    let mut connection2 = Connection::connect(&rig.home.socket_file()).await.unwrap();
    let reply2 = connection2
        .request(Client::Cli, Command::Status(StatusRequest { target: None }))
        .await
        .unwrap();
    let (Reply::Teams { teams: teams2 } | Reply::Overview { teams: teams2, .. }) = reply2 else {
        panic!("expected Reply::Teams, got {reply2:?}");
    };
    let team_summary2 = teams2.iter().find(|t| t.id == team).unwrap();
    let steps2 = team_summary2.task_plans.get(&tasks[0]).unwrap();
    assert_eq!(steps2.len(), 1);
    assert_eq!(steps2[0].id, "1.1".parse().unwrap());
    assert_eq!(serde_json::to_value(steps2[0].mark).unwrap(), serde_json::json!("x"));
    assert_eq!(
        steps2[0].state,
        Some(agent_core::plan::judge::JudgementState::Confirmed)
    );

    rig.stop().await.unwrap();
}

#[tokio::test]
async fn plan_steps_with_various_checks_report_declared_and_mismatch() {
    let mut rig = round_with_changes("script:first,fix-script:second").await;
    let plan = "\
Do several.

1. Steps
[x] 1.1 Reviewer
    Check: reviewer
[x] 1.2 No check
[x] 1.3 Command check
    Check: printf plan-check
[x] 1.4 Exists file
    Check: exists one.txt
[x] 1.5 Missing file
    Check: exists missing.txt
";
    committing_plan_script(&rig, "first", plan, "one.txt");
    committing_plan_script_waiting(&rig, "second", plan, "missing.txt", "continue.signal");
    rig.start().await;

    let (team, tasks) = rig.run(&["claude-1"], ONE, MergeMode::Autopilot, None).await;

    // Wait until run 1 has ended
    rig.state_where("run 1 has ended", |state| {
        state.find_task(&tasks[0]).is_some_and(|f| {
            f.member.runs().len() >= 2
                || (!f.member.runs().is_empty()
                    && matches!(f.member.runs()[0].state(), agent_core::state::RunState::Ended { .. }))
        })
    })
    .await;

    let mut connection = Connection::connect(&rig.home.socket_file()).await.unwrap();
    let reply = connection
        .request(Client::Cli, Command::Status(StatusRequest { target: None }))
        .await
        .unwrap();
    let (Reply::Teams { teams } | Reply::Overview { teams, .. }) = reply else {
        panic!("expected Reply::Teams, got {reply:?}");
    };
    let team_summary = teams.into_iter().find(|t| t.id == team).unwrap();
    let steps = team_summary.task_plans.get(&tasks[0]).unwrap().clone();
    assert_eq!(steps.len(), 5);

    // 1.1 reviewer -> Declared
    assert_eq!(steps[0].id, "1.1".parse().unwrap());
    assert_eq!(steps[0].state, Some(agent_core::plan::judge::JudgementState::Declared));
    assert_eq!(steps[0].reason, None);

    // 1.2 no check -> Declared
    assert_eq!(steps[1].id, "1.2".parse().unwrap());
    assert_eq!(steps[1].state, Some(agent_core::plan::judge::JudgementState::Declared));
    assert_eq!(steps[1].reason, None);

    // 1.3 command check -> Declared in Part A
    assert_eq!(steps[2].id, "1.3".parse().unwrap());
    assert_eq!(steps[2].state, Some(agent_core::plan::judge::JudgementState::Declared));

    // 1.4 exists file present -> Confirmed
    assert_eq!(steps[3].id, "1.4".parse().unwrap());
    assert_eq!(steps[3].state, Some(agent_core::plan::judge::JudgementState::Confirmed));
    assert_eq!(steps[3].reason, None);

    // 1.5 missing file -> Mismatch in run 1
    assert_eq!(steps[4].id, "1.5".parse().unwrap());
    assert_eq!(steps[4].state, Some(agent_core::plan::judge::JudgementState::Mismatch));
    assert!(
        steps[4]
            .reason
            .as_deref()
            .unwrap_or("")
            .contains("path does not exist: missing.txt"),
        "{:?}",
        steps[4].reason
    );

    // Signal run 2 to proceed and create missing.txt
    let worktree = rig.home.worktree_dir(&example_repo(), &tasks[0]);
    std::fs::write(worktree.join("continue.signal"), "ok").unwrap();

    // Once run 2 creates missing.txt, round reaches Done and 1.5 becomes Confirmed
    rig.item_where("1.1", "done", |phase| matches!(phase, ItemPhase::Done { .. }))
        .await;

    let reply = connection
        .request(Client::Cli, Command::Status(StatusRequest { target: None }))
        .await
        .unwrap();
    let (Reply::Teams { teams } | Reply::Overview { teams, .. }) = reply else {
        panic!("expected Reply::Teams, got {reply:?}");
    };
    let team_summary = teams.into_iter().find(|t| t.id == team).unwrap();
    let steps = team_summary.task_plans.get(&tasks[0]).unwrap();
    assert_eq!(steps[4].state, Some(agent_core::plan::judge::JudgementState::Confirmed));
    assert_eq!(steps[4].reason, None);

    rig.stop().await.unwrap();
}

#[tokio::test]
async fn unchanged_confirmed_steps_are_not_rejudged_across_runs_and_roles() {
    let mut rig = round_with_changes("script:first,fix-script:second").await;
    let plan = "\
Do two.

1. Steps
[x] 1.1 First
    Check: exists one.txt
[x] 1.2 Second
    Check: exists missing.txt
";
    committing_plan_script(&rig, "first", plan, "one.txt");
    committing_plan_script_waiting(&rig, "second", plan, "missing.txt", "continue.signal");
    rig.start().await;

    let (_team, tasks) = rig.run(&["claude-1"], ONE, MergeMode::Autopilot, None).await;

    // Wait until run 1 has finished and fix pass run 2 has started
    rig.state_where("run 1 has ended", |state| {
        state.find_task(&tasks[0]).is_some_and(|f| {
            f.member.runs().len() >= 2
                || (!f.member.runs().is_empty()
                    && matches!(f.member.runs()[0].state(), agent_core::state::RunState::Ended { .. }))
        })
    })
    .await;

    // Signal run 2 to proceed and create missing.txt
    let worktree = rig.home.worktree_dir(&example_repo(), &tasks[0]);
    std::fs::write(worktree.join("continue.signal"), "ok").unwrap();

    // Run 2 completes and round reaches Done
    rig.item_where("1.1", "done", |phase| matches!(phase, ItemPhase::Done { .. }))
        .await;

    let reader = Reader::open(&rig.home.history_file()).unwrap();
    let judgements = reader.step_judgements(&tasks[0]).unwrap();

    let step1 = "1.1".parse::<agent_core::ids::ItemId>().unwrap();
    let step2 = "1.2".parse::<agent_core::ids::ItemId>().unwrap();

    // 1.1 was confirmed in run 1, remained unchanged in run 2 -> deduplicated, exactly 1 judgement in history:
    let step1_judgements: Vec<_> = judgements.iter().filter(|j| j.step == step1).collect();
    assert_eq!(step1_judgements.len(), 1, "step 1.1 must only be judged once");
    assert_eq!(
        step1_judgements[0].state,
        agent_core::plan::judge::JudgementState::Confirmed
    );

    // 1.2 was mismatch in run 1, confirmed in run 2 -> re-judged, 2 judgements in history:
    let step2_judgements: Vec<_> = judgements.iter().filter(|j| j.step == step2).collect();
    assert_eq!(step2_judgements.len(), 2, "step 1.2 mismatch must be re-judged");
    assert_eq!(
        step2_judgements[0].state,
        agent_core::plan::judge::JudgementState::Mismatch
    );
    assert_eq!(
        step2_judgements[1].state,
        agent_core::plan::judge::JudgementState::Confirmed
    );

    rig.stop().await.unwrap();
}

#[tokio::test]
async fn run_with_plan_steps_settles_normally_when_history_is_disabled() {
    let mut rig = approved_round("script:done").await;
    let config = std::fs::read_to_string(rig.home.config_file()).unwrap();
    std::fs::write(
        rig.home.config_file(),
        format!("{config}\n[history]\nenabled = false\n"),
    )
    .unwrap();
    committing_plan_script(
        &rig,
        "done",
        "Do one.\n\n1. Work\n[x] 1.1 Write it\n    Check: exists one.txt\n",
        "one.txt",
    );
    rig.start().await;

    let (_team, _tasks) = rig.run(&["claude-1"], ONE, MergeMode::Autopilot, None).await;
    rig.item_where("1.1", "done", |phase| matches!(phase, ItemPhase::Done { .. }))
        .await;

    rig.stop().await.unwrap();
}

#[tokio::test]
async fn reviewer_tasks_have_no_plan_summaries_in_status() {
    let mut rig = approved_round("script:first").await;
    committing_plan_script(&rig, "first", "Do one.\n\n1. Work\n[x] 1.1 Write it\n", "one.txt");
    rig.start().await;

    let (team, tasks) = rig.run(&["claude-1"], ONE, MergeMode::Autopilot, None).await;
    let state = rig
        .item_where("1.1", "done", |phase| matches!(phase, ItemPhase::Done { .. }))
        .await;

    let mut connection = Connection::connect(&rig.home.socket_file()).await.unwrap();
    let reply = connection
        .request(Client::Cli, Command::Status(StatusRequest { target: None }))
        .await
        .unwrap();

    let (Reply::Teams { teams } | Reply::Overview { teams, .. }) = reply else {
        panic!("expected Reply::Teams, got {reply:?}");
    };

    let team_summary = teams.iter().find(|t| t.id == team).unwrap();
    // Lead task has plan summaries:
    assert!(team_summary.task_plans.contains_key(&tasks[0]));

    // Reviewer tasks have NO plan summaries:
    let reviewers = state.team(&team).unwrap().reviewers();
    assert_eq!(reviewers.len(), 2);
    for reviewer in reviewers {
        let reviewer_steps = team_summary.task_plans.get(reviewer.task());
        assert!(reviewer_steps.is_none() || reviewer_steps.unwrap().is_empty());
    }

    rig.stop().await.unwrap();
}
