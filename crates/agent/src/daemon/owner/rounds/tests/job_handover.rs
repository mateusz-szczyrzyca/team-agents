//! What happens to a review and an investigation when another holder takes the job over, or the daemon starts again: the
//! take-over is a `holder_changed` checkpoint of the new holder, and a run is briefed from its own task's last checkpoint
//! only, through a daemon in a temporary home.

use agent_core::history::{CheckpointCause, CheckpointJob};
use agent_core::ids::{RoundId, TaskId};
use agent_core::kinds::Role;
use agent_core::state::{ItemPhase, MergeMode, Review, State};
use agent_core::status::TaskStatus;
use agent_runtime::history::{LastCheckpoint, Reader};
use rand::SeedableRng;

use super::rig::{ONE, Rig, investigator, normal};

/// The checkpoints of the jobs of `round`, oldest first.
fn job_checkpoints(rig: &Rig, round: &RoundId) -> Vec<LastCheckpoint> {
    Reader::open(&rig.home.history_file())
        .unwrap()
        .job_checkpoints_of_round(round)
        .unwrap()
}

/// The reviews of the item in review of the only team of `state`; none when no item is in review.
fn reviews(state: &State) -> Vec<Review> {
    state
        .teams()
        .first()
        .and_then(|team| {
            team.items().iter().find_map(|item| match item.phase() {
                ItemPhase::InReview { reviews, .. } => Some(reviews.clone()),
                _ => None,
            })
        })
        .unwrap_or_default()
}

/// The task of the review that `agent` gives in `state`, when it gives one.
fn review_task_of(state: &State, agent: &str) -> Option<TaskId> {
    reviews(state)
        .into_iter()
        .find(|review| review.reviewer.as_str() == agent)
        .map(|review| review.task)
}

/// The first prompt of `task`, once the stand-in has kept it.
fn first_prompt(rig: &Rig, task: &TaskId) -> Option<String> {
    std::fs::read_to_string(rig.worktree(task).join(".fake-prompt-1")).ok()
}

/// Whether a `holder_changed` checkpoint of `task` is in `kept` before its `started` checkpoint, if it has one.
fn holder_changed_of<'a>(kept: &'a [LastCheckpoint], task: &TaskId) -> Option<&'a LastCheckpoint> {
    kept.iter()
        .find(|checkpoint| checkpoint.task == *task && checkpoint.cause == CheckpointCause::HolderChanged)
}

#[tokio::test]
async fn two_reviewers_of_one_pass_that_run_side_by_side_are_not_briefed_as_replacements() {
    let mut rig = Rig::new(
        &[
            normal("claude-1", "work"),
            investigator("inv-1", "hold"),
            investigator("inv-2", "hold"),
        ],
        1,
        false,
    )
    .await;
    rig.start().await;
    rig.run(&["claude-1"], ONE, MergeMode::Autopilot, None).await;
    let state = rig
        .state_where("both reviews are asked", |state| {
            review_task_of(state, "inv-1").is_some() && review_task_of(state, "inv-2").is_some()
        })
        .await;
    let tasks = [
        review_task_of(&state, "inv-1").unwrap(),
        review_task_of(&state, "inv-2").unwrap(),
    ];
    rig.wait_for("both reviewers have their prompt", || {
        tasks.iter().all(|task| first_prompt(&rig, task).is_some())
    })
    .await;
    rig.stop().await.unwrap();

    for task in &tasks {
        let prompt = first_prompt(&rig, task).unwrap();
        assert!(!prompt.contains("STARTED BEFORE YOU"), "{prompt}");
    }
}

#[tokio::test]
async fn a_replaced_reviewer_is_handed_over_by_a_holder_changed_checkpoint_of_the_replacement() {
    let mut rig = Rig::new(
        &[
            normal("claude-1", "work"),
            investigator("inv-1", "findings-quota"),
            investigator("inv-2", "opencode-verdict:APPROVE"),
            investigator("inv-3", "done:VERDICT: APPROVE\\nSUMMARY: fine"),
            investigator("inv-4", "opencode-verdict:APPROVE"),
        ],
        1,
        false,
    )
    .await;
    with_opencode_investigators(&rig, &["inv-2", "inv-4"]);
    slow_patience_and_long_quota_pause(&rig);
    rig.start_with_rng(rand::rngs::StdRng::seed_from_u64(0)).await;
    let (team, _) = rig.run(&["claude-1"], ONE, MergeMode::Autopilot, None).await;
    rig.state_where("the reviewer inv-1 paused by its quota", |state| {
        reviewer_paused(state, "inv-1")
    })
    .await;
    rig.advance_history_days(1);
    let replaced = rig
        .state_where("the review of inv-1 given to inv-4", |state| {
            review_task_of(state, "inv-4").is_some() && review_task_of(state, "inv-1").is_none()
        })
        .await;
    let replacement = review_task_of(&replaced, "inv-4").unwrap();
    let round = replaced.team(&team).unwrap().rounds()[0].id().clone();
    rig.item_where("1.1", "merged", |phase| matches!(phase, ItemPhase::Done { .. }))
        .await;
    rig.stop().await.unwrap();

    let kept = job_checkpoints(&rig, &round);
    let handed = holder_changed_of(&kept, &replacement).expect("the replacement has a holder_changed checkpoint");
    assert_eq!(handed.job, CheckpointJob::Review);
    assert_eq!(handed.review_pass, Some(1));
    let content = handed.content.as_ref().unwrap();
    assert!(content.done.contains("ERR-3 a.rs:4"), "{content:?}");
    assert!(content.done.contains("DOC-1 b.rs:9"), "{content:?}");
    let own: Vec<_> = kept
        .iter()
        .filter(|checkpoint| checkpoint.task == replacement)
        .map(|checkpoint| checkpoint.cause)
        .collect();
    assert_eq!(own.first(), Some(&CheckpointCause::HolderChanged), "{own:?}");
}

#[tokio::test]
async fn an_investigation_tried_again_is_handed_over_by_a_holder_changed_checkpoint_of_the_second_investigator() {
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
        .state_where("the investigation was tried twice", |state| {
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
        job_checkpoints(&rig, &round).iter().any(|checkpoint| {
            checkpoint.task == attempts[1] && checkpoint.cause == CheckpointCause::InvestigationAnswered
        })
    })
    .await;
    rig.stop().await.unwrap();

    let kept = job_checkpoints(&rig, &round);
    let handed =
        holder_changed_of(&kept, &attempts[1]).expect("the second investigator has a holder_changed checkpoint");
    assert_eq!(handed.job, CheckpointJob::Investigation);
    assert!(
        handed.content.as_ref().unwrap().done.contains("notes.md"),
        "{handed:#?}"
    );
    let own: Vec<_> = kept
        .iter()
        .filter(|checkpoint| checkpoint.task == attempts[1])
        .map(|checkpoint| checkpoint.cause)
        .collect();
    assert_eq!(own.first(), Some(&CheckpointCause::HolderChanged), "{own:?}");
    assert!(holder_changed_of(&kept, &attempts[0]).is_none(), "{kept:#?}");
}

#[tokio::test]
async fn a_review_that_was_going_when_the_daemon_stopped_is_briefed_from_its_daemon_started_checkpoint() {
    let mut rig = Rig::new(
        &[
            normal("claude-1", "work"),
            investigator("inv-1", "hold"),
            investigator("inv-2", "hold"),
        ],
        1,
        false,
    )
    .await;
    rig.start().await;
    let (team, _) = rig.run(&["claude-1"], ONE, MergeMode::Autopilot, None).await;
    let state = rig
        .state_where("the review is asked", |state| review_task_of(state, "inv-1").is_some())
        .await;
    let reviewer = review_task_of(&state, "inv-1").unwrap();
    let round = state.team(&team).unwrap().rounds()[0].id().clone();
    rig.wait_for("the checkpoint of the review's start", || {
        job_checkpoints(&rig, &round)
            .iter()
            .any(|checkpoint| checkpoint.task == reviewer && checkpoint.cause == CheckpointCause::ReviewStarted)
    })
    .await;
    std::fs::write(
        rig.worktree(&reviewer).join("scratch.md"),
        "looked at before the stop\n",
    )
    .unwrap();
    rig.stop().await.unwrap();

    rig.start().await;
    rig.wait_for("the review's second prompt", || {
        std::fs::read_to_string(rig.worktree(&reviewer).join(".fake-prompt-2")).is_ok()
    })
    .await;
    rig.stop().await.unwrap();

    let prompt = std::fs::read_to_string(rig.worktree(&reviewer).join(".fake-prompt-2")).unwrap();
    assert!(prompt.contains("# THIS REVIEW WAS STARTED BEFORE YOU"), "{prompt}");
    assert!(prompt.contains("## Checkpoint: daemon_started at"), "{prompt}");
    assert!(prompt.contains("scratch.md"), "{prompt}");
}

/// Makes the investigators `names` of `rig` `OpenCode` ones: their runs go to the same stand-in CLI, and their provider is
/// `opencode`, which no Claude investigator shares.
fn with_opencode_investigators(rig: &Rig, names: &[&str]) {
    let path = rig.home.config_file();
    let mut config = std::fs::read_to_string(&path).unwrap();
    for name in names {
        let claude = format!("[investigators.{name}]\ncli = \"claude\"");
        assert!(config.contains(&claude), "{config}");
        config = config.replace(&claude, &format!("[investigators.{name}]\ncli = \"opencode\""));
    }
    let stand_in = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/team-cli.sh");
    let config = config.replace(
        "[binaries]\n",
        &format!("[binaries]\nopencode = \"{}\"\n", stand_in.display()),
    );
    std::fs::write(path, config).unwrap();
}

/// Makes the patience of a reviewer and a quota pause far longer than the test: only the refusal rule changes the reviewer.
fn slow_patience_and_long_quota_pause(rig: &Rig) {
    let path = rig.home.config_file();
    let config = std::fs::read_to_string(&path).unwrap();
    let config = config.replace("[review]\n", "[review]\nreplace_after_minutes = 100000\n");
    std::fs::write(path, format!("{config}[limits]\nquota_pause_minutes = 600\n")).unwrap();
}

/// Whether the reviewer `agent` of the only team of `state` is paused by its provider.
fn reviewer_paused(state: &State, agent: &str) -> bool {
    state.teams().first().is_some_and(|team| {
        team.reviewers()
            .iter()
            .any(|member| member.status() == TaskStatus::Paused && member.agent().name() == agent)
    })
}
