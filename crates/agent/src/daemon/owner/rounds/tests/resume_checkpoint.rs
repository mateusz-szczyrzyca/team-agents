//! Every way a task goes on after a failure starts from the last checkpoint: a member that is stopped and renewed, an
//! agent that is busy elsewhere or fails in the middle of its work and is replaced, and a session that ends three times
//! without a status. The next run's prompt tells what is done and what is left per requirement, in the same worktree on
//! the same branch at the same head, and lists nothing done as left.

use std::path::{Path, PathBuf};

use agent_core::ids::TaskId;
use agent_core::protocol::{Client, Command, ControlRequest, Reply, Target};
use agent_core::state::MergeMode;
use agent_core::status::TaskStatus;

use super::rig::{ONE, Rig, git, investigator, normal};
use crate::client::Connection;

/// A reviewer that approves.
const APPROVE: &str = "verify:VERDICT: APPROVE\\nSUMMARY: fine";

/// The header every briefing of a new session starts with.
const BRIEFING: &str = "# A NEW SESSION ON A TASK THAT WAS ALREADY STARTED";

/// The line that ends the checkpoint in a briefing.
const AFTER_CHECKPOINT: &str = "\n\nThis is where the work stands.";

/// Where the work of the script stood when its run went bad: the head, the branch and the file it left.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Standing {
    /// The head of the worktree.
    head: String,
    /// The branch of the worktree.
    branch: String,
}

impl Standing {
    /// What `worktree` stands at now.
    fn of(worktree: &Path) -> Self {
        Self {
            head: git(worktree, &["rev-parse", "HEAD"]),
            branch: git(worktree, &["rev-parse", "--abbrev-ref", "HEAD"]),
        }
    }
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

/// Writes the script `name` of the stand-in: the first time it runs it writes a plan whose step 1.1 is done for R1 and
/// whose step 1.2 is to do for K1, makes one commit of step 1.1 and leaves a file; every time it touches `midway`, and,
/// when `waits`, holds until the file `finish` is there. Returns the directory of the files it writes and waits for.
fn midway_script(rig: &Rig, name: &str, waits: bool) -> PathBuf {
    let signals = rig.home.root().join(format!("signals-{name}"));
    std::fs::create_dir_all(&signals).unwrap();
    let scripts = rig.home.root().join("fake-scripts");
    std::fs::create_dir_all(&scripts).unwrap();
    let mut text = format!(
        "set -eu\n\
         sig='{}'\n\
         if [ ! -e \"$sig/first\" ]; then\n\
           mkdir -p .agent\n\
           printf 'Do it.\\n\\n1. Work\\n[x] 1.1 First\\n    For: R1\\n[ ] 1.2 Second\\n    For: K1\\n' > .agent/plan.md\n\
           git -c user.name=Fake -c user.email=fake@example.com -c commit.gpgsign=false commit -q --allow-empty -m 'One' -m 'Step: 1.1'\n\
           printf 'draft\\n' > draft.txt\n\
           touch \"$sig/first\"\n\
         fi\n\
         touch \"$sig/midway\"\n",
        signals.display()
    );
    if waits {
        text.push_str("while [ ! -e \"$sig/finish\" ]; do sleep 0.05; done\n");
    }
    std::fs::write(scripts.join(format!("{name}.sh")), text).unwrap();
    signals
}

/// Checks that `briefing` is the briefing of a new session that opens with a checkpoint of the work `standing` describes:
/// the commit and the requirement it meets under what is done, the step that is not done under what is left, nothing done
/// listed as left, and the worktree still on the same branch at the same head.
fn assert_resumed(briefing: &str, worktree: &Path, standing: &Standing) {
    assert!(briefing.starts_with(BRIEFING), "{briefing}");
    assert!(briefing.contains("## Checkpoint: "), "{briefing}");
    let short: String = standing.head.chars().take(7).collect();
    let done = part(briefing, "### What is done\n");
    assert!(
        done.contains(&short),
        "the commit {short} is not under What is done: {done}"
    );
    assert!(done.contains("R1 met"), "{done}");
    let left = part(briefing, "### What is left\n");
    assert!(left.contains("K1 not started — steps left: 1.2 (not done)"), "{left}");
    assert!(!left.contains("R1"), "something done is listed as left: {left}");
    assert_eq!(&Standing::of(worktree), standing, "the same branch at the same head");
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

/// Asks the daemon to stop or renew `task`, as `agent stop` and `agent renew` do, and checks that it was applied.
async fn control(rig: &Rig, task: &TaskId, make: fn(ControlRequest) -> Command) {
    let reply = ask(
        rig,
        make(ControlRequest {
            target: Target::Task(task.clone()),
        }),
    )
    .await;
    assert!(matches!(reply, Reply::Applied { .. }), "unexpected reply: {reply:?}");
}

// Rule: a member that is stopped in the middle of its step 1.2 and started again with `agent renew` goes on from the last
// checkpoint: its next prompt tells the commit it made as done and the step that is not done as left.
#[tokio::test]
async fn a_member_stopped_in_the_middle_of_a_step_and_renewed_goes_on_from_the_last_checkpoint() {
    let mut rig = Rig::new(
        &[
            investigator("inv-lead", "sleep"),
            normal("claude-1", "script:midway,hold"),
            investigator("inv-r1", APPROVE),
            investigator("inv-r2", APPROVE),
        ],
        5,
        true,
    )
    .await;
    let signals = midway_script(&rig, "midway", true);
    rig.start().await;
    let (_, tasks) = rig
        .run(&["inv-lead", "claude-1"], ONE, MergeMode::Autopilot, Some("inv-lead"))
        .await;
    rig.wait_for("the work of the first run", || signals.join("midway").exists())
        .await;
    let worktree = rig.worktree(&tasks[1]);
    let standing = Standing::of(&worktree);

    control(&rig, &tasks[1], Command::Stop).await;
    rig.status_is(&tasks[1], TaskStatus::Stopped).await;
    control(&rig, &tasks[1], Command::Renew).await;
    let briefing = rig.prompt(&tasks[1], 2).await;
    assert_resumed(&briefing, &worktree, &standing);
    assert!(
        briefing.contains("draft.txt"),
        "the uncommitted file is not told: {briefing}"
    );
    std::fs::write(signals.join("finish"), "").unwrap();
    rig.stop().await.unwrap();
}

// Rule: a member whose agent is busy in another team for longer than `busy_after` when the task is renewed is handed to
// the next agent of `[replacement] agents`, whose first prompt is the briefing from the last checkpoint.
#[tokio::test]
async fn a_member_handed_over_because_its_agent_is_busy_is_briefed_from_the_last_checkpoint() {
    let mut rig = Rig::new(
        &[
            investigator("inv-lead", "sleep"),
            normal("claude-1", "script:midway,hold"),
            normal("claude-2", "hold"),
            investigator("inv-r1", APPROVE),
            investigator("inv-r2", APPROVE),
        ],
        5,
        true,
    )
    .await;
    configure(&rig, "[replacement]\nagents = [\"claude-2\"]\nbusy_after = 300\n");
    let signals = midway_script(&rig, "midway", true);
    rig.start().await;
    let (_, tasks) = rig
        .run(&["inv-lead", "claude-1"], ONE, MergeMode::Autopilot, Some("inv-lead"))
        .await;
    rig.wait_for("the work of the first run", || signals.join("midway").exists())
        .await;
    let worktree = rig.worktree(&tasks[1]);
    let standing = Standing::of(&worktree);
    control(&rig, &tasks[1], Command::Stop).await;
    rig.status_is(&tasks[1], TaskStatus::Stopped).await;
    // The agent is busy in another team: its run there holds until `finish`.
    let (_, busy) = rig.run(&["claude-1"], ONE, MergeMode::NoMerge, None).await;
    rig.status_is(&busy[0], TaskStatus::Running).await;

    control(&rig, &tasks[1], Command::Renew).await;
    rig.advance_history_days(1);
    let state = rig
        .state_where("the member is held by claude-2", |state| {
            state
                .find_task(&tasks[1])
                .is_some_and(|found| found.member.agent().name() == "claude-2" && found.member.current_run().is_some())
        })
        .await;
    let runs = u32::try_from(state.find_task(&tasks[1]).unwrap().member.runs().len()).unwrap();
    let briefing = rig.prompt(&tasks[1], runs).await;
    assert_resumed(&briefing, &worktree, &standing);
    assert!(briefing.contains("`busy`"), "{briefing}");
    std::fs::write(signals.join("finish"), "").unwrap();
    rig.stop().await.unwrap();
}

// Rule: a member that reports `STATUS: FAILED` in the middle of its work is replaced, and the agent that takes the task
// over is briefed from the last checkpoint.
#[tokio::test]
async fn a_member_that_fails_in_the_middle_of_its_work_is_replaced_and_the_new_agent_is_briefed_from_the_checkpoint() {
    let mut rig = Rig::new(
        &[
            investigator("inv-lead", "sleep"),
            normal("claude-1", "script-failed-tool:midway"),
            normal("claude-2", "hold"),
            investigator("inv-r1", APPROVE),
            investigator("inv-r2", APPROVE),
        ],
        5,
        true,
    )
    .await;
    configure(&rig, "[replacement]\nagents = [\"claude-2\"]\n");
    let signals = midway_script(&rig, "midway", false);
    rig.start().await;
    let (_, tasks) = rig
        .run(&["inv-lead", "claude-1"], ONE, MergeMode::Autopilot, Some("inv-lead"))
        .await;
    let state = rig
        .state_where("the member is held by claude-2", |state| {
            state
                .find_task(&tasks[1])
                .is_some_and(|found| found.member.agent().name() == "claude-2" && found.member.current_run().is_some())
        })
        .await;
    let runs = u32::try_from(state.find_task(&tasks[1]).unwrap().member.runs().len()).unwrap();
    let worktree = rig.worktree(&tasks[1]);
    let briefing = rig.prompt(&tasks[1], runs).await;
    let standing = Standing::of(&worktree);
    assert!(signals.join("first").exists());
    assert_resumed(&briefing, &worktree, &standing);
    rig.stop().await.unwrap();
}

// Rule: three runs of a session that end with no status leave the task to a new session, whose briefing opens with the
// last checkpoint.
#[tokio::test]
async fn a_session_that_ends_three_times_without_a_status_goes_on_in_a_new_session_from_the_last_checkpoint() {
    let mut rig = Rig::new(
        &[
            normal(
                "claude-1",
                "script-nostatus:midway,script-nostatus:midway,script-nostatus:midway,hold",
            ),
            investigator("inv-r1", APPROVE),
            investigator("inv-r2", APPROVE),
        ],
        5,
        false,
    )
    .await;
    let signals = midway_script(&rig, "midway", false);
    rig.start().await;
    let (_, tasks) = rig.run(&["claude-1"], ONE, MergeMode::Autopilot, None).await;
    let briefing = rig.prompt(&tasks[0], 4).await;
    let worktree = rig.worktree(&tasks[0]);
    let standing = Standing::of(&worktree);
    assert!(signals.join("first").exists());
    assert_resumed(&briefing, &worktree, &standing);
    rig.stop().await.unwrap();
}

/// The texts the bus of `rig` keeps, every file of its directory read as text.
fn bus_texts(rig: &Rig) -> String {
    let mut texts = String::new();
    let mut dirs = vec![rig.home.root().join("bus")];
    while let Some(dir) = dirs.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries {
            let path = entry.unwrap().path();
            if path.is_dir() {
                dirs.push(path);
            } else if let Ok(text) = std::fs::read_to_string(&path) {
                texts.push_str(&text);
            }
        }
    }
    texts
}

// Rule: the lead that is given the work of a member whose sessions went bad is told the last checkpoint of the member's
// round, with what is done and what is left per requirement, as a new holder of the task would be.
#[tokio::test]
async fn the_lead_given_a_members_work_is_told_the_last_checkpoint() {
    let mut rig = Rig::new(
        &[
            investigator("inv-lead", "sleep"),
            normal("claude-1", "script-nostatus:midway"),
            investigator("inv-r1", APPROVE),
            investigator("inv-r2", APPROVE),
        ],
        5,
        true,
    )
    .await;
    configure(&rig, "[replacement]\nagents = [\"inv-lead\"]\n");
    midway_script(&rig, "midway", false);
    rig.start().await;
    let (_, tasks) = rig
        .run(&["inv-lead", "claude-1"], ONE, MergeMode::Autopilot, Some("inv-lead"))
        .await;
    rig.status_is(&tasks[1], TaskStatus::Stopped).await;
    rig.wait_for("the lead's text on the bus", || {
        bus_texts(&rig).contains("WORK OF A MEMBER THAT IS GONE")
    })
    .await;
    let bus = bus_texts(&rig);
    let head = Standing::of(&rig.worktree(&tasks[1])).head;
    rig.stop().await.unwrap();

    // The bus keeps the text of a message as a JSON string: its line breaks are escaped.
    let text = bus
        .split("WORK OF A MEMBER THAT IS GONE")
        .nth(1)
        .unwrap_or_default()
        .replace("\\n", "\n");
    let text = text.as_str();
    assert!(text.contains("## Checkpoint: "), "{text}");
    let short: String = head.chars().take(7).collect();
    let done = part(text, "### What is done\n");
    assert!(
        done.contains(&short),
        "the commit {short} is not under What is done: {done}"
    );
    assert!(done.contains("R1 met"), "{done}");
    let left = part(text, "### What is left\n");
    assert!(left.contains("K1 not started — steps left: 1.2 (not done)"), "{left}");
    assert!(!left.contains("R1"), "something done is listed as left: {left}");
}
