//! The briefing of a new session opens with the last checkpoint of the round, and keeps the snapshot of that checkpoint
//! when the worktree has not changed since: a round limit that cuts a solo round with local commits, and the renewals of a
//! task that waits for the human, through a daemon in a temporary home.

use std::path::Path;

use agent_core::history::{CheckpointCause, HistoryEvent, Record};
use agent_core::ids::{TaskId, TeamId};
use agent_core::protocol::{Client, Command, ControlRequest, Reply, Target};
use agent_core::state::MergeMode;
use agent_core::status::TaskStatus;
use agent_runtime::history::Reader;

use super::rig::{ONE, Rig, git, investigator, normal};
use crate::client::Connection;

/// A reviewer that approves.
const APPROVE: &str = "verify:VERDICT: APPROVE\\nSUMMARY: fine";

/// A first-pass finding followed by acceptance of the committed fix.
const CHANGES: &str = "verify:VERDICT: REQUEST_CHANGES\\nFINDINGS\\n1. [MAJOR] TDD-2 src/lib.rs:7 — the case is untested → add coverage\\n   CHECK: read src/lib.rs:7,verify:VERDICT: APPROVE\\nSUMMARY: fixed";

/// The header every briefing of a new session starts with.
const BRIEFING: &str = "# A NEW SESSION ON A TASK THAT WAS ALREADY STARTED";

/// What the plan of a round that is cut says of what is left to do.
const NEXT_WITHOUT_PULL_REQUEST: &str = "commit what is finished, run the gate and hand the round over";

/// The line that ends the checkpoint in a briefing.
const AFTER_CHECKPOINT: &str = "\n\nThis is where the work stands.";

/// A checkpoint of the history, as the tests look at it.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Kept {
    /// The step it follows.
    cause: CheckpointCause,
    /// The reference of its snapshot.
    snapshot: Option<String>,
}

/// Whether the history has a checkpoint of `team` taken after `wanted`.
fn has_checkpoint(rig: &Rig, team: &TeamId, wanted: CheckpointCause) -> bool {
    Reader::open(&rig.home.history_file())
        .unwrap()
        .team_records(team)
        .unwrap()
        .into_iter()
        .any(|stored| match stored.record {
            Record::Event(event) => matches!(event.event, HistoryEvent::Checkpoint { cause, .. } if cause == wanted),
            _ => false,
        })
}

/// The checkpoint of the round of `team` that tells `task`'s next holder how the work stands, as the history gives it.
fn last_told(rig: &Rig, team: &TeamId, task: &TaskId) -> Kept {
    let round = rig.state().team(team).unwrap().rounds()[0].id().clone();
    let found = Reader::open(&rig.home.history_file())
        .unwrap()
        .last_told_checkpoint_of_round(&round, task)
        .unwrap()
        .expect("the round has a checkpoint");
    Kept {
        cause: found.cause,
        snapshot: found.snapshot,
    }
}

/// The snapshot references the repository of the worktree holds.
fn snapshot_refs(worktree: &Path) -> Vec<String> {
    git(
        worktree,
        &["for-each-ref", "--format=%(refname)", "refs/agents/snapshots/"],
    )
    .lines()
    .map(str::to_owned)
    .collect()
}

/// The part of `briefing` that is the checkpoint's part `heading`, up to the next heading or the end of the checkpoint.
fn part<'a>(briefing: &'a str, heading: &str) -> &'a str {
    let (_, rest) = briefing
        .split_once(heading)
        .unwrap_or_else(|| panic!("no {heading}: {briefing}"));
    let to = [rest.find("\n### "), rest.find(AFTER_CHECKPOINT)]
        .into_iter()
        .flatten()
        .min()
        .unwrap_or(rest.len());
    rest.split_at(to).0
}

/// Adds `table` to the configuration of `rig`, before the daemon starts.
fn configure(rig: &Rig, table: &str) {
    let mut config = std::fs::read_to_string(rig.home.config_file()).unwrap();
    config.push_str(table);
    std::fs::write(rig.home.config_file(), config).unwrap();
}

/// Writes the script `name` of the stand-in: a plan whose step 1.1 is done for R1 and whose step 1.2 is to do for K1, two
/// commits with `Step: 1.1`, and a wait for the file `finish`. Returns the directory of the files it writes and waits for.
fn two_commits_script(rig: &Rig, name: &str) -> std::path::PathBuf {
    let signals = rig.home.root().join("signals");
    std::fs::create_dir_all(&signals).unwrap();
    let scripts = rig.home.root().join("fake-scripts");
    std::fs::create_dir_all(&scripts).unwrap();
    let text = format!(
        "set -eu\n\
         sig='{}'\n\
         g() {{ git -c user.name=Fake -c user.email=fake@example.com -c commit.gpgsign=false \"$@\"; }}\n\
         mkdir -p .agent\n\
         printf 'Do it.\\n\\n1. Work\\n[x] 1.1 First\\n    For: R1\\n[ ] 1.2 Second\\n    For: K1\\n' > .agent/plan.md\n\
         g commit -q --allow-empty -m 'One' -m 'Step: 1.1'\n\
         g commit -q --allow-empty -m 'Two' -m 'Step: 1.1'\n\
         touch \"$sig/two\"\n\
         while [ ! -e \"$sig/finish\" ]; do sleep 0.05; done\n",
        signals.display()
    );
    std::fs::write(scripts.join(format!("{name}.sh")), text).unwrap();
    signals
}

// Rule: the round limit cuts a solo round whose agent made two local commits and pushed nothing: the agent that takes the
// task over is briefed with the checkpoint of the cut, which tells both commits as done, the steps that are not done and
// what to do next, and it works in the same worktree on the same branch at the same head.
#[tokio::test]
async fn the_new_holder_of_a_round_the_limit_cut_is_briefed_with_the_checkpoint_of_the_cut() {
    let mut rig = Rig::new(
        &[
            normal("claude-1", "script:two"),
            normal("claude-2", "hold"),
            investigator("inv-r1", APPROVE),
            investigator("inv-r2", APPROVE),
        ],
        5,
        false,
    )
    .await;
    configure(
        &rig,
        "[replacement]\nagents = [\"claude-2\"]\n[limits]\nround_minutes = 120\n",
    );
    let signals = two_commits_script(&rig, "two");
    rig.start().await;
    let (team, tasks) = rig.run(&["claude-1"], ONE, MergeMode::Autopilot, None).await;
    let worktree = rig.worktree(&tasks[0]);
    rig.wait_for("two commits", || signals.join("two").exists()).await;
    let head = git(&worktree, &["rev-parse", "HEAD"]);
    let branch = git(&worktree, &["rev-parse", "--abbrev-ref", "HEAD"]);
    let first = git(&worktree, &["rev-parse", "HEAD~1"]);

    rig.advance_history_days(1);
    let briefing = rig.prompt(&tasks[0], 2).await;
    let (head_after, branch_after) = (
        git(&worktree, &["rev-parse", "HEAD"]),
        git(&worktree, &["rev-parse", "--abbrev-ref", "HEAD"]),
    );
    std::fs::write(signals.join("finish"), "").unwrap();
    rig.stop().await.unwrap();

    assert!(briefing.starts_with(BRIEFING), "{briefing}");
    assert!(briefing.contains("## Checkpoint: round_overdue at "), "{briefing}");
    let done = part(&briefing, "### What is done\n");
    for commit in [&first, &head].map(|full| full.chars().take(7).collect::<String>()) {
        assert!(
            done.contains(&commit),
            "commit {commit} is not under What is done: {done}"
        );
    }
    assert!(done.contains("R1 met"), "{done}");
    let left = part(&briefing, "### What is left\n");
    assert!(left.contains("K1 not started — steps left: 1.2 (not done)"), "{left}");
    assert!(left.contains(&format!("next: {NEXT_WITHOUT_PULL_REQUEST}")), "{left}");
    assert_eq!(
        (head_after, branch_after),
        (head, branch),
        "the same branch at the same head"
    );
    assert_eq!(last_told(&rig, &team, &tasks[0]).cause, CheckpointCause::RoundOverdue);
}

/// Sends `command` as the command line does and returns the reply.
async fn ask(rig: &Rig, command: Command) -> Reply {
    Connection::connect(&rig.home.socket_file())
        .await
        .unwrap()
        .request(Client::Cli, command)
        .await
        .unwrap()
}

/// A solo task that has a finding open and waits for the human, with its team, its task and its worktree.
async fn waiting_with_a_finding() -> (Rig, TeamId, TaskId, std::path::PathBuf) {
    let mut rig = Rig::new(
        &[
            normal("claude-1", "work,fix-done"),
            investigator("inv-r1", CHANGES),
            investigator("inv-r2", APPROVE),
        ],
        5,
        true,
    )
    .await;
    rig.start().await;
    let (team, tasks) = rig.run(&["claude-1"], ONE, MergeMode::Autopilot, None).await;
    rig.notification_with("FIXED needs a commit").await;
    rig.status_is(&tasks[0], TaskStatus::Waiting).await;
    rig.wait_for("the checkpoint of the wait", || {
        has_checkpoint(&rig, &team, CheckpointCause::WaitsForHuman)
    })
    .await;
    let worktree = rig.worktree(&tasks[0]);
    (rig, team, tasks[0].clone(), worktree)
}

// Rule: a renewal of a task whose last checkpoint is of a review: the briefing has that checkpoint, tells the open finding
// under what is left, and takes the snapshot of the checkpoint instead of a new one, as the worktree is as the checkpoint
// kept it.
#[tokio::test]
async fn a_renewal_is_briefed_with_the_last_checkpoint_and_makes_no_new_snapshot() {
    let (mut rig, team, task, worktree) = waiting_with_a_finding().await;
    let runs_before = rig.runs(&task);
    let told = last_told(&rig, &team, &task);
    let reference = told.snapshot.clone().expect("the checkpoint kept a snapshot");
    let before = snapshot_refs(&worktree);

    let renewed = ask(
        &rig,
        Command::Renew(ControlRequest {
            target: Target::Task(task.clone()),
        }),
    )
    .await;
    let briefing = rig.prompt(&task, runs_before + 1).await;
    let after = snapshot_refs(&worktree);
    rig.stop().await.unwrap();

    assert!(matches!(renewed, Reply::Applied { .. }), "{renewed:?}");
    assert!(briefing.starts_with(BRIEFING), "{briefing}");
    assert!(
        briefing.contains(&format!("## Checkpoint: {} at ", told.cause)),
        "{briefing}"
    );
    let left = part(&briefing, "### What is left\n");
    assert!(
        left.contains("[MAJOR]") && left.contains("src/lib.rs:7"),
        "the open finding is not under What is left: {left}"
    );
    assert!(
        briefing.contains(&format!("kept as a snapshot, {reference}.")),
        "the snapshot of the checkpoint is not the briefing's: {briefing}"
    );
    assert_eq!(after, before, "no snapshot reference was made");
}

// Rule: a worktree changed after the last checkpoint is kept in a new snapshot, and the briefing still names the checkpoint.
#[tokio::test]
async fn a_worktree_changed_after_the_checkpoint_is_kept_in_a_new_snapshot_and_the_checkpoint_is_still_named() {
    let (mut rig, team, task, worktree) = waiting_with_a_finding().await;
    let runs_before = rig.runs(&task);
    let told = last_told(&rig, &team, &task);
    let checkpoint_reference = told.snapshot.clone().expect("the checkpoint kept a snapshot");
    let before = snapshot_refs(&worktree);
    std::fs::write(worktree.join("changed-after.txt"), "after\n").unwrap();

    let renewed = ask(
        &rig,
        Command::Renew(ControlRequest {
            target: Target::Task(task.clone()),
        }),
    )
    .await;
    let briefing = rig.prompt(&task, runs_before + 1).await;
    let after = snapshot_refs(&worktree);
    rig.stop().await.unwrap();

    assert!(matches!(renewed, Reply::Applied { .. }), "{renewed:?}");
    assert!(
        briefing.contains(&format!("## Checkpoint: {} at ", told.cause)),
        "{briefing}"
    );
    assert!(
        after.len() > before.len(),
        "a new snapshot reference: {before:?} -> {after:?}"
    );
    assert!(
        !briefing.contains(&format!("kept as a snapshot, {checkpoint_reference}.")),
        "the old snapshot lacks the changed file: {briefing}"
    );
    assert!(briefing.contains("- changed-after.txt"), "{briefing}");
}

// Rule: a new session after runs without a status, whose worktree is as the last checkpoint kept it, is briefed from that
// checkpoint's snapshot and makes no snapshot of its own: every reference is a checkpoint's, none is numbered.
// Test-First: not possible — the behaviour exists since the briefing took the checkpoint's snapshot; this test pins it.
#[tokio::test]
async fn a_no_status_new_session_whose_worktree_matches_the_last_checkpoint_makes_no_numbered_snapshot() {
    let mut rig = Rig::new(
        &[
            normal("claude-1", "nostatus-dirty,nostatus,nostatus,work"),
            investigator("inv-r1", APPROVE),
            investigator("inv-r2", APPROVE),
        ],
        5,
        false,
    )
    .await;
    rig.start().await;
    let (_, tasks) = rig.run(&["claude-1"], ONE, MergeMode::Autopilot, None).await;

    let briefing = rig.prompt(&tasks[0], 4).await;
    let references = snapshot_refs(&rig.repo);
    rig.stop().await.unwrap();

    assert!(briefing.starts_with(BRIEFING), "{briefing}");
    let named: Vec<&String> = references
        .iter()
        .filter(|reference| briefing.contains(reference.as_str()))
        .collect();
    assert_eq!(named.len(), 1, "one snapshot is named: {references:?}\n{briefing}");
    let leaves: Vec<&str> = references
        .iter()
        .filter_map(|reference| reference.rsplit('/').next())
        .collect();
    assert_eq!(leaves.len(), references.len(), "{references:?}");
    assert!(
        leaves.iter().all(|leaf| leaf.starts_with('c')),
        "no numbered snapshot is made: {references:?}"
    );
}
