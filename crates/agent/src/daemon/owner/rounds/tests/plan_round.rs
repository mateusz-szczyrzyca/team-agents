//! The plan of a solo agent through a round: a run that ends done with a step to do goes back, one that then loses a
//! step goes back again, and the run that settles its plan goes on; every version is kept.

use agent_core::state::{ItemPhase, MergeMode, PlanStage};
use agent_core::status::TaskStatus;
use agent_runtime::history::{PlanRead, Reader};

use super::rig::{ONE, Rig, investigator, normal};

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
