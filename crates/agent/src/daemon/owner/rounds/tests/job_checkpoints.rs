//! The checkpoints of the jobs that are no item of a round — a worker's request, a review, an investigation — and the
//! take-over from the last one, through a daemon in a temporary home.

use agent_core::history::{CheckpointCause, CheckpointJob};
use agent_core::ids::RoundId;
use agent_core::kinds::Role;
use agent_core::state::MergeMode;
use agent_runtime::history::{LastCheckpoint, Reader};

use super::rig::{ONE, Rig, investigator, normal};

/// The checkpoints of the jobs of `round`, oldest first.
fn job_checkpoints(rig: &Rig, round: &RoundId) -> Vec<LastCheckpoint> {
    Reader::open(&rig.home.history_file())
        .unwrap()
        .job_checkpoints_of_round(round)
        .unwrap()
}

#[tokio::test]
async fn an_investigation_tried_again_starts_from_the_checkpoint_of_the_first_investigator() {
    let mut rig = Rig::new(
        &[
            normal("claude-lead", "sleep"),
            normal("claude-1", "inv-add:Why does the split fail?,sleep"),
            investigator("inv-r1", "inv-retry-notes"),
            investigator("inv-r2", "inv-retry-notes"),
        ],
        1,
        true,
    )
    .await;
    rig.start().await;
    let (team, _) = rig
        .run(
            &["claude-lead", "claude-1"],
            ONE,
            MergeMode::NoMerge,
            Some("claude-lead"),
        )
        .await;
    let state = rig
        .state_where("the investigation was answered", |state| {
            state.team(&team).is_some_and(|team| {
                team.reviewers()
                    .iter()
                    .filter(|member| member.role() == Some(Role::Investigator))
                    .count()
                    == 2
            })
        })
        .await;
    let attempts: Vec<_> = state
        .team(&team)
        .unwrap()
        .reviewers()
        .iter()
        .filter(|member| member.role() == Some(Role::Investigator))
        .map(|member| member.task().clone())
        .collect();
    let round = state.team(&team).unwrap().rounds()[0].id().clone();
    rig.wait_for("the second investigator has its answer checkpoint", || {
        job_checkpoints(&rig, &round)
            .iter()
            .any(|checkpoint| checkpoint.cause == CheckpointCause::InvestigationAnswered)
    })
    .await;
    rig.stop().await.unwrap();

    let kept = job_checkpoints(&rig, &round);
    let first_ended = kept
        .iter()
        .find(|checkpoint| checkpoint.task == attempts[0] && checkpoint.cause == CheckpointCause::RunEnded)
        .expect("the end of the first investigator's run is a checkpoint");
    assert_eq!(first_ended.job, CheckpointJob::Investigation);
    let content = first_ended.content.as_ref().unwrap();
    assert!(
        content.meant.contains("Answer the question: Why does the split fail?"),
        "{content:?}"
    );
    assert!(content.done.contains("notes.md"), "{content:?}");
    assert_eq!(content.left, "next: answer the question");
    let started: Vec<_> = kept
        .iter()
        .filter(|checkpoint| checkpoint.cause == CheckpointCause::InvestigationStarted)
        .map(|checkpoint| checkpoint.task.clone())
        .collect();
    assert_eq!(started, attempts);
    let prompt = std::fs::read_to_string(rig.worktree(&attempts[1]).join(".fake-prompt-1")).unwrap();
    assert!(prompt.contains("# THIS INVESTIGATION WAS TRIED BEFORE YOU"), "{prompt}");
    assert!(prompt.contains("notes.md"), "{prompt}");
    let first_prompt = std::fs::read_to_string(rig.worktree(&attempts[0]).join(".fake-prompt-1")).unwrap();
    assert!(!first_prompt.contains("TRIED BEFORE YOU"), "{first_prompt}");
}
