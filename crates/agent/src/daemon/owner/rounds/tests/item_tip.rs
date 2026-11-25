//! An item is judged and published from the tip of its own branch, whatever branch the lead's worktree has out: the checks,
//! the gate and the push read the item's branch, and a branch without a gate at its tip is not pushed from another one.

use std::collections::BTreeSet;
use std::num::NonZeroU32;

use agent_core::ids::TaskId;
use agent_core::log::{LogEntry, LogEvent};
use agent_core::questions::QuestionsFile;
use agent_core::state::MergeMode;
use agent_core::status::TaskStatus;
use agent_runtime::history::Reader;
use agent_runtime::store::load_checks_record;

use super::fake_github::{MergeAnswer, SLUG};
use super::rig::{PARALLEL, Rig, git, investigator, normal};

/// A report that asks for changes on a file that every branch has, held until the test releases the reviewer
/// (`.fake-release-1`).
const HELD_REQUEST_FOR_README: &str = "clean-held-done:VERDICT: REQUEST_CHANGES\\nFINDINGS\\n1. [MAJOR] TYP-3 README.md:1 — units → document units\\n   CHECK: read README.md";

/// What every script starts with: git commits as a stand-in without signing.
const PRELUDE: &str = "set -eu\ncommit() { git -c user.name=Fake -c user.email=fake@example.com -c commit.gpgsign=false commit -q \"$@\"; }\n";

/// Item 2.1 (the other item, `A`): commits code with its test, says it started, and holds until the test releases it, so that
/// the reviewers' verdict on item 1.1 (`B`) comes while the run goes. The worktree ends on the branch of 2.1.
const OTHER_ITEM: &str = "echo 'fn a() {}' > src/a.txt\necho case > tests/new.txt\ngit add src/a.txt tests/new.txt\n\
    commit -m 'Code and test of the other item'\n: > .second-started\n\
    while [ ! -e .second-release ]; do sleep 0.05; done\n";

/// The fix of item 1.1 that edits an existing test: the checks hold it for the human.
const FIX_EDITING_A_TEST: &str =
    "sed -i 2d tests/existing.txt\ngit add tests/existing.txt\ncommit -m 'Change the existing test'\n";

/// The fix of item 1.1 that the checks let through.
const FIX_WITHOUT_ISSUE: &str = "echo fix > fix.txt\ngit add fix.txt\ncommit -m 'Fix the item'\n";

/// What a run does after its fix: leaves the worktree on the branch it had out before the harness switched it, which is the
/// branch of the other item.
const BACK_TO_THE_OTHER_ITEM: &str = "git switch -q -\n";

/// Both items of a rig, once the run on the other item is held and the verdict on item 1.1 is in.
struct Items {
    /// The solo agent's task.
    task: TaskId,
    /// Its worktree.
    lead: std::path::PathBuf,
    /// The branch of item 1.1, the item that is fixed.
    fixed: String,
    /// The branch of item 2.1, which the worktree has out when the fix pass ends.
    other: String,
    /// The tip of the branch of 1.1 when its pull request was opened.
    fixed_tip_before: String,
    /// The tip of the branch of 2.1 on origin; empty while nothing of it was pushed.
    other_remote_before: String,
    /// The tip of the branch of 2.1.
    other_tip: String,
}

/// A rig with a solo agent that works 1.1 (run 1), 2.1 (run 2, `OTHER_ITEM`) and then the fix pass of 1.1 by `fix_run`; run 4
/// is `fourth`. Both reviewers ask for changes once. The repository has the checks table and a test, a file of code and a
/// protected file.
async fn rig_of(fix_run: &'static str, fourth: &'static str) -> Rig {
    let scenario: &'static str = Box::leak(format!("work,script:second,{fix_run},{fourth}").into_boxed_str());
    let rig = Rig::new(
        &[
            normal("claude-1", scenario),
            investigator("inv-r1", HELD_REQUEST_FOR_README),
            investigator("inv-r2", HELD_REQUEST_FOR_README),
        ],
        5,
        false,
    )
    .await;
    let text = std::fs::read_to_string(rig.home.config_file()).unwrap();
    let text = text.replace("fix_passes = 5", "fix_passes = 5\ncheck_passes = 0");
    std::fs::write(
        rig.home.config_file(),
        format!("{text}[repos.example.checks]\ntest_paths = [\"tests/\"]\ncode_paths = [\"src/\"]\n"),
    )
    .unwrap();
    for (name, text) in [("tests/existing.txt", "a\nb\nc\n"), ("src/code.txt", "code\n")] {
        let file = rig.repo.join(name);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(file, text).unwrap();
    }
    git(&rig.repo, &["add", "-A"]);
    git(&rig.repo, &["commit", "-q", "-m", "Seed the tests and the code"]);
    git(&rig.repo, &["push", "-q", "origin", "main"]);
    let scripts = rig.home.root().join("fake-scripts");
    std::fs::create_dir_all(&scripts).unwrap();
    std::fs::write(scripts.join("second.sh"), format!("{PRELUDE}{OTHER_ITEM}")).unwrap();
    rig
}

/// Writes the script `name` of the stand-in CLI: the fix `body` after the prelude, and what follows it, `then`.
fn fix_script(rig: &Rig, name: &str, body: &str, then: &str) {
    let scripts = rig.home.root().join("fake-scripts");
    std::fs::write(scripts.join(format!("{name}.sh")), format!("{PRELUDE}{body}")).unwrap();
    std::fs::write(scripts.join(format!("{name}-then.sh")), format!("{PRELUDE}{then}")).unwrap();
}

/// Starts the task and goes on until the run on the other item is held, the reviewers' verdict on 1.1 is in and the run on
/// 2.1 has been released: the fix pass of 1.1 is next.
async fn items_ready(rig: &mut Rig) -> Items {
    rig.start().await;
    let (team, tasks) = rig.run(&["claude-1"], PARALLEL, MergeMode::Autopilot, None).await;
    let task = tasks[0].clone();
    let lead = rig.worktree(&task);
    let state = rig
        .state_where("both reviewers to be running", |state| {
            state.team(&team).is_some_and(|team| {
                team.reviewers().len() == 2
                    && team
                        .reviewers()
                        .iter()
                        .all(|member| rig.worktree(member.task()).join(".fake-prompt-1").is_file())
            })
        })
        .await;
    let released: Vec<_> = state
        .team(&team)
        .unwrap()
        .reviewers()
        .iter()
        .map(|member| rig.worktree(member.task()).join(".fake-release-1"))
        .collect();
    rig.wait_for("the run on the other item to be held", || {
        lead.join(".second-started").exists()
    })
    .await;
    let fixed = format!("agents/claude-1/{task}-1.1");
    let other = format!("agents/claude-1/{task}-2.1");
    let fixed_tip_before = rig.remote_tip(&fixed);
    let other_tip = git(&lead, &["rev-parse", &other]);
    let other_remote_before = remote_tip_of(rig, &other);
    for file in &released {
        std::fs::write(file, "").unwrap();
    }
    std::fs::write(lead.join(".second-release"), "").unwrap();
    Items {
        task,
        lead,
        fixed,
        other,
        fixed_tip_before,
        other_remote_before,
        other_tip,
    }
}

/// The tip of `branch` on origin, empty when origin has no such branch.
fn remote_tip_of(rig: &Rig, branch: &str) -> String {
    git(
        &rig.origin,
        &[
            "for-each-ref",
            "--format=%(objectname)",
            &format!("refs/heads/{branch}"),
        ],
    )
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

// Rule: the checks of an item read the tip of its own branch: the fix of 1.1 edits an existing test while the worktree has the
// branch of 2.1 out, and the question to the human is about the test of 1.1 and about the tip of 1.1.
#[tokio::test]
async fn checks_judge_the_item_branch_tip() {
    let mut rig = rig_of("fix-script-then:bfix", "questions").await;
    fix_script(&rig, "bfix", FIX_EDITING_A_TEST, BACK_TO_THE_OTHER_ITEM);
    let items = items_ready(&mut rig).await;

    rig.status_is(&items.task, TaskStatus::Questions).await;
    let file =
        QuestionsFile::parse(&std::fs::read_to_string(rig.home.questions_file(&items.task, NonZeroU32::MIN)).unwrap())
            .unwrap();
    let record = load_checks_record(&rig.home, &items.task).await.unwrap().unwrap();
    let fixed_tip = git(&items.lead, &["rev-parse", &items.fixed]);
    rig.stop().await.unwrap();

    let title = file.entries()[0].title();
    assert!(title.contains("`tests/existing.txt`"), "{title}");
    assert!(!title.contains("tests/new.txt"), "{title}");
    assert_eq!(record.basis.map(|basis| basis.head.to_string()), Some(fixed_tip));
}

// Rule: a gate that passed at the tip of the item's branch is enough: the fix is pushed to its own branch while the worktree
// has the branch of the other item out, and the branch of the other item on origin does not move.
#[tokio::test]
async fn gated_item_tip_publishes_while_off_branch() {
    let mut rig = rig_of("fix-script-then:bfix", "done").await;
    fix_script(&rig, "bfix", FIX_WITHOUT_ISSUE, BACK_TO_THE_OTHER_ITEM);
    let items = items_ready(&mut rig).await;

    rig.wait_for("the fix of 1.1 to be pushed", || {
        rig.remote_tip(&items.fixed) != items.fixed_tip_before
    })
    .await;
    let pushed = git(
        &rig.origin,
        &["log", "-1", "--format=%s", &format!("refs/heads/{}", items.fixed)],
    );
    let other_remote = remote_tip_of(&rig, &items.other);
    rig.stop().await.unwrap();

    assert_eq!(pushed, "Fix the item");
    assert_eq!(other_remote, items.other_remote_before);
}

/// The scenario of the two tests that follow: the fix of 1.1 is committed on its branch, the worktree ends on the branch of
/// 2.1, and the gate of the run is recorded there, at the tip of 2.1: nothing passed at the tip of 1.1.
async fn off_branch_fix_pass() -> (Rig, Items) {
    let mut rig = rig_of("fix-script-answers:bfix", "done").await;
    let scripts = rig.home.root().join("fake-scripts");
    std::fs::write(
        scripts.join("bfix.sh"),
        format!("{PRELUDE}{FIX_WITHOUT_ISSUE}{BACK_TO_THE_OTHER_ITEM}"),
    )
    .unwrap();
    let items = items_ready(&mut rig).await;
    rig.prompt(&items.task, 4).await;
    (rig, items)
}

// Rule: a lead that is done with another branch out and no gate at the tip of its item is sent back with the text that names
// both branches; the harness starts no gate of its own, and the event is logged.
#[tokio::test]
async fn off_branch_lead_is_sent_back() {
    let (mut rig, items) = off_branch_fix_pass().await;

    let prompt = rig.prompt(&items.task, 4).await;
    rig.stop().await.unwrap();

    let (fixed, other) = (&items.fixed, &items.other);
    let text = format!(
        "The harness publishes `{fixed}`, but your worktree has `{other}` out, and no gate has passed at the tip\n\
         of `{fixed}`. Commit what you have on `{other}`, run `git switch {fixed}`, then `agent gate`, and\n\
         end with `STATUS: DONE`."
    );
    assert!(prompt.contains(&text), "{prompt}");
    let started: Vec<_> = rows(&rig, LogEvent::ChecksDecided)
        .into_iter()
        .filter(|row| row.fields().get("outcome").and_then(serde_json::Value::as_str) == Some("gate_started"))
        .collect();
    assert_eq!(started, []);
    let off = rows(&rig, LogEvent::PublishOffBranch);
    assert_eq!(off.len(), 1, "{off:#?}");
    assert_eq!(off[0].task(), Some(&items.task));
    assert_eq!(field(&off[0], "branch"), fixed.as_str());
    assert_eq!(field(&off[0], "checked_out"), other.as_str());
}

// Rule: the tip of another item, gated, is never pushed to the branch of this one: the branch of 1.1 on origin keeps its head.
#[tokio::test]
async fn push_never_carries_another_items_tip() {
    let (mut rig, items) = off_branch_fix_pass().await;

    rig.prompt(&items.task, 4).await;
    let remote = rig.remote_tip(&items.fixed);
    rig.stop().await.unwrap();

    assert_eq!(remote, items.fixed_tip_before);
    assert_ne!(remote, items.other_tip);
    // The tip of the other item was never even tried: a push that origin would refuse is a failed call.
    assert_eq!(rows(&rig, LogEvent::GithubCallFailed), []);
    assert_eq!(rows(&rig, LogEvent::PublishOffBranch).len(), 1);
}

/// The reports of both reviewers: they approve once the test releases them (`.fake-release-1`).
const HELD_APPROVAL: &str = "clean-held-done:VERDICT: APPROVE\\nSUMMARY: fine";

/// The run of the lead in a merge conflict pass of 1.1 while the worktree has 2.1 out: it waits until the base has moved, goes
/// to the branch of 1.1, merges the base there, and (after the gate is recorded at that tip) leaves the worktree on 2.1.
const MERGE_BASE_ON_THE_ITEM: &str = "while [ ! -e .fake-base-ready ]; do sleep 0.05; done\n\
    item=$(git for-each-ref --format='%(refname:short)' 'refs/heads/agents/*/task-*-1.1')\n\
    git switch -q \"$item\"\ngit fetch -q origin main\n\
    git -c user.name=Fake -c user.email=fake@example.com -c commit.gpgsign=false merge -q --no-ff --no-edit origin/main\n";

// Rule: an approved item whose merge was refused for a conflict is judged by the tip of its own branch: the lead merged the base
// on the branch of 1.1 and left the worktree on the branch of 2.1, which does not contain the moved base, and the merged tip is
// pushed and the pull request merged as when 1.1 is checked out.
#[tokio::test]
async fn approved_push_judges_the_item_tip() {
    let scenario = "work,script:second,fix-script-then:bmerge";
    let mut rig = Rig::new(
        &[
            normal("claude-1", scenario),
            investigator("inv-r1", HELD_APPROVAL),
            investigator("inv-r2", HELD_APPROVAL),
        ],
        5,
        false,
    )
    .await;
    let scripts = rig.home.root().join("fake-scripts");
    std::fs::create_dir_all(&scripts).unwrap();
    let second = "echo two > work-2.txt\ngit add work-2.txt\ncommit -m 'Work of the second item'\n: > .second-started\n\
         while [ ! -e .second-release ]; do sleep 0.05; done\n";
    std::fs::write(scripts.join("second.sh"), format!("{PRELUDE}{second}")).unwrap();
    fix_script(&rig, "bmerge", MERGE_BASE_ON_THE_ITEM, BACK_TO_THE_OTHER_ITEM);
    // "Pull Request is not mergeable" is a conflict with the base.
    rig.github.answer_merges(MergeAnswer::Refuse(405));
    rig.start().await;
    let (team, tasks) = rig.run(&["claude-1"], PARALLEL, MergeMode::Autopilot, None).await;
    let task = tasks[0].clone();
    let lead = rig.worktree(&task);
    let state = rig
        .state_where("both reviewers to be running", |state| {
            state.team(&team).is_some_and(|team| {
                team.reviewers().len() == 2
                    && team
                        .reviewers()
                        .iter()
                        .all(|member| rig.worktree(member.task()).join(".fake-prompt-1").is_file())
            })
        })
        .await;
    rig.wait_for("the run on the other item to be held", || {
        lead.join(".second-started").exists()
    })
    .await;
    for member in state.team(&team).unwrap().reviewers() {
        std::fs::write(rig.worktree(member.task()).join(".fake-release-1"), "").unwrap();
    }
    rig.state_where("the lead to be sent to merge the base", |state| {
        state.team(&team).is_some_and(|team| {
            team.item("1.1".parse().unwrap())
                .is_some_and(|item| item.merge_conflict().is_some())
        })
    })
    .await;

    // GitHub merges from now on; the base moves, the run on 2.1 ends, and the lead merges the base on the branch of 1.1.
    rig.github.answer_merges(MergeAnswer::Allow);
    std::fs::write(rig.repo.join("base-file.txt"), "moved\n").unwrap();
    git(&rig.repo, &["add", "base-file.txt"]);
    git(&rig.repo, &["commit", "-q", "-m", "Move the base"]);
    git(
        &rig.repo,
        &["push", "-q", rig.origin.to_str().unwrap(), "HEAD:refs/heads/main"],
    );
    std::fs::write(lead.join(".fake-base-ready"), "").unwrap();
    std::fs::write(lead.join(".second-release"), "").unwrap();
    rig.item_where("1.1", "done", |phase| {
        matches!(phase, agent_core::state::ItemPhase::Done { .. })
    })
    .await;
    let fixed = format!("agents/claude-1/{task}-1.1");
    let item_tip = git(&lead, &["rev-parse", &fixed]);
    let pushed = remote_tip_of(&rig, &fixed);
    let other_remote = remote_tip_of(&rig, &format!("agents/claude-1/{task}-2.1"));
    let on_branch = git(&lead, &["symbolic-ref", "--short", "HEAD"]);
    rig.stop().await.unwrap();

    assert_eq!(on_branch, format!("agents/claude-1/{task}-2.1"));
    assert_eq!(pushed, item_tip);
    assert_ne!(other_remote, item_tip);
    assert_eq!(
        rig.github.calls("PUT", &format!("/repos/{SLUG}/pulls/1/merge")).len(),
        2,
        "the refused merge and the merge tried again"
    );
}

/// Produces the event of a branch that is not published off-branch by its real cause, and records it into `observed`: a lead
/// that is done with another item's branch out and no gate at the tip of its own.
pub(in crate::daemon::owner) async fn record_publish_off_branch(observed: &mut BTreeSet<LogEvent>) {
    let (mut rig, items) = off_branch_fix_pass().await;
    rig.prompt(&items.task, 4).await;
    rig.stop().await.unwrap();

    let found = rows(&rig, LogEvent::PublishOffBranch);
    assert_eq!(found.len(), 1, "{found:#?}");
    assert_eq!(found[0].level(), LogEvent::PublishOffBranch.level());
    assert_eq!(found[0].task(), Some(&items.task));
    observed.insert(LogEvent::PublishOffBranch);
}

/// Like [`off_branch_fix_pass`], but the worktree ends detached at the tip of 2.1 instead of on its branch.
async fn detached_off_branch_fix_pass() -> (Rig, Items) {
    let mut rig = rig_of("fix-script-answers:bfix", "done").await;
    let scripts = rig.home.root().join("fake-scripts");
    std::fs::write(
        scripts.join("bfix.sh"),
        format!("{PRELUDE}{FIX_WITHOUT_ISSUE}{BACK_TO_THE_OTHER_ITEM}git checkout -q --detach HEAD\n"),
    )
    .unwrap();
    let items = items_ready(&mut rig).await;
    rig.prompt(&items.task, 4).await;
    (rig, items)
}

// Rule: a worktree detached at the tip of another item is that item's work, not this one's: the lead is sent back with the
// text that names the other branch, and nothing of 2.1 is pushed to the branch of 1.1.
#[tokio::test]
async fn detached_at_another_items_tip_is_sent_back() {
    let (mut rig, items) = detached_off_branch_fix_pass().await;

    let prompt = rig.prompt(&items.task, 4).await;
    let remote = rig.remote_tip(&items.fixed);
    rig.stop().await.unwrap();

    let fixed = &items.fixed;
    assert!(
        prompt.contains(&format!("The harness publishes `{fixed}`, but your worktree has")),
        "{prompt}"
    );
    assert_eq!(remote, items.fixed_tip_before);
    assert_eq!(rows(&rig, LogEvent::GithubCallFailed), []);
    let off = rows(&rig, LogEvent::PublishOffBranch);
    assert_eq!(off.len(), 1, "{off:#?}");
    assert_eq!(field(&off[0], "branch"), fixed.as_str());
}
