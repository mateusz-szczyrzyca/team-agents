//! An item is judged and published from the tip of its own branch, whatever branch the lead's worktree has out: the checks,
//! the gate and the push read the item's branch, and a branch without a gate at its tip is not pushed from another one.

use std::num::NonZeroU32;

use agent_core::ids::TaskId;
use agent_core::log::{LogEntry, LogEvent};
use agent_core::questions::QuestionsFile;
use agent_core::state::MergeMode;
use agent_core::status::TaskStatus;
use agent_runtime::history::Reader;
use agent_runtime::store::load_checks_record;

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
    let mut rig = rig_of("fix-script-then:bfix", "done").await;
    fix_script(&rig, "bfix", FIX_EDITING_A_TEST, BACK_TO_THE_OTHER_ITEM);
    let items = items_ready(&mut rig).await;

    rig.status_is(&items.task, TaskStatus::Questions).await;
    let file =
        QuestionsFile::parse(&std::fs::read_to_string(rig.home.questions_file(&items.task, NonZeroU32::MIN)).unwrap())
            .unwrap();
    let record = load_checks_record(&rig.home, &items.task).await.unwrap().unwrap();
    let on_branch = git(&items.lead, &["symbolic-ref", "--short", "HEAD"]);
    let fixed_tip = git(&items.lead, &["rev-parse", &items.fixed]);
    rig.stop().await.unwrap();

    assert_eq!(on_branch, items.other);
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
    let mut rig = rig_of("fix-script:bfix", "done").await;
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
        .filter(|row| field(row, "outcome") == "gate_started")
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
