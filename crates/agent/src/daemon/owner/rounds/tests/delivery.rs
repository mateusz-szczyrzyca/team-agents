//! A coordinator's already-merged claim completes the next item without publishing again.

use agent_core::state::{Completion, ItemPhase, MergeMode, RunState};
use agent_core::status::TaskStatus;

use super::rig::{CHAIN, Rig, investigator, normal};

#[tokio::test]
async fn delivery_claim_after_an_earlier_team_merge_finishes_two_items_with_one_pr() {
    let mut rig = Rig::new(
        &[
            investigator("inv-lead", "sleep,integrate,done:DELIVERED: #1"),
            normal("claude-1", "commit-clean,done:Already delivered"),
            investigator("inv-r1", "done:VERDICT: APPROVE\\nSUMMARY: fine"),
            investigator("inv-r2", "done:VERDICT: APPROVE\\nSUMMARY: fine"),
        ],
        5,
        true,
    )
    .await;
    rig.start().await;
    let (team, tasks) = rig
        .run(&["inv-lead", "claude-1"], CHAIN, MergeMode::Autopilot, Some("inv-lead"))
        .await;
    rig.state_where("the coordinator's third run to settle", |state| {
        let lead = state.team(&team).unwrap().find(&tasks[0]).unwrap().1;
        lead.runs().len() >= 3
            && lead
                .runs()
                .last()
                .is_some_and(|run| matches!(run.state(), RunState::Ended { .. }))
    })
    .await;
    for _ in 0..3 {
        let reply = crate::client::Connection::connect(&rig.home.socket_file())
            .await
            .unwrap()
            .request(
                agent_core::protocol::Client::Cli,
                agent_core::protocol::Command::Status(agent_core::protocol::StatusRequest { target: None }),
            )
            .await
            .unwrap();
        assert!(matches!(reply, agent_core::protocol::Reply::Teams { .. }));
    }
    let state = rig.state();
    rig.stop().await.unwrap();
    let found = state.team(&team).unwrap();
    for id in ["1.1", "1.2"] {
        let item = found.item(id.parse().unwrap()).unwrap();
        assert!(
            matches!(item.phase(), ItemPhase::Done { completion: Completion::Merged { pr, .. } } if pr.number.get().get() == 1),
            "{id}: {:?}",
            item.phase()
        );
    }
    assert_eq!(rig.github.pulls().len(), 1);
    assert_eq!(found.find(&tasks[1]).unwrap().1.runs().len(), 2);
    assert_eq!(found.find(&tasks[0]).unwrap().1.status(), TaskStatus::Done);
    assert!(found.rounds().iter().all(|round| !round.is_open()));
    assert!(!rig.worktree(&tasks[1]).exists());
}

async fn assert_delivery_refused(
    lead: &'static str,
    member: &'static str,
    reason: &str,
) -> Result<(), crate::daemon::DaemonError> {
    let mut rig = Rig::new(
        &[
            investigator("inv-lead", lead),
            normal("claude-1", member),
            investigator("inv-r1", "done:VERDICT: APPROVE\\nSUMMARY: fine"),
            investigator("inv-r2", "done:VERDICT: APPROVE\\nSUMMARY: fine"),
        ],
        5,
        true,
    )
    .await;
    rig.start().await;
    let (team, tasks) = rig
        .run(&["inv-lead", "claude-1"], CHAIN, MergeMode::Autopilot, Some("inv-lead"))
        .await;
    let state = rig
        .state_where("delivery refusal or an unexpected second publication", |state| {
            let found = state.team(&team).unwrap();
            found.find(&tasks[0]).unwrap().1.runs().len() >= 4
                || matches!(
                    found.item("1.2".parse().unwrap()).unwrap().phase(),
                    ItemPhase::Done { .. }
                )
        })
        .await;
    assert_eq!(rig.github.pulls().len(), 1, "a refused claim must not publish an item");
    let prompt = rig.prompt(&tasks[0], 4).await;
    rig.stop().await?;
    assert!(prompt.contains(reason), "{reason}: {prompt}");
    let found = state.team(&team).unwrap();
    let round = found.round_of("1.2".parse().unwrap()).unwrap();
    assert_eq!(round.delivered_by(), None);
    assert!(round.is_open());
    assert!(matches!(
        found.item("1.2".parse().unwrap()).unwrap().phase(),
        ItemPhase::InProgress { .. }
    ));
    assert_eq!(rig.github.pulls().len(), 1);
    Ok(())
}

#[tokio::test]
async fn delivery_refuses_an_unknown_team_pr_and_tells_the_lead_why() {
    assert_delivery_refused(
        "sleep,integrate,done:DELIVERED: #99,sleep",
        "commit-clean,done:Already delivered",
        "PR #99 belongs to no other round of this team",
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn delivery_refuses_private_commits_and_tells_the_lead_why() {
    assert_delivery_refused(
        "sleep,integrate,done:DELIVERED: #1,sleep",
        "commit-clean,work",
        "the item has commits outside the PR and base branches",
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn delivery_refuses_a_dirty_worktree_and_tells_the_lead_why() {
    assert_delivery_refused(
        "sleep,integrate,done:DELIVERED: #1,sleep",
        "commit-clean,dirty-done",
        "the item's worktree has uncommitted changes",
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn delivery_open_claim_survives_restart_and_closes_on_its_pr_merge() {
    let mut rig = Rig::new(
        &[
            investigator("inv-lead", "sleep,integrate,done:DELIVERED: #1,sleep"),
            normal("claude-1", "commit-clean"),
            normal("codex-1", "delivery-held"),
            investigator("inv-r1", "done:VERDICT: APPROVE\\nSUMMARY: fine"),
            investigator("inv-r2", "done:VERDICT: APPROVE\\nSUMMARY: fine"),
        ],
        5,
        true,
    )
    .await;
    rig.start().await;
    let (team, tasks) = rig
        .run(
            &["inv-lead", "claude-1", "codex-1"],
            super::rig::PARALLEL,
            MergeMode::NoMerge,
            Some("inv-lead"),
        )
        .await;
    rig.item_where("1.1", "the owning PR to be approved", |phase| {
        matches!(phase, ItemPhase::Approved { .. })
    })
    .await;
    std::fs::write(rig.home.root().join("delivery-release"), "release").unwrap();
    let waiting = rig
        .state_where("the open claim to be checked", |state| {
            let found = state.team(&team).unwrap();
            found.round_of("2.1".parse().unwrap()).unwrap().delivered_by().is_some()
                || found.find(&tasks[0]).unwrap().1.runs().len() >= 4
        })
        .await;
    let found = waiting.team(&team).unwrap();
    let round = found.round_of("2.1".parse().unwrap()).unwrap();
    assert_eq!(round.delivered_by().map(|pr| pr.number.get().get()), Some(1));
    assert!(round.is_open());
    assert_eq!(found.find(&tasks[2]).unwrap().1.runs().len(), 1);
    rig.stop().await.unwrap();
    rig.start().await;
    let before = rig
        .state_where("restarted waiting member", |state| {
            state.team(&team).unwrap().find(&tasks[2]).unwrap().1.status() == TaskStatus::Waiting
        })
        .await;
    assert!(
        before
            .team(&team)
            .unwrap()
            .round_of("2.1".parse().unwrap())
            .unwrap()
            .is_open()
    );
    rig.github.merged_by_a_human(1);
    rig.item_where("1.1", "the owning PR to merge", |phase| {
        matches!(phase, ItemPhase::Done { .. })
    })
    .await;
    for _ in 0..3 {
        let reply = crate::client::Connection::connect(&rig.home.socket_file())
            .await
            .unwrap()
            .request(
                agent_core::protocol::Client::Cli,
                agent_core::protocol::Command::Status(agent_core::protocol::StatusRequest { target: None }),
            )
            .await
            .unwrap();
        assert!(matches!(reply, agent_core::protocol::Reply::Teams { .. }));
    }
    let done = rig.state();
    rig.stop().await.unwrap();
    let found = done.team(&team).unwrap();
    assert!(
        found
            .items()
            .iter()
            .all(|item| matches!(item.phase(), ItemPhase::Done { .. }))
    );
    assert!(found.rounds().iter().all(|round| !round.is_open()));
    assert_eq!(found.find(&tasks[0]).unwrap().1.status(), TaskStatus::Done);
    assert_eq!(found.find(&tasks[2]).unwrap().1.runs().len(), 1);
    assert_eq!(rig.github.pulls().len(), 1);
    assert!(!rig.worktree(&tasks[2]).exists());
}

#[tokio::test]
async fn delivery_waits_through_a_stop_and_resume_and_closes_at_its_pr_merge_with_one_pr() {
    use agent_core::protocol::{Client, Command, ControlRequest, Reply, Target};
    let mut rig = Rig::new(
        &[
            investigator("inv-lead", "sleep,integrate,done:DELIVERED: #1,sleep"),
            normal("claude-1", "commit-clean"),
            normal("codex-1", "delivery-held"),
            investigator("inv-r1", "done:VERDICT: APPROVE\\nSUMMARY: fine"),
            investigator("inv-r2", "done:VERDICT: APPROVE\\nSUMMARY: fine"),
        ],
        5,
        true,
    )
    .await;
    rig.start().await;
    let (team, tasks) = rig
        .run(
            &["inv-lead", "claude-1", "codex-1"],
            super::rig::PARALLEL,
            MergeMode::NoMerge,
            Some("inv-lead"),
        )
        .await;
    rig.item_where("1.1", "the owning PR to be approved", |phase| {
        matches!(phase, ItemPhase::Approved { .. })
    })
    .await;
    std::fs::write(rig.home.root().join("delivery-release"), "release").unwrap();
    rig.state_where("the open claim to be accepted", |state| {
        state
            .team(&team)
            .unwrap()
            .round_of("2.1".parse().unwrap())
            .unwrap()
            .delivered_by()
            .is_some()
    })
    .await;
    let reply = crate::client::Connection::connect(&rig.home.socket_file())
        .await
        .unwrap()
        .request(
            Client::Cli,
            Command::Stop(ControlRequest {
                target: Target::Team(team.clone()),
            }),
        )
        .await
        .unwrap();
    assert!(matches!(reply, Reply::Applied { .. }), "{reply:?}");
    let stopped = rig
        .state_where("the stop to abandon the delivered round", |state| {
            !state
                .team(&team)
                .unwrap()
                .round_of("2.1".parse().unwrap())
                .unwrap()
                .is_open()
        })
        .await;
    let round = stopped.team(&team).unwrap().round_of("2.1".parse().unwrap()).unwrap();
    assert_eq!(round.delivered_by().map(|pr| pr.number.get().get()), Some(1));
    for task in &tasks {
        rig.resume(task).await;
    }
    rig.github.merged_by_a_human(1);
    rig.item_where("2.1", "the delivering PR's merge to close the item", |phase| {
        matches!(phase, ItemPhase::Done { .. })
    })
    .await;
    let done = rig.state();
    rig.stop().await.unwrap();
    let found = done.team(&team).unwrap();
    let item = found.item("2.1".parse().unwrap()).unwrap();
    assert!(
        matches!(item.phase(), ItemPhase::Done { completion: Completion::Merged { pr, .. } } if pr.number.get().get() == 1),
        "{:?}",
        item.phase()
    );
    assert_eq!(
        rig.github.pulls().len(),
        1,
        "the delivered item must not publish a second PR"
    );
    assert_eq!(
        found
            .rounds()
            .iter()
            .filter(|round| round.item() == Some("2.1".parse().unwrap()))
            .count(),
        1
    );
}

/// Runs the scenario of an open delivering PR #1 whose item 1.1 is approved and whose member of item 2.1 still holds,
/// lets `spoil` change what GitHub or the remote says about it, then lets the coordinator claim `claim` for 2.1.
/// The claim must be refused with `reason` in the lead's next prompt, and the round of 2.1 must stay open and unclaimed.
async fn assert_open_claim_refused(
    lead: &'static str,
    members: [&'static str; 2],
    spoil: impl FnOnce(&Rig),
    reason: &str,
) {
    let mut rig = Rig::new(
        &[
            investigator("inv-lead", lead),
            normal("claude-1", members[0]),
            normal("codex-1", members[1]),
            investigator("inv-r1", "done:VERDICT: APPROVE\\nSUMMARY: fine"),
            investigator("inv-r2", "done:VERDICT: APPROVE\\nSUMMARY: fine"),
        ],
        5,
        true,
    )
    .await;
    rig.start().await;
    let (team, tasks) = rig
        .run(
            &["inv-lead", "claude-1", "codex-1"],
            super::rig::PARALLEL,
            MergeMode::NoMerge,
            Some("inv-lead"),
        )
        .await;
    rig.item_where("1.1", "the owning PR to be approved", |phase| {
        matches!(phase, ItemPhase::Approved { .. })
    })
    .await;
    spoil(&rig);
    std::fs::write(rig.home.root().join("delivery-release"), "release").unwrap();
    let state = rig
        .state_where("the refusal to reach the lead", |state| {
            let found = state.team(&team).unwrap();
            found.find(&tasks[0]).unwrap().1.runs().len() >= 4
                || found.round_of("2.1".parse().unwrap()).unwrap().delivered_by().is_some()
        })
        .await;
    let prompt = rig.prompt(&tasks[0], 4).await;
    rig.stop().await.unwrap();
    assert!(prompt.contains(reason), "{reason}: {prompt}");
    let found = state.team(&team).unwrap();
    let round = found.round_of("2.1".parse().unwrap()).unwrap();
    assert_eq!(round.delivered_by(), None);
    assert!(round.is_open());
    assert!(matches!(
        found.item("2.1".parse().unwrap()).unwrap().phase(),
        ItemPhase::InProgress { .. }
    ));
}

#[tokio::test]
async fn delivery_refuses_an_existing_pr_of_another_team_and_tells_the_lead_why() {
    assert_open_claim_refused(
        "sleep,integrate,done:DELIVERED: #2,sleep",
        ["commit-clean", "delivery-held"],
        |rig| {
            super::rig::git(&rig.origin, &["branch", "foreign-work", "main"]);
            assert_eq!(rig.github.foreign_pull("foreign-work"), 2);
        },
        "PR #2 belongs to no other round of this team",
    )
    .await;
}

#[tokio::test]
async fn delivery_refuses_a_pr_that_no_longer_uses_its_recorded_branch() {
    assert_open_claim_refused(
        "sleep,integrate,done:DELIVERED: #1,sleep",
        ["commit-clean", "delivery-held"],
        |rig| {
            let head = rig.github.pulls()[0].head.clone();
            super::rig::git(&rig.origin, &["branch", "moved-head", &format!("refs/heads/{head}")]);
            rig.github.head_changed_by_a_human(1, "moved-head");
        },
        "the delivering pull request does not use its recorded team branch",
    )
    .await;
}

#[tokio::test]
async fn delivery_refuses_a_pr_closed_without_a_merge() {
    assert_open_claim_refused(
        "sleep,integrate,done:DELIVERED: #1,sleep",
        ["commit-clean", "delivery-held"],
        |rig| rig.github.closed_by_a_human(1),
        "the delivering pull request was closed without merging",
    )
    .await;
}

#[tokio::test]
async fn delivery_refuses_work_that_the_pr_has_on_no_published_commit() {
    assert_open_claim_refused(
        "sleep,integrate,done:DELIVERED: #1,sleep",
        ["commit-clean", "adopt-held"],
        |rig| {
            let head = rig.github.pulls()[0].head.clone();
            let base = super::rig::git(&rig.origin, &["rev-parse", "refs/heads/main"]);
            super::rig::git(&rig.origin, &["update-ref", &format!("refs/heads/{head}"), &base]);
        },
        "the item has commits outside the PR and base branches",
    )
    .await;
}

#[tokio::test]
async fn delivery_closed_claim_reopens_the_round_and_runs_the_member_again() {
    let mut rig = Rig::new(
        &[
            investigator(
                "inv-lead",
                "sleep,integrate,done:DELIVERED: #1,delivery-return-lead,sleep",
            ),
            normal("claude-1", "commit-clean"),
            normal("codex-1", "delivery-held,delivery-return-work"),
            investigator("inv-r1", "done:VERDICT: APPROVE\\nSUMMARY: fine"),
            investigator("inv-r2", "done:VERDICT: APPROVE\\nSUMMARY: fine"),
        ],
        5,
        true,
    )
    .await;
    rig.start().await;
    let (team, tasks) = rig
        .run(
            &["inv-lead", "claude-1", "codex-1"],
            super::rig::PARALLEL,
            MergeMode::NoMerge,
            Some("inv-lead"),
        )
        .await;
    rig.item_where("1.1", "the first PR to be approved", |phase| {
        matches!(phase, ItemPhase::Approved { .. })
    })
    .await;
    std::fs::write(rig.home.root().join("delivery-release"), "release").unwrap();
    let accepted = rig
        .state_where("the delivery to be accepted", |state| {
            state
                .team(&team)
                .unwrap()
                .round_of("2.1".parse().unwrap())
                .unwrap()
                .delivered_by()
                .is_some()
        })
        .await;
    let old = accepted
        .team(&team)
        .unwrap()
        .round_of("2.1".parse().unwrap())
        .unwrap()
        .id()
        .clone();
    rig.github.closed_by_a_human(1);
    let returned = rig
        .state_where("the returned member to complete its next run", |state| {
            let found = state.team(&team).unwrap();
            found.find(&tasks[2]).unwrap().1.runs().len() >= 2
                && found.find(&tasks[2]).unwrap().1.status() == TaskStatus::Waiting
        })
        .await;
    rig.prompt(&tasks[0], 4).await;
    let messages = std::fs::read_to_string(rig.home.root().join("bus/messages.jsonl")).unwrap();
    let found = returned.team(&team).unwrap();
    assert!(found.round(&old).unwrap().delivery_closed_at().is_some());
    let fresh = found.round_of("2.1".parse().unwrap()).unwrap();
    assert_ne!(fresh.id(), &old);
    assert_eq!(fresh.delivered_by(), None);
    assert!(fresh.is_open());
    assert_eq!(
        found.find(&tasks[2]).unwrap().1.runs().last().unwrap().rounds(),
        &[fresh.id().clone()]
    );
    assert!(messages.contains("PR #1 was closed without merging"), "{messages}");
    rig.item_where("2.1", "the returned item's independent PR to be approved", |phase| {
        matches!(phase, ItemPhase::Approved { .. })
    })
    .await;
    assert_eq!(rig.github.pulls().len(), 2);
    rig.github.merged_by_a_human(2);
    rig.item_where(
        "2.1",
        "the returned item to finish while its old PR stays closed",
        |phase| matches!(phase, ItemPhase::Done { .. }),
    )
    .await;
    push_integrated_owning_pr(&rig, &team, &tasks[0]);
    rig.github.reopened_by_a_human(1);
    rig.github.merged_by_a_human(1);
    let done = rig
        .state_where("the team to finish after both independent merges", |state| {
            state.team(&team).unwrap().find(&tasks[0]).unwrap().1.status() == TaskStatus::Done
        })
        .await;
    rig.stop().await.unwrap();
    assert!(
        done.team(&team)
            .unwrap()
            .items()
            .iter()
            .all(|item| matches!(item.phase(), ItemPhase::Done { .. }))
    );
    assert!(done.team(&team).unwrap().rounds().iter().all(|round| !round.is_open()));
}

/// Updates the unrelated PR only after the returned item's independent merge.
fn push_integrated_owning_pr(rig: &Rig, team: &agent_core::ids::TeamId, lead: &agent_core::ids::TaskId) {
    let state = rig.state();
    let ItemPhase::Approved { pr, .. } = state.team(team).unwrap().item("1.1".parse().unwrap()).unwrap().phase() else {
        panic!("the unrelated owning PR still waits")
    };
    super::rig::git(&rig.worktree(lead), &["push", "origin", pr.branch.as_str()]);
}
