//! The plan of a solo agent through a round: a run that ends done with a step to do goes back, one that then loses a
//! step goes back again, and the run that settles its plan goes on; every version is kept.

use agent_core::state::{MergeMode, PlanStage};
use agent_core::status::TaskStatus;
use agent_runtime::history::{PlanRead, Reader};

use super::rig::{Rig, investigator, normal};

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
