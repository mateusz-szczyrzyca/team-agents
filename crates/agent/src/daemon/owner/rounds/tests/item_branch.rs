//! A run for an item starts on the item's branch: a fix pass is given its item's branch even when the worktree was left on
//! another one, and a worktree that cannot be switched without losing changes is left and the lead is told.

use std::collections::BTreeSet;
use std::path::Path;

use agent_core::ids::{BranchName, TaskId, TeamId};
use agent_core::log::{LogEntry, LogEvent, LogLevel};
use agent_core::state::{ItemPhase, MergeMode};
use agent_runtime::history::Reader;

use super::rig::{ONE, Rig, git, investigator, normal};

/// What the first run of the lead does: commits the item, then leaves the worktree on a branch of another name that has the
/// same tip, as a run that switched away from the item's branch does.
const LEAVE_THE_BRANCH: &str = "set -eu\n\
    commit() { git -c user.name=Fake -c user.email=fake@example.com -c commit.gpgsign=false commit -q \"$@\"; }\n\
    echo item > work-1.txt\n\
    git add work-1.txt\n\
    commit -m 'Work of the item'\n\
    git switch -q -c decoy\n";

/// A report that asks for changes, held until the test releases the reviewer (`.fake-release-1`).
const HELD_REQUEST_FOR_CHANGES: &str = "clean-held-done:VERDICT: REQUEST_CHANGES\\nFINDINGS\\n1. [MAJOR] TYP-3 work-1.txt:1 — units → document units\\n   CHECK: read work-1.txt";

/// The text the lead is given when its worktree cannot be switched, for the branch `checked_out` and the item's `branch`.
fn other_branch_text(checked_out: &str, branch: &str) -> String {
    format!(
        "Your worktree has `{checked_out}` out, with changes that are not committed, so the harness did not switch it to\n\
         `{branch}`. Commit those changes on `{checked_out}` first, then run `git switch {branch}`, and do the fix\n\
         pass there. Never commit the fixes on `{checked_out}`."
    )
}

/// A rig whose solo agent commits its item and leaves the worktree on `decoy` when `leave` (else works as `work` does) and
/// holds its fix pass; both reviewers ask for changes once the test releases them.
async fn held_fix_pass(leave: bool) -> Rig {
    let scenario = if leave {
        "script:leave,fix-held:1"
    } else {
        "work,fix-held:1"
    };
    let rig = Rig::new(
        &[
            normal("claude-1", scenario),
            investigator("inv-r1", HELD_REQUEST_FOR_CHANGES),
            investigator("inv-r2", HELD_REQUEST_FOR_CHANGES),
        ],
        1,
        false,
    )
    .await;
    let scripts = rig.home.root().join("fake-scripts");
    std::fs::create_dir_all(&scripts).unwrap();
    std::fs::write(scripts.join("leave.sh"), LEAVE_THE_BRANCH).unwrap();
    rig
}

/// Waits until both reviewers of `team` are running their first review, which is when the lead's first run has ended and its
/// pull request is open. Returns what releases them: the files that let their held reports go.
async fn reviewers_running(rig: &Rig, team: &TeamId) -> Vec<std::path::PathBuf> {
    let state = rig
        .state_where("both reviewers to be running", |state| {
            state.team(team).is_some_and(|team| {
                team.reviewers().len() == 2
                    && team
                        .reviewers()
                        .iter()
                        .all(|member| rig.worktree(member.task()).join(".fake-prompt-1").is_file())
            })
        })
        .await;
    state
        .team(team)
        .unwrap()
        .reviewers()
        .iter()
        .map(|member| rig.worktree(member.task()).join(".fake-release-1"))
        .collect()
}

/// Lets the held reviewers report.
fn release(files: &[std::path::PathBuf]) {
    for file in files {
        std::fs::write(file, "").unwrap();
    }
}

/// The branch of item 1.1 while its fix pass goes on.
async fn fixing_branch(rig: &Rig) -> BranchName {
    let state = rig
        .item_where("1.1", "the fix pass", |phase| matches!(phase, ItemPhase::Fixing { .. }))
        .await;
    let team = &state.teams()[0];
    let ItemPhase::Fixing { branch, .. } = team.item("1.1".parse().unwrap()).unwrap().phase() else {
        panic!("item 1.1 is not being fixed")
    };
    branch.clone()
}

/// The branch each run of the stand-in found out when it started, in the order of the runs: the third word of the lines of
/// its log.
fn branches_at_the_start(worktree: &Path) -> Vec<String> {
    std::fs::read_to_string(worktree.join(".fake-log"))
        .unwrap()
        .lines()
        .map(|line| line.split_whitespace().nth(2).unwrap().to_owned())
        .collect()
}

/// The rows of `event` in the daemon's log of `rig`, once the daemon has stopped.
fn rows(rig: &Rig, event: LogEvent) -> Vec<LogEntry> {
    Reader::open(&rig.home.history_file())
        .unwrap()
        .daemon_logs()
        .unwrap()
        .into_iter()
        .filter(|row| row.event() == event)
        .collect()
}

/// The text of the field `name` of `entry`.
fn field<'a>(entry: &'a LogEntry, name: &str) -> &'a str {
    entry
        .fields()
        .get(name)
        .and_then(serde_json::Value::as_str)
        .unwrap_or_else(|| panic!("no text field {name}: {entry:?}"))
}

// Rule: a fix pass of an item runs on the item's branch: the worktree that the run before left on another branch, with a
// clean tree, is switched, and the switch is logged once with the branch it left and the one it went to.
#[tokio::test]
async fn fix_pass_runs_on_the_item_branch() {
    let mut rig = held_fix_pass(true).await;
    rig.start().await;
    let (team, tasks) = rig.run(&["claude-1"], ONE, MergeMode::Autopilot, None).await;
    let lead = rig.worktree(&tasks[0]);
    release(&reviewers_running(&rig, &team).await);
    let item = fixing_branch(&rig).await;
    rig.wait_for("the fix pass run to start", || lead.join(".fake-started-2").exists())
        .await;

    let seen = branches_at_the_start(&lead);
    std::fs::write(lead.join(".fake-release-2"), "").unwrap();
    rig.stop().await.unwrap();

    assert_eq!(seen, [item.to_string(), item.to_string()]);
    let switched = rows(&rig, LogEvent::WorktreeSwitched);
    assert_eq!(switched.len(), 1, "{switched:#?}");
    assert_eq!(switched[0].level(), LogLevel::Info);
    assert_eq!(switched[0].task(), Some(&tasks[0]));
    assert_eq!(field(&switched[0], "from"), "decoy");
    assert_eq!(field(&switched[0], "to"), item.as_str());
    assert_eq!(rows(&rig, LogEvent::WorktreeSwitchRefused), []);
}

// Rule: a worktree with a changed tracked file is not switched, whatever it has out: the file is still changed, the branch is
// the one it had, the refusal is logged, and the lead's prompt tells it to commit and switch itself.
#[tokio::test]
async fn dirty_other_branch_is_not_switched_and_told() {
    let mut rig = held_fix_pass(true).await;
    rig.start().await;
    let (team, tasks) = rig.run(&["claude-1"], ONE, MergeMode::Autopilot, None).await;
    let lead = rig.worktree(&tasks[0]);
    let released = reviewers_running(&rig, &team).await;
    std::fs::write(lead.join("work-1.txt"), "changed after the run\n").unwrap();
    release(&released);
    let item = fixing_branch(&rig).await;
    rig.wait_for("the fix pass run to start", || lead.join(".fake-started-2").exists())
        .await;

    let on_branch = git(&lead, &["symbolic-ref", "--short", "HEAD"]);
    let kept = std::fs::read_to_string(lead.join("work-1.txt")).unwrap();
    let prompt = rig.kept_prompt(&tasks[0], 2);
    std::fs::write(lead.join(".fake-release-2"), "").unwrap();
    rig.stop().await.unwrap();

    assert_eq!(on_branch, "decoy");
    assert_eq!(kept, "changed after the run\n");
    let told = other_branch_text("decoy", item.as_str());
    let at = prompt
        .find(&told)
        .unwrap_or_else(|| panic!("the prompt does not say {told:?}: {prompt}"));
    let pass = prompt.find("THE REVIEWERS ASKED FOR CHANGES").unwrap();
    assert!(at < pass, "the text comes before the fix pass: {prompt}");
    let refused = rows(&rig, LogEvent::WorktreeSwitchRefused);
    assert_eq!(refused.len(), 1, "{refused:#?}");
    assert_eq!(refused[0].level(), LogLevel::Warn);
    assert_eq!(refused[0].task(), Some(&tasks[0]));
    assert_eq!(field(&refused[0], "from"), "decoy");
    assert_eq!(field(&refused[0], "to"), item.as_str());
    assert_eq!(rows(&rig, LogEvent::WorktreeSwitched), []);
}

// Rule: a worktree that has the item's branch out is left as it is, and nothing is said of it.
#[tokio::test]
async fn run_on_its_own_branch_does_not_switch() {
    let mut rig = held_fix_pass(false).await;
    rig.start().await;
    let (team, tasks) = rig.run(&["claude-1"], ONE, MergeMode::Autopilot, None).await;
    let lead = rig.worktree(&tasks[0]);
    release(&reviewers_running(&rig, &team).await);
    let item = fixing_branch(&rig).await;
    rig.wait_for("the fix pass run to start", || lead.join(".fake-started-2").exists())
        .await;

    let seen = branches_at_the_start(&lead);
    let prompt = rig.kept_prompt(&tasks[0], 2);
    std::fs::write(lead.join(".fake-release-2"), "").unwrap();
    rig.stop().await.unwrap();

    assert_eq!(seen, [item.to_string(), item.to_string()]);
    assert_eq!(rows(&rig, LogEvent::WorktreeSwitched), []);
    assert_eq!(rows(&rig, LogEvent::WorktreeSwitchRefused), []);
    assert!(!prompt.contains("so the harness did not switch it to"), "{prompt}");
}

/// Produces both events of a worktree switch by their real cause, and records them into `observed`: the daemon switches a
/// clean worktree, and leaves a changed one.
pub(in crate::daemon::owner) async fn record_worktree_switch_events(observed: &mut BTreeSet<LogEvent>) {
    for (dirty, event) in [
        (false, LogEvent::WorktreeSwitched),
        (true, LogEvent::WorktreeSwitchRefused),
    ] {
        let mut rig = held_fix_pass(true).await;
        rig.start().await;
        let (team, tasks) = rig.run(&["claude-1"], ONE, MergeMode::Autopilot, None).await;
        let lead = rig.worktree(&tasks[0]);
        let released = reviewers_running(&rig, &team).await;
        if dirty {
            std::fs::write(lead.join("work-1.txt"), "changed after the run\n").unwrap();
        }
        release(&released);
        rig.wait_for("the fix pass run to start", || lead.join(".fake-started-2").exists())
            .await;
        std::fs::write(lead.join(".fake-release-2"), "").unwrap();
        rig.stop().await.unwrap();

        let found = rows(&rig, event);
        assert_eq!(found.len(), 1, "{event}: {found:#?}");
        let (task, row): (&TaskId, &LogEntry) = (&tasks[0], &found[0]);
        assert_eq!(row.level(), event.level());
        assert_eq!(row.team(), Some(&team));
        assert_eq!(row.task(), Some(task));
        observed.insert(event);
    }
}
