//! A run for an item starts on the item's branch: a fix pass is given its item's branch even when the worktree was left on
//! another one, and a worktree that cannot be switched without losing changes is left and the lead is told.

use std::collections::BTreeSet;
use std::path::Path;

use agent_core::ids::{BranchName, TaskId, TeamId};
use agent_core::log::{LogEntry, LogEvent, LogLevel};
use agent_core::state::{ItemPhase, MergeMode};
use agent_runtime::history::Reader;

use super::rig::PARALLEL;
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

/// Like [`HELD_REQUEST_FOR_CHANGES`], on a file that every branch has: with two items, the checks of the harness read the
/// worktree's `HEAD`, which is the other item's tip, and a file of the item's own would not be there.
const HELD_REQUEST_FOR_README: &str = "clean-held-done:VERDICT: REQUEST_CHANGES\\nFINDINGS\\n1. [MAJOR] TYP-3 README.md:1 — units → document units\\n   CHECK: read README.md";

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

/// What the run on item 2.1 does: commits its file, then holds until the test releases it, so that the reviewers' verdict
/// on 1.1 comes while the run is going (a run that started before the verdict is not judged as a response to it), and then
/// ends having left `leftover` in the worktree.
fn second_item_script(leftover: &str) -> String {
    format!(
        "set -eu\n\
         commit() {{ git -c user.name=Fake -c user.email=fake@example.com -c commit.gpgsign=false commit -q \"$@\"; }}\n\
         echo two > work-2.txt\n\
         git add work-2.txt\n\
         commit -m 'Work of the second item'\n\
         : > .second-started\n\
         while [ ! -e .second-release ]; do sleep 0.05; done\n\
         {leftover}\n"
    )
}

/// A rig whose solo agent works two items that do not wait for each other: run 1 commits item 1.1, run 2 is the script
/// `second` on item 2.1 (the worktree has 2.1's branch out, at its own tip, while 1.1 is in review), and run 3 holds the
/// fix pass of 1.1. Both reviewers ask for changes on the pull request of 1.1 once the test releases them. `leftover` is what
/// the script leaves in the worktree.
async fn two_items_rig(leftover: &str) -> Rig {
    let rig = Rig::new(
        &[
            normal("claude-1", "work,script:second,fix-held:1"),
            investigator("inv-r1", HELD_REQUEST_FOR_README),
            investigator("inv-r2", HELD_REQUEST_FOR_README),
        ],
        1,
        false,
    )
    .await;
    let scripts = rig.home.root().join("fake-scripts");
    std::fs::create_dir_all(&scripts).unwrap();
    std::fs::write(scripts.join("second.sh"), second_item_script(leftover)).unwrap();
    rig
}

/// What a test of two items holds once the fix pass of 1.1 has started.
struct TwoItems {
    /// The task of the solo agent.
    task: TaskId,
    /// Its worktree.
    lead: std::path::PathBuf,
    /// The branch of item 2.1, which the worktree had out when the fix pass was due.
    other: String,
    /// The branch of item 1.1.
    item: BranchName,
}

/// Starts the task of [`two_items_rig`] and goes on until the fix pass of 1.1 has started: the run on 2.1 is held while the
/// reviewers report, then released.
async fn fix_pass_started(rig: &mut Rig) -> TwoItems {
    rig.start().await;
    let (team, tasks) = rig.run(&["claude-1"], PARALLEL, MergeMode::Autopilot, None).await;
    let task = tasks[0].clone();
    let lead = rig.worktree(&task);
    let released = reviewers_running(rig, &team).await;
    rig.wait_for("the run on the second item to be held", || {
        lead.join(".second-started").exists()
    })
    .await;
    release(&released);
    let item = fixing_branch(rig).await;
    let other = git(&lead, &["symbolic-ref", "--short", "HEAD"]);
    std::fs::write(lead.join(".second-release"), "").unwrap();
    rig.wait_for("the fix pass run to start", || lead.join(".fake-started-3").exists())
        .await;
    TwoItems {
        task,
        lead,
        other,
        item,
    }
}

// Rule: the case of the issue: item 1.1 is fixed while the worktree has the branch of item 2.1 out, at that branch's own tip.
// The fix pass starts on the branch of 1.1, and the switch is logged once with both branches.
#[tokio::test]
async fn fix_pass_of_one_item_leaves_the_branch_of_another_item() {
    let mut rig = two_items_rig("true").await;
    let two = fix_pass_started(&mut rig).await;

    let seen = branches_at_the_start(&two.lead);
    std::fs::write(two.lead.join(".fake-release-3"), "").unwrap();
    rig.stop().await.unwrap();

    assert_ne!(two.other, two.item.as_str());
    assert_eq!(seen.last().map(String::as_str), Some(two.item.as_str()), "{seen:?}");
    let switched = rows(&rig, LogEvent::WorktreeSwitched);
    assert_eq!(switched.len(), 1, "{switched:#?}");
    assert_eq!(switched[0].task(), Some(&two.task));
    assert_eq!(field(&switched[0], "from"), two.other);
    assert_eq!(field(&switched[0], "to"), two.item.as_str());
    assert_eq!(rows(&rig, LogEvent::WorktreeSwitchRefused), []);
}

// Rule: a changed tracked file stops the switch off the branch of another item: the worktree keeps its branch and the
// change, and the refusal is logged. (The text that tells the lead to commit and switch is checked with one item above: with
// two, the response judging of the harness, which reads `HEAD`, turns this run into a request to correct a response.)
#[tokio::test]
async fn dirty_branch_of_another_item_is_not_switched_and_told() {
    let mut rig = two_items_rig("echo changed >> README.md").await;
    let two = fix_pass_started(&mut rig).await;

    let on_branch = git(&two.lead, &["symbolic-ref", "--short", "HEAD"]);
    let kept = std::fs::read_to_string(two.lead.join("README.md")).unwrap();
    std::fs::write(two.lead.join(".fake-release-3"), "").unwrap();
    rig.stop().await.unwrap();

    assert_eq!(on_branch, two.other);
    assert_eq!(kept, "Hello\nchanged\n");
    let refused = rows(&rig, LogEvent::WorktreeSwitchRefused);
    assert_eq!(refused.len(), 1, "{refused:#?}");
    assert_eq!(field(&refused[0], "from"), two.other);
    assert_eq!(field(&refused[0], "to"), two.item.as_str());
    assert_eq!(refused[0].fields().get("error"), None);
    assert_eq!(rows(&rig, LogEvent::WorktreeSwitched), []);
}

// Rule: a switch that git itself refuses on a tree the harness took for clean (an untracked file that the item's branch
// tracks would be overwritten) is logged as a refusal with git's answer; the worktree stays where it was and the run starts.
#[tokio::test]
async fn switch_that_git_refuses_is_logged_with_its_error_and_the_run_starts() {
    let mut rig = two_items_rig("echo in the way > work-1.txt").await;
    let two = fix_pass_started(&mut rig).await;

    let on_branch = git(&two.lead, &["symbolic-ref", "--short", "HEAD"]);
    std::fs::write(two.lead.join(".fake-release-3"), "").unwrap();
    rig.stop().await.unwrap();

    assert_eq!(on_branch, two.other);
    let refused = rows(&rig, LogEvent::WorktreeSwitchRefused);
    assert_eq!(refused.len(), 1, "{refused:#?}");
    assert_eq!(refused[0].level(), LogLevel::Warn);
    assert_eq!(refused[0].task(), Some(&two.task));
    assert_eq!(field(&refused[0], "from"), two.other);
    assert_eq!(field(&refused[0], "to"), two.item.as_str());
    assert!(!field(&refused[0], "error").is_empty(), "{refused:#?}");
    assert_eq!(rows(&rig, LogEvent::WorktreeSwitched), []);
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
