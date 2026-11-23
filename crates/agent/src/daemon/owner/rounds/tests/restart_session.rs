//! A run that the daemon's stop cut goes on in a new session briefed with what it left, unless `[restart] session` says
//! it goes on in the old one; the round keeps its pull request and its review passes.

use agent_core::decide::ladder::Trouble;
use agent_core::state::{ItemPhase, MergeMode, ReviewOutcome};
use agent_core::status::TaskStatus;

use super::fake_github::SLUG;
use super::rig::{ONE, Rig, git, investigator, normal};

/// A reviewer that approves.
const APPROVING: &str = "verify:VERDICT: APPROVE\\nSUMMARY: fine";

/// A reviewer that asks for changes the first time and approves afterwards.
const CHANGES_ONCE: &str = "verify:VERDICT: REQUEST_CHANGES\\nFINDINGS\\n1. [MAJOR] TDD-2 src/lib.rs:7 — the case is untested → add coverage\\n   CHECK: read src/lib.rs:7,verify:VERDICT: APPROVE\\nSUMMARY: fixed";

/// The header every briefing of a new session starts with.
const BRIEFING: &str = "# A NEW SESSION ON A TASK THAT WAS ALREADY STARTED";

/// The bytes of the binary file the cut run leaves: a NUL, bytes that are no UTF-8 and a line break.
const BLOB: &[u8] = &[0x00, 0xff, 0xfe, 0x01, b'b', b'i', b'n', 0x0a, 0x80];

/// The script of the run that is cut: it leaves a file of every kind of change and holds until the daemon stops.
const CUT: &str = r#"
exclude=$(git rev-parse --git-path info/exclude)
grep -qxF '.fake-*' "$exclude" || printf '%s\n' '.fake-*' >> "$exclude"
printf 'one\n' > kept.txt
git add kept.txt
git -c user.name=Fixture -c user.email=fixture@example.com commit -q -m 'Add kept.txt'
printf 'two\n' >> kept.txt
printf 'three\n' > staged.txt
git add staged.txt
rm README.md
printf '\000\377\376\001bin\n\200' > blob.bin
for _ in $(seq 1 600); do
  [[ -f .fake-continue ]] && break
  sleep 0.05
done
"#;

/// The script of the run that goes on: it waits to be released, then commits what the worktree holds.
const FINISH: &str = r#"
exclude=$(git rev-parse --git-path info/exclude)
grep -qxF '.fake-*' "$exclude" || printf '%s\n' '.fake-*' >> "$exclude"
for _ in $(seq 1 600); do
  [[ -f .fake-continue ]] && break
  sleep 0.05
done
git add -A
git -c user.name=Fixture -c user.email=fixture@example.com commit -q -m 'Finish the work'
"#;

/// A rig whose agent runs `scenario`, with the two scripts of this file, and reviewers that approve.
async fn rig_with(scenario: &'static str, config: &str) -> Rig {
    let rig = Rig::new(
        &[
            normal("claude-1", scenario),
            investigator("inv-r1", APPROVING),
            investigator("inv-r2", APPROVING),
        ],
        5,
        false,
    )
    .await;
    let scripts = rig.home.root().join("fake-scripts");
    std::fs::create_dir_all(&scripts).unwrap();
    std::fs::write(scripts.join("cut.sh"), CUT).unwrap();
    std::fs::write(scripts.join("finish.sh"), FINISH).unwrap();
    let path = rig.home.config_file();
    let text = std::fs::read_to_string(&path).unwrap();
    std::fs::write(path, format!("{text}\n{config}")).unwrap();
    rig
}

/// What `git status` says in `dir`, with the files that stand for the fake CLI's own left out.
fn status_of(dir: &std::path::Path) -> String {
    git(dir, &["status", "--porcelain", "--untracked-files=all"])
}

// Rule: a run the daemon's stop cut goes on in a new session whose prompt is the briefing with the reason of the
// restart; every file the cut run left — modified, staged, deleted, untracked, binary — is in the worktree byte for byte
// and is listed in the briefing; the task finishes; no new session is counted against the task.
#[tokio::test]
async fn a_run_cut_by_the_daemon_stopping_goes_on_in_a_new_session_briefed_with_every_file_it_left() {
    let mut rig = rig_with("script:cut,script:finish", "").await;
    rig.start().await;
    let (_, tasks) = rig.run(&["claude-1"], ONE, MergeMode::Autopilot, None).await;
    let worktree = rig.worktree(&tasks[0]);
    rig.wait_for("the cut run to leave its files", || worktree.join("blob.bin").exists())
        .await;
    let before = status_of(&worktree);
    let kept = std::fs::read(worktree.join("kept.txt")).unwrap();
    let staged = std::fs::read(worktree.join("staged.txt")).unwrap();

    rig.stop().await.unwrap();
    rig.start().await;
    let briefing = rig.prompt(&tasks[0], 2).await;
    let args = std::fs::read_to_string(worktree.join(".fake-args-2")).unwrap();
    let after = status_of(&worktree);
    let blob = std::fs::read(worktree.join("blob.bin")).unwrap();
    let readme_gone = !worktree.join("README.md").exists();
    let kept_after = std::fs::read(worktree.join("kept.txt")).unwrap();
    let staged_after = std::fs::read(worktree.join("staged.txt")).unwrap();
    std::fs::write(worktree.join(".fake-continue"), b"").unwrap();
    let state = rig
        .item_where("1.1", "merged after the new session", |phase| {
            matches!(phase, ItemPhase::Done { .. })
        })
        .await;
    rig.stop().await.unwrap();

    assert!(briefing.starts_with(BRIEFING), "{briefing}");
    assert!(briefing.contains("`daemon_down`"), "{briefing}");
    for file in ["- kept.txt", "- staged.txt", "- README.md", "- blob.bin"] {
        assert!(briefing.contains(file), "{file} is not listed: {briefing}");
    }
    assert!(briefing.contains("Do the first thing in the repository."), "{briefing}");
    assert!(args.contains("--session-id"), "a new session: {args}");
    assert!(!args.contains("--resume"), "not the old session: {args}");
    assert_eq!(after, before, "the worktree is what the cut run left");
    assert_eq!(blob, BLOB);
    assert!(readme_gone);
    assert_eq!(kept_after, kept);
    assert_eq!(staged_after, staged);
    let member = state.find_task(&tasks[0]).unwrap().member;
    assert_eq!(member.status(), TaskStatus::Done);
    let renewals = member.renewals().expect("the cut began a new session");
    assert_eq!(renewals.reason(), Trouble::DaemonDown);
    assert_eq!(renewals.used(), 0, "a cut the daemon caused is no retry of the task");
    assert_ne!(member.runs()[0].session(), member.runs()[1].session());
}

// Rule: a round that already has its pull request and a review pass keeps both when a run is cut: the fix pass goes on
// in a new session, and the same pull request is reviewed again and merged.
#[tokio::test]
async fn a_fix_pass_cut_by_the_daemon_stopping_keeps_its_pull_request_and_its_pass_number() {
    let mut rig = Rig::new(
        &[
            normal("claude-1", "work,fix-script:cut,fix-work"),
            investigator("inv-r1", CHANGES_ONCE),
            investigator("inv-r2", APPROVING),
        ],
        5,
        false,
    )
    .await;
    let scripts = rig.home.root().join("fake-scripts");
    std::fs::create_dir_all(&scripts).unwrap();
    std::fs::write(scripts.join("cut.sh"), CUT).unwrap();
    rig.start().await;
    let (team, tasks) = rig.run(&["claude-1"], ONE, MergeMode::Autopilot, None).await;
    let worktree = rig.worktree(&tasks[0]);
    rig.wait_for("the cut fix pass to leave its files", || {
        worktree.join("blob.bin").exists()
    })
    .await;
    let before = rig.state();
    let progress = before.team(&team).unwrap().item("1.1".parse().unwrap()).unwrap();
    assert_eq!(progress.fix_passes().len(), 1);

    rig.stop().await.unwrap();
    rig.start().await;
    let briefing = rig.prompt(&tasks[0], 3).await;
    let state = rig
        .item_where("1.1", "done", |phase| matches!(phase, ItemPhase::Done { .. }))
        .await;
    rig.stop().await.unwrap();

    assert!(briefing.starts_with(BRIEFING), "{briefing}");
    assert!(briefing.contains("pull request #1, fix pass 1 of 5"), "{briefing}");
    let progress = state.team(&team).unwrap().item("1.1".parse().unwrap()).unwrap();
    assert_eq!(progress.fix_passes().len(), 1, "no pass was added by the cut");
    assert!(
        progress.fix_passes()[0]
            .requested_by
            .iter()
            .all(|review| matches!(review.outcome, ReviewOutcome::Submitted { .. })),
        "{:?}",
        progress.fix_passes()
    );
    assert_eq!(rig.github.calls("POST", &format!("/repos/{SLUG}/pulls")).len(), 1);
    assert_eq!(rig.github.reviews(1).len(), 4);
    assert_eq!(
        rig.github.calls("PUT", &format!("/repos/{SLUG}/pulls/1/merge")).len(),
        1
    );
}

// Rule: with `[restart] session = "same"` a cut run is resumed in the session it was cut in, with the note that the
// harness restarted, as before the briefing existed; the task is not started anew.
#[tokio::test]
async fn with_the_same_session_a_cut_run_resumes_its_old_session_as_before() {
    let mut rig = rig_with("script:cut,script:finish", "[restart]\nsession = \"same\"\n").await;
    rig.start().await;
    let (_, tasks) = rig.run(&["claude-1"], ONE, MergeMode::Autopilot, None).await;
    let worktree = rig.worktree(&tasks[0]);
    rig.wait_for("the cut run to leave its files", || worktree.join("blob.bin").exists())
        .await;

    rig.stop().await.unwrap();
    rig.start().await;
    let prompt = rig.prompt(&tasks[0], 2).await;
    let args = std::fs::read_to_string(worktree.join(".fake-args-2")).unwrap();
    std::fs::write(worktree.join(".fake-continue"), b"").unwrap();
    let state = rig
        .item_where("1.1", "merged in the old session", |phase| {
            matches!(phase, ItemPhase::Done { .. })
        })
        .await;
    rig.stop().await.unwrap();

    assert!(prompt.contains("THE HARNESS RESTARTED"), "{prompt}");
    assert!(!prompt.contains(BRIEFING), "{prompt}");
    assert!(args.contains("--resume"), "the old session: {args}");
    let member = state.find_task(&tasks[0]).unwrap().member;
    assert!(member.renewals().is_none(), "no new session was begun");
    assert_eq!(member.runs()[0].session(), member.runs()[1].session());
}

// Rule: a reviewer keeps its findings file only: a review cut by the stop is started again, in no new session and with
// no briefing, as before.
#[tokio::test]
async fn a_review_cut_by_the_daemon_stopping_starts_again_with_no_briefing() {
    let mut rig = Rig::new(
        &[
            normal("claude-1", "work"),
            investigator("inv-r1", "hold,verify:VERDICT: APPROVE\\nSUMMARY: fine"),
            investigator("inv-r2", APPROVING),
        ],
        5,
        false,
    )
    .await;
    rig.start().await;
    let (team, _) = rig.run(&["claude-1"], ONE, MergeMode::Autopilot, None).await;
    let held = rig
        .state_where("a review that holds", |state| {
            state.team(&team).is_some_and(|found| {
                found
                    .reviewers()
                    .iter()
                    .any(|reviewer| reviewer.status() == TaskStatus::Running)
            })
        })
        .await;
    let reviewer = held
        .team(&team)
        .unwrap()
        .reviewers()
        .iter()
        .find(|reviewer| reviewer.status() == TaskStatus::Running)
        .unwrap()
        .task()
        .clone();
    let worktree = rig.worktree(&reviewer);
    rig.wait_for("the review to hold", || worktree.join(".fake-prompt-1").exists())
        .await;

    rig.stop().await.unwrap();
    rig.start().await;
    let state = rig
        .item_where("1.1", "merged after the review began again", |phase| {
            matches!(phase, ItemPhase::Done { .. })
        })
        .await;
    rig.stop().await.unwrap();

    let prompt = rig.kept_prompt(&reviewer, 2);
    assert!(!prompt.contains(BRIEFING), "{prompt}");
    let found = state.team(&team).unwrap();
    let member = found
        .reviewers()
        .iter()
        .find(|member| member.task() == &reviewer)
        .unwrap();
    assert!(member.renewals().is_none(), "a reviewer is not started anew");
}
