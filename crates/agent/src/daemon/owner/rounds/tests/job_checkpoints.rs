//! The checkpoints of the jobs that are no item of a round — a worker's request, a review, an investigation — and the
//! take-over from the last one, through a daemon in a temporary home.

use agent_core::history::{CheckpointCause, CheckpointJob};
use agent_core::ids::RoundId;
use agent_core::ids::TaskId;
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

/// The names of the reviewers of the review pass in `state`, in the order asked.
fn reviewer_names(state: &State) -> Vec<String> {
    reviews(state)
        .iter()
        .map(|review| review.reviewer.as_str().to_owned())
        .collect()
}

/// The task of the review that `agent` gives in `state`, when it gives one.
fn review_task_of(state: &State, agent: &str) -> Option<TaskId> {
    reviews(state)
        .into_iter()
        .find(|review| review.reviewer.as_str() == agent)
        .map(|review| review.task)
}

/// Whether the reviewer `agent` of the only team of `state` is paused by its provider.
fn reviewer_paused(state: &State, agent: &str) -> bool {
    state.teams().first().is_some_and(|team| {
        team.reviewers()
            .iter()
            .any(|member| member.status() == TaskStatus::Paused && member.agent().name() == agent)
    })
}

#[tokio::test]
async fn a_reviewer_that_wrote_two_findings_and_was_replaced_hands_them_to_the_next_with_the_same_pass() {
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
    rig.run(&["claude-1"], ONE, MergeMode::Autopilot, None).await;
    let paused = rig
        .state_where("the reviewer inv-1 paused by its quota", |state| {
            reviewer_paused(state, "inv-1")
        })
        .await;
    assert!(review_task_of(&paused, "inv-1").is_some());
    rig.advance_history_days(1);
    let replaced = rig
        .state_where("the review of inv-1 given to inv-4", |state| {
            let names = reviewer_names(state);
            names.iter().any(|name| name == "inv-4") && !names.iter().any(|name| name == "inv-1")
        })
        .await;
    let replacement = review_task_of(&replaced, "inv-4").unwrap();
    rig.item_where("1.1", "merged", |phase| matches!(phase, ItemPhase::Done { .. }))
        .await;
    rig.stop().await.unwrap();

    let prompt = std::fs::read_to_string(rig.worktree(&replacement).join(".fake-prompt-1")).unwrap();
    assert!(prompt.contains("# THIS REVIEW WAS STARTED BEFORE YOU"), "{prompt}");
    assert!(prompt.contains("1. [MAJOR] ERR-3 a.rs:4 — a panic"), "{prompt}");
    assert!(prompt.contains("2. [MINOR] DOC-1 b.rs:9 — no docs"), "{prompt}");
    assert!(prompt.contains("finish the review of pass 1"), "{prompt}");
    assert!(
        prompt.contains("review-pr"),
        "the replacement has a findings file of its own: {prompt}"
    );
}

/// The `daemon_started` checkpoints of the jobs of `round`, oldest first.
fn daemon_started_jobs(rig: &Rig, round: &RoundId) -> Vec<LastCheckpoint> {
    job_checkpoints(rig, round)
        .into_iter()
        .filter(|checkpoint| checkpoint.cause == CheckpointCause::DaemonStarted)
        .collect()
}

#[tokio::test]
async fn a_daemon_that_starts_again_checkpoints_an_investigation_that_was_going_with_its_notes() {
    let mut rig = Rig::new(
        &[
            normal("claude-lead", "sleep"),
            normal("claude-1", "inv-add:Why does the split fail?,sleep"),
            investigator("inv-r1", "hold"),
            investigator("inv-r2", "hold"),
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
        .state_where("the investigator", |state| {
            state.team(&team).is_some_and(|team| {
                team.reviewers()
                    .iter()
                    .any(|member| member.role() == Some(Role::Investigator))
            })
        })
        .await;
    let investigator = state
        .team(&team)
        .unwrap()
        .reviewers()
        .iter()
        .find(|member| member.role() == Some(Role::Investigator))
        .map(|member| member.task().clone())
        .unwrap();
    let round = state.team(&team).unwrap().rounds()[0].id().clone();
    rig.wait_for("the checkpoint of the investigation's start", || {
        job_checkpoints(&rig, &round)
            .iter()
            .any(|checkpoint| checkpoint.cause == CheckpointCause::InvestigationStarted)
    })
    .await;
    std::fs::write(rig.worktree(&investigator).join("notes.md"), "found before the kill\n").unwrap();
    rig.stop().await.unwrap();

    rig.start().await;
    rig.wait_for("the checkpoint of the start of the daemon", || {
        !daemon_started_jobs(&rig, &round).is_empty()
    })
    .await;
    rig.stop().await.unwrap();

    let started: Vec<_> = daemon_started_jobs(&rig, &round)
        .into_iter()
        .filter(|checkpoint| checkpoint.task == investigator)
        .collect();
    assert_eq!(started.len(), 1, "{started:#?}");
    assert_eq!(started[0].job, CheckpointJob::Investigation);
    assert_eq!(started[0].task, investigator);
    let content = started[0].content.as_ref().unwrap();
    assert!(content.meant.contains("Why does the split fail?"), "{content:?}");
    assert!(content.done.contains("notes.md"), "{content:?}");
    assert_eq!(content.left, "next: answer the question");
}

#[tokio::test]
async fn a_daemon_that_starts_again_checkpoints_a_review_that_was_going_with_its_pass() {
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
            .any(|checkpoint| checkpoint.cause == CheckpointCause::ReviewStarted)
    })
    .await;
    std::fs::write(
        rig.worktree(&reviewer).join("scratch.md"),
        "looked at before the kill\n",
    )
    .unwrap();
    rig.stop().await.unwrap();

    rig.start().await;
    rig.wait_for("the checkpoint of the start of the daemon", || {
        !daemon_started_jobs(&rig, &round).is_empty()
    })
    .await;
    rig.stop().await.unwrap();

    let started: Vec<_> = daemon_started_jobs(&rig, &round)
        .into_iter()
        .filter(|checkpoint| checkpoint.task == reviewer)
        .collect();
    assert_eq!(started.len(), 1, "{started:#?}");
    assert_eq!(started[0].job, CheckpointJob::Review);
    assert_eq!(started[0].task, reviewer);
    assert_eq!(started[0].review_pass, Some(1));
    assert!(started[0].pull_request.is_some(), "{started:#?}");
    let content = started[0].content.as_ref().unwrap();
    assert_eq!(
        content.left,
        "next: finish the review of pass 1 and submit it with a verdict"
    );
}

#[tokio::test]
async fn the_look_checkpoints_a_commit_of_an_investigator_and_the_findings_a_reviewer_wrote() {
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
    let started = job_checkpoints(&rig, &round)
        .into_iter()
        .find(|checkpoint| checkpoint.task == reviewer && checkpoint.cause == CheckpointCause::ReviewStarted)
        .unwrap();
    let file = rig
        .home
        .review_findings_file(&reviewer, started.pull_request.unwrap(), started.review_pass.unwrap());
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::write(&file, "1. [MAJOR] ERR-3 a.rs:4 — a panic\n").unwrap();
    rig.wait_for("the checkpoint of the findings", || {
        job_checkpoints(&rig, &round)
            .iter()
            .any(|checkpoint| checkpoint.task == reviewer && checkpoint.cause == CheckpointCause::FindingsWritten)
    })
    .await;
    let worktree = rig.worktree(&reviewer);
    std::fs::write(worktree.join("notes.md"), "a note\n").unwrap();
    for args in [
        ["add", "notes.md"].as_slice(),
        &["commit", "-q", "-m", "Note the review"],
    ] {
        let status = std::process::Command::new("git")
            .args(["-c", "user.name=Reviewer", "-c", "user.email=reviewer@example.com"])
            .args(args)
            .current_dir(&worktree)
            .status()
            .unwrap();
        assert!(status.success(), "git {args:?}");
    }
    rig.wait_for("the checkpoint of the commit", || {
        job_checkpoints(&rig, &round)
            .iter()
            .any(|checkpoint| checkpoint.task == reviewer && checkpoint.cause == CheckpointCause::Commit)
    })
    .await;
    rig.stop().await.unwrap();

    let kept = job_checkpoints(&rig, &round);
    let written = kept
        .iter()
        .find(|checkpoint| checkpoint.task == reviewer && checkpoint.cause == CheckpointCause::FindingsWritten)
        .unwrap();
    assert_eq!(written.job, CheckpointJob::Review);
    assert!(
        written.content.as_ref().unwrap().done.contains("ERR-3 a.rs:4"),
        "{written:#?}"
    );
    let committed = kept
        .iter()
        .find(|checkpoint| checkpoint.task == reviewer && checkpoint.cause == CheckpointCause::Commit)
        .unwrap();
    assert!(committed.head.is_some(), "{committed:#?}");
    assert_ne!(committed.head, started.head);
}
