//! Tests for coordinator delivery reports (`DELIVERED: #N`).

use agent_core::decide::wait::Obligation;
use agent_core::ids::{ItemId, RoundId};
use agent_core::state::{Completion, ItemPhase, MergeMode, PrNumber, RoundEnd, RoundLife, Timestamp};

use super::rig::{PARALLEL, Rig, investigator, normal};
use crate::daemon::owner::fixture::Daemon;
use crate::daemon::owner::teams::Look;

fn pr(number: u32) -> PrNumber {
    PrNumber::new(std::num::NonZeroU32::new(number).unwrap())
}

#[tokio::test]
async fn coordinator_delivers_item_by_open_pr_of_team_transitions_round_to_delivered_and_waits() {
    let mut rig = Rig::new(
        &[
            normal("claude-1", "work,sleep"),
            investigator("inv-r1", "done:VERDICT: APPROVE\\nSUMMARY: fine"),
            investigator("inv-r2", "done:VERDICT: APPROVE\\nSUMMARY: fine"),
        ],
        5,
        false,
    )
    .await;
    rig.start().await;
    let (team, _) = rig.run(&["claude-1"], PARALLEL, MergeMode::NoMerge, None).await;
    let item2: ItemId = "2.1".parse().unwrap();
    let state = rig
        .state_where("round for 2.1 is open and 1.1 is approved", |state| {
            let Some(team) = state.team(&team) else { return false };
            let Some(item1) = team.item("1.1".parse().unwrap()) else {
                return false;
            };
            matches!(item1.phase(), ItemPhase::Approved { .. })
                && team
                    .rounds()
                    .iter()
                    .any(|r| r.item() == Some(item2) && matches!(r.life(), RoundLife::Open))
        })
        .await;
    rig.stop().await.unwrap();

    let round2: RoundId = state
        .team(&team)
        .unwrap()
        .rounds()
        .iter()
        .find(|r| r.item() == Some(item2))
        .unwrap()
        .id()
        .clone();

    let daemon = Daemon::new(&state, None);
    let (mut owner, _sinks) = daemon.bare_owner().await;
    let lead = owner.lead_of(&team).unwrap();

    let mut look = Look::default();
    let accepted = owner
        .coordinator_delivered(&team, &lead.task, pr(1), &mut look)
        .await
        .unwrap();

    assert!(accepted, "open PR #1 of same team should be accepted");

    let team_after = owner.parts.state.team(&team).unwrap();
    let round2_life = team_after.round(&round2).unwrap().life();
    assert_eq!(round2_life, &RoundLife::Delivered { pr: pr(1) });
}

#[tokio::test]
async fn coordinator_delivers_item_by_already_merged_pr_closes_item_and_round_at_once() {
    let mut rig = Rig::new(
        &[
            normal("claude-1", "work,sleep"),
            investigator("inv-r1", "done:VERDICT: APPROVE\\nSUMMARY: fine"),
            investigator("inv-r2", "done:VERDICT: APPROVE\\nSUMMARY: fine"),
        ],
        5,
        false,
    )
    .await;
    rig.start().await;
    let (team, _) = rig.run(&["claude-1"], PARALLEL, MergeMode::NoMerge, None).await;
    let item2: ItemId = "2.1".parse().unwrap();
    let mut state = rig
        .state_where("round for 2.1 is open and 1.1 is approved", |state| {
            let Some(team) = state.team(&team) else { return false };
            let Some(item1) = team.item("1.1".parse().unwrap()) else {
                return false;
            };
            matches!(item1.phase(), ItemPhase::Approved { .. })
                && team
                    .rounds()
                    .iter()
                    .any(|r| r.item() == Some(item2) && matches!(r.life(), RoundLife::Open))
        })
        .await;
    rig.stop().await.unwrap();

    let round2: RoundId = state
        .team(&team)
        .unwrap()
        .rounds()
        .iter()
        .find(|r| r.item() == Some(item2))
        .unwrap()
        .id()
        .clone();

    // Mark item 1.1 as already merged
    let item1: ItemId = "1.1".parse().unwrap();
    {
        let mut team_mut = state.team_mut(&team).unwrap();
        let ItemPhase::Approved { pr, .. } = team_mut.item(item1).unwrap().phase().clone() else {
            panic!("expected approved");
        };
        let round = team_mut.rounds().iter().find(|r| r.item() == Some(item1)).unwrap();
        let now = Timestamp::from_utc(round.opened_at().as_utc() + time::Duration::seconds(10)).unwrap();
        team_mut
            .advance_item(
                item1,
                ItemPhase::Done {
                    completion: Completion::Merged { pr, at: now },
                },
            )
            .unwrap();
    }

    let daemon = Daemon::new(&state, None);
    let (mut owner, _sinks) = daemon.bare_owner().await;
    let lead = owner.lead_of(&team).unwrap();

    let mut look = Look::default();
    let accepted = owner
        .coordinator_delivered(&team, &lead.task, pr(1), &mut look)
        .await
        .unwrap();

    assert!(accepted, "merged PR #1 of same team should be accepted");

    let team_after = owner.parts.state.team(&team).unwrap();
    let round2_life = team_after.round(&round2).unwrap().life();
    assert!(
        matches!(
            round2_life,
            RoundLife::Closed {
                how: RoundEnd::Delivered,
                ..
            }
        ),
        "round2 should be closed as Delivered, got: {round2_life:?}"
    );

    let item2_phase = team_after.item(item2).unwrap().phase();
    assert!(
        matches!(
            item2_phase,
            ItemPhase::Done {
                completion: Completion::Delivered { pr: delivered_pr, .. }
            } if *delivered_pr == pr(1)
        ),
        "item2 should be Done with Completion::Delivered, got: {item2_phase:?}"
    );
}

#[tokio::test]
async fn coordinator_delivery_refused_if_pr_does_not_belong_to_team() {
    let mut rig = Rig::new(
        &[
            normal("claude-1", "work,sleep"),
            investigator("inv-r1", "done:VERDICT: APPROVE\\nSUMMARY: fine"),
            investigator("inv-r2", "done:VERDICT: APPROVE\\nSUMMARY: fine"),
        ],
        5,
        false,
    )
    .await;
    rig.start().await;
    let (team, _) = rig.run(&["claude-1"], PARALLEL, MergeMode::NoMerge, None).await;
    let item2: ItemId = "2.1".parse().unwrap();
    let state = rig
        .state_where("round for 2.1 is open and 1.1 is approved", |state| {
            let Some(team) = state.team(&team) else { return false };
            let Some(item1) = team.item("1.1".parse().unwrap()) else {
                return false;
            };
            matches!(item1.phase(), ItemPhase::Approved { .. })
                && team
                    .rounds()
                    .iter()
                    .any(|r| r.item() == Some(item2) && matches!(r.life(), RoundLife::Open))
        })
        .await;
    rig.stop().await.unwrap();

    let round2: RoundId = state
        .team(&team)
        .unwrap()
        .rounds()
        .iter()
        .find(|r| r.item() == Some(item2))
        .unwrap()
        .id()
        .clone();

    let daemon = Daemon::new(&state, None);
    let (mut owner, _sinks) = daemon.bare_owner().await;
    let lead = owner.lead_of(&team).unwrap();

    let mut look = Look::default();
    let accepted = owner
        .coordinator_delivered(&team, &lead.task, pr(999), &mut look)
        .await
        .unwrap();

    assert!(!accepted, "PR #999 does not belong to team and should be refused");

    let team_after = owner.parts.state.team(&team).unwrap();
    let round2_life = team_after.round(&round2).unwrap().life();
    assert_eq!(round2_life, &RoundLife::Open);

    // Verify refusal message sent to lead
    assert!(
        owner.outbox.iter().any(|job| {
            job.team == team
                && matches!(&job.job, crate::daemon::team::BusJob::Message { to, text } if *to == lead.agent && text.contains("999"))
        }),
        "lead should be told why delivery report was refused, outbox: {:?}",
        owner.outbox
    );
}

#[tokio::test]
async fn coordinator_delivery_refused_if_item_worktree_is_dirty() {
    let mut rig = Rig::new(
        &[
            normal("claude-1", "work,sleep"),
            investigator("inv-r1", "done:VERDICT: APPROVE\\nSUMMARY: fine"),
            investigator("inv-r2", "done:VERDICT: APPROVE\\nSUMMARY: fine"),
        ],
        5,
        false,
    )
    .await;
    rig.start().await;
    let (team, tasks) = rig.run(&["claude-1"], PARALLEL, MergeMode::NoMerge, None).await;
    let item2: ItemId = "2.1".parse().unwrap();
    let state = rig
        .state_where("round for 2.1 is open and 1.1 is approved", |state| {
            let Some(team) = state.team(&team) else { return false };
            let Some(item1) = team.item("1.1".parse().unwrap()) else {
                return false;
            };
            matches!(item1.phase(), ItemPhase::Approved { .. })
                && team
                    .rounds()
                    .iter()
                    .any(|r| r.item() == Some(item2) && matches!(r.life(), RoundLife::Open))
        })
        .await;

    let round2: RoundId = state
        .team(&team)
        .unwrap()
        .rounds()
        .iter()
        .find(|r| r.item() == Some(item2))
        .unwrap()
        .id()
        .clone();

    rig.stop().await.unwrap();

    let daemon = Daemon::new(&state, None);
    let (mut owner, _sinks) = daemon.bare_owner().await;
    let lead = owner.lead_of(&team).unwrap();

    // Copy worktree from rig to owner's home and make it dirty
    let rig_worktree = rig.worktree(&tasks[0]);
    let target_worktree = owner.parts.store.home().worktree_dir(&lead.repo, &tasks[0]);
    std::fs::create_dir_all(target_worktree.parent().unwrap()).unwrap();
    let cp = std::process::Command::new("cp")
        .args(["-a", rig_worktree.to_str().unwrap(), target_worktree.to_str().unwrap()])
        .status()
        .unwrap();
    assert!(cp.success());
    std::fs::write(target_worktree.join("dirty.txt"), "uncommitted").unwrap();

    let mut look = Look::default();
    let accepted = owner
        .coordinator_delivered(&team, &lead.task, pr(1), &mut look)
        .await
        .unwrap();

    assert!(!accepted, "delivery report should be refused when worktree is dirty");

    let team_after = owner.parts.state.team(&team).unwrap();
    let round2_life = team_after.round(&round2).unwrap().life();
    assert_eq!(round2_life, &RoundLife::Open);

    // Verify refusal message sent to lead
    assert!(
        owner.outbox.iter().any(|job| {
            job.team == team
                && matches!(&job.job, crate::daemon::team::BusJob::Message { to, text } if *to == lead.agent && text.contains("uncommitted changes"))
        }),
        "lead should be told about dirty worktree refusal, outbox: {:?}",
        owner.outbox
    );
}

#[tokio::test]
async fn coordinator_delivery_refused_if_item_branch_holds_extra_commits() {
    let mut rig = Rig::new(
        &[
            normal("claude-1", "work,sleep"),
            investigator("inv-r1", "done:VERDICT: APPROVE\\nSUMMARY: fine"),
            investigator("inv-r2", "done:VERDICT: APPROVE\\nSUMMARY: fine"),
        ],
        5,
        false,
    )
    .await;
    rig.start().await;
    let (team, tasks) = rig.run(&["claude-1"], PARALLEL, MergeMode::NoMerge, None).await;
    let item2: ItemId = "2.1".parse().unwrap();
    let state = rig
        .state_where("round for 2.1 is open and 1.1 is approved", |state| {
            let Some(team) = state.team(&team) else { return false };
            let Some(item1) = team.item("1.1".parse().unwrap()) else {
                return false;
            };
            matches!(item1.phase(), ItemPhase::Approved { .. })
                && team
                    .rounds()
                    .iter()
                    .any(|r| r.item() == Some(item2) && matches!(r.life(), RoundLife::Open))
        })
        .await;

    let round2: RoundId = state
        .team(&team)
        .unwrap()
        .rounds()
        .iter()
        .find(|r| r.item() == Some(item2))
        .unwrap()
        .id()
        .clone();

    rig.stop().await.unwrap();

    let daemon = Daemon::new(&state, None);
    let (mut owner, _sinks) = daemon.bare_owner().await;
    let lead = owner.lead_of(&team).unwrap();

    // Copy worktree from rig to owner's home and commit extra commit on member branch
    let rig_worktree = rig.worktree(&tasks[0]);
    let target_worktree = owner.parts.store.home().worktree_dir(&lead.repo, &tasks[0]);
    std::fs::create_dir_all(target_worktree.parent().unwrap()).unwrap();
    let cp = std::process::Command::new("cp")
        .args(["-a", rig_worktree.to_str().unwrap(), target_worktree.to_str().unwrap()])
        .status()
        .unwrap();
    assert!(cp.success());

    // Clean any untracked fake logs so tree is clean
    let clean = std::process::Command::new("git")
        .args(["clean", "-fd"])
        .current_dir(&target_worktree)
        .status()
        .unwrap();
    assert!(clean.success());

    let status = std::process::Command::new("git")
        .args(["commit", "--allow-empty", "-m", "extra commit on member branch"])
        .current_dir(&target_worktree)
        .status()
        .unwrap();
    assert!(status.success());

    let mut look = Look::default();
    let accepted = owner
        .coordinator_delivered(&team, &lead.task, pr(1), &mut look)
        .await
        .unwrap();

    assert!(
        !accepted,
        "delivery report should be refused when branch holds extra commits"
    );

    let team_after = owner.parts.state.team(&team).unwrap();
    let round2_life = team_after.round(&round2).unwrap().life();
    assert_eq!(round2_life, &RoundLife::Open);

    // Verify refusal message sent to lead
    assert!(
        owner.outbox.iter().any(|job| {
            job.team == team
                && matches!(&job.job, crate::daemon::team::BusJob::Message { to, text } if *to == lead.agent && text.contains("holds commits not in pull request"))
        }),
        "lead should be told about extra commits refusal, outbox: {:?}",
        owner.outbox
    );
}

#[tokio::test]
async fn coordinator_delivery_refusal_for_pr_of_another_team_and_unknown_pr_keeps_round_open_and_tells_lead() {
    let mut rig = Rig::new(
        &[
            normal("claude-1", "work,sleep"),
            normal("claude-2", "work,sleep"),
            investigator("inv-r1", "done:VERDICT: APPROVE\\nSUMMARY: fine"),
            investigator("inv-r2", "done:VERDICT: APPROVE\\nSUMMARY: fine"),
        ],
        5,
        false,
    )
    .await;
    rig.start().await;
    // Team 1: gets PR #1 approved
    let (team1, _) = rig.run(&["claude-1"], PARALLEL, MergeMode::NoMerge, None).await;
    let item2: ItemId = "2.1".parse().unwrap();
    let _unused = rig
        .state_where("team1 round for 2.1 is open and 1.1 is approved", |state| {
            let Some(team) = state.team(&team1) else { return false };
            let Some(item1) = team.item("1.1".parse().unwrap()) else {
                return false;
            };
            matches!(item1.phase(), ItemPhase::Approved { .. })
                && team
                    .rounds()
                    .iter()
                    .any(|r| r.item() == Some(item2) && matches!(r.life(), RoundLife::Open))
        })
        .await;

    // Team 2: has round open
    let (team2, _) = rig.run(&["claude-2"], PARALLEL, MergeMode::NoMerge, None).await;
    let state = rig
        .state_where("team2 has round for 1.1 open", |state| {
            let Some(team) = state.team(&team2) else { return false };
            team.rounds().iter().any(|r| matches!(r.life(), RoundLife::Open))
        })
        .await;
    rig.stop().await.unwrap();

    let daemon = Daemon::new(&state, None);
    let (mut owner, _sinks) = daemon.bare_owner().await;
    let lead2 = owner.lead_of(&team2).unwrap();
    let round2_open = owner
        .parts
        .state
        .team(&team2)
        .unwrap()
        .rounds()
        .iter()
        .find(|r| matches!(r.life(), RoundLife::Open))
        .unwrap()
        .id()
        .clone();

    // 1. Report PR #1 (which belongs to team 1, NOT team 2): refused, round stays open, lead told reason
    let mut look = Look::default();
    let accepted_other = owner
        .coordinator_delivered(&team2, &lead2.task, pr(1), &mut look)
        .await
        .unwrap();
    assert!(!accepted_other, "PR #1 of another team must be refused");

    let team2_after_other = owner.parts.state.team(&team2).unwrap();
    assert_eq!(
        team2_after_other.round(&round2_open).unwrap().life(),
        &RoundLife::Open,
        "round must stay open after refusal of PR from another team"
    );
    assert!(
        owner.outbox.iter().any(|job| {
            job.team == team2
                && matches!(&job.job, crate::daemon::team::BusJob::Message { to, text } if *to == lead2.agent && text.contains("pull request #1 does not belong to a round of this team"))
        }),
        "lead must be told why PR #1 of another team was refused"
    );

    // 2. Report PR #999 (unknown PR number): refused, round stays open, lead told reason
    let accepted_unknown = owner
        .coordinator_delivered(&team2, &lead2.task, pr(999), &mut look)
        .await
        .unwrap();
    assert!(!accepted_unknown, "unknown PR #999 must be refused");

    let team2_after_unknown = owner.parts.state.team(&team2).unwrap();
    assert_eq!(
        team2_after_unknown.round(&round2_open).unwrap().life(),
        &RoundLife::Open,
        "round must stay open after refusal of unknown PR"
    );
    assert!(
        owner.outbox.iter().any(|job| {
            job.team == team2
                && matches!(&job.job, crate::daemon::team::BusJob::Message { to, text } if *to == lead2.agent && text.contains("pull request #999 does not belong to a round of this team"))
        }),
        "lead must be told why unknown PR #999 was refused"
    );
}

#[tokio::test]
async fn fix_pass_of_named_pull_request_starts_no_run_of_waiting_item() {
    let mut rig = Rig::new(
        &[
            normal("claude-1", "work,sleep"),
            investigator(
                "inv-r1",
                "verify:VERDICT: REQUEST_CHANGES\\nFINDINGS\\n1. [MAJOR] TDD-2 src/lib.rs:7 — untested → add coverage\\n   CHECK: read src/lib.rs:7",
            ),
            investigator("inv-r2", "done:VERDICT: APPROVE\\nSUMMARY: fine"),
        ],
        5,
        false,
    )
    .await;
    rig.start().await;
    let (team, _) = rig.run(&["claude-1"], PARALLEL, MergeMode::NoMerge, None).await;
    let item2: ItemId = "2.1".parse().unwrap();
    let state = rig
        .state_where("round for 2.1 is open and 1.1 is fixing", |state| {
            let Some(team) = state.team(&team) else { return false };
            let Some(item1) = team.item("1.1".parse().unwrap()) else {
                return false;
            };
            matches!(item1.phase(), ItemPhase::Fixing { .. })
                && team
                    .rounds()
                    .iter()
                    .any(|r| r.item() == Some(item2) && matches!(r.life(), RoundLife::Open))
        })
        .await;
    rig.stop().await.unwrap();

    let round2: RoundId = state
        .team(&team)
        .unwrap()
        .rounds()
        .iter()
        .find(|r| r.item() == Some(item2))
        .unwrap()
        .id()
        .clone();

    let daemon = Daemon::new(&state, None);
    let (mut owner, _sinks) = daemon.bare_owner().await;
    let lead = owner.lead_of(&team).unwrap();

    // Deliver round 2 by PR #1 (which is currently fixing)
    let mut look = Look::default();
    let accepted = owner
        .coordinator_delivered(&team, &lead.task, pr(1), &mut look)
        .await
        .unwrap();
    assert!(accepted);
    assert_eq!(
        owner.parts.state.team(&team).unwrap().round(&round2).unwrap().life(),
        &RoundLife::Delivered { pr: pr(1) }
    );

    // Checking obligations and waiting tasks:
    // Delivered round 2 waits for Obligation::Merge
    let team_ref = owner.parts.state.team(&team).unwrap();
    assert_eq!(
        owner.round_obligation(team_ref, team_ref.round(&round2).unwrap()),
        Some(Obligation::Merge)
    );

    // The waiting item 2.1 has not started any run, and its member is not woken
    let item2_progress = owner.parts.state.team(&team).unwrap().item(item2).unwrap().clone();
    assert!(
        matches!(item2_progress.phase(), ItemPhase::InProgress { .. }),
        "item 2 remains in progress waiting for merge"
    );
    let woke_item2_member = owner.outbox.iter().any(|job| {
        job.team == team
            && matches!(&job.job, crate::daemon::team::BusJob::Message { to, text } if to.as_str() == "claude-1" && text.contains("2.1"))
    });
    assert!(
        !woke_item2_member,
        "member must not be woken for item 2.1 during fix pass of PR 1"
    );
}

#[tokio::test]
async fn after_daemon_restart_waiting_delivered_item_still_waits_and_closes_when_pr_merged() {
    let mut rig = Rig::new(
        &[
            normal("claude-1", "work,sleep"),
            investigator("inv-r1", "done:VERDICT: APPROVE\\nSUMMARY: fine"),
            investigator("inv-r2", "done:VERDICT: APPROVE\\nSUMMARY: fine"),
        ],
        5,
        false,
    )
    .await;
    rig.start().await;
    let (team, _) = rig.run(&["claude-1"], PARALLEL, MergeMode::NoMerge, None).await;
    let item2: ItemId = "2.1".parse().unwrap();
    let state = rig
        .state_where("round for 2.1 is open and 1.1 is approved", |state| {
            let Some(team) = state.team(&team) else { return false };
            let Some(item1) = team.item("1.1".parse().unwrap()) else {
                return false;
            };
            matches!(item1.phase(), ItemPhase::Approved { .. })
                && team
                    .rounds()
                    .iter()
                    .any(|r| r.item() == Some(item2) && matches!(r.life(), RoundLife::Open))
        })
        .await;

    let round2: RoundId = state
        .team(&team)
        .unwrap()
        .rounds()
        .iter()
        .find(|r| r.item() == Some(item2))
        .unwrap()
        .id()
        .clone();

    // Deliver round 2 by PR #1 and write to state file before restart
    rig.stop().await.unwrap();

    let mut state = state;
    state.deliver_round(&round2, pr(1)).unwrap();
    std::fs::write(rig.home.state_file(), state.to_json().unwrap()).unwrap();

    // Restart daemon
    rig.start().await;

    // After daemon restart, verify waiting item still waits with status delivered
    let restarted_state = rig
        .state_where("round 2.1 is delivered", |state| {
            let Some(team) = state.team(&team) else { return false };
            matches!(
                team.round(&round2).map(agent_core::state::Round::life),
                Some(RoundLife::Delivered { .. })
            )
        })
        .await;
    assert_eq!(
        restarted_state.team(&team).unwrap().round(&round2).unwrap().life(),
        &RoundLife::Delivered { pr: pr(1) }
    );

    // Merge PR #1 (simulating human merging PR 1)
    rig.github.merged_by_a_human(1);

    // Verify round 2.1 and item 2.1 close as delivered!
    let final_state = rig
        .state_where("item 2.1 is done as delivered", |state| {
            let Some(team) = state.team(&team) else { return false };
            let Some(item) = team.item(item2) else { return false };
            matches!(
                item.phase(),
                ItemPhase::Done {
                    completion: Completion::Delivered { .. }
                }
            )
        })
        .await;
    rig.stop().await.unwrap();

    let team_after = final_state.team(&team).unwrap();
    assert!(matches!(
        team_after.round(&round2).unwrap().life(),
        RoundLife::Closed {
            how: RoundEnd::Delivered,
            ..
        }
    ));
}

#[tokio::test]
async fn team_of_two_items_whose_lead_delivers_both_in_first_pr_ends_with_both_done_and_one_pr() {
    let mut rig = Rig::new(
        &[
            investigator("inv-lead", "sleep,integrate,done:DELIVERED: #1"),
            normal("claude-1", "work,done"),
            investigator("inv-r1", "done:VERDICT: APPROVE\\nSUMMARY: fine"),
            investigator("inv-r2", "done:VERDICT: APPROVE\\nSUMMARY: fine"),
        ],
        5,
        true,
    )
    .await;
    std::fs::write(rig.repo.join(".git/info/exclude"), ".fake-*\n").unwrap();
    rig.start().await;

    let (team, _) = rig
        .run(
            &["inv-lead", "claude-1"],
            super::rig::CHAIN,
            MergeMode::Autopilot,
            Some("inv-lead"),
        )
        .await;

    // Wait for both items to reach Done
    let state = rig
        .state_where("both items are done", |state| {
            let Some(team) = state.team(&team) else { return false };
            team.items().len() == 2 && team.items().iter().all(|i| matches!(i.phase(), ItemPhase::Done { .. }))
        })
        .await;

    rig.notification_with("is finished").await;
    rig.stop().await.unwrap();

    let team_state = state.team(&team).unwrap();
    let pr_expected = pr(1);
    let item1: ItemId = "1.1".parse().unwrap();
    assert!(matches!(
        team_state.item(item1).unwrap().phase(),
        ItemPhase::Done {
            completion: Completion::Merged { pr, .. }
        } if pr.number == pr_expected
    ));

    let item2: ItemId = "1.2".parse().unwrap();
    assert!(matches!(
        team_state.item(item2).unwrap().phase(),
        ItemPhase::Done {
            completion: Completion::Delivered { pr: delivered_pr, .. }
        } if *delivered_pr == pr_expected
    ));

    // Verify only one PR was created on the remote
    let created_prs = rig
        .github
        .calls("POST", &format!("/repos/{}/pulls", super::fake_github::SLUG));
    assert_eq!(
        created_prs.len(),
        1,
        "only one PR should have been opened, got: {created_prs:?}"
    );
}
