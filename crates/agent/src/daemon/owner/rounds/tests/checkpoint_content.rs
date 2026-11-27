//! What the checkpoints of a round say of its work, told against the requirements of its item, through a daemon in a
//! temporary home: a stand-in CLI that writes a plan with `For:` lines and commits with a `Step:` trailer.

use agent_core::history::{CheckpointCause, CheckpointContent, HistoryEvent, Record};
use agent_core::ids::TeamId;
use agent_core::log::LogEvent;
use agent_core::state::{ItemPhase, MergeMode};
use agent_core::task_list::TaskList;
use agent_runtime::history::Reader;

use crate::commands::checkpoints::{CheckpointsRequest, Mode, Target, checkpoints_in};

use super::rig::{ONE, Rig, git, investigator, normal};

/// A reviewer that approves.
const APPROVING: &str = "verify:VERDICT: APPROVE\\nSUMMARY: fine";

/// The checkpoints of `team` that carry a content, in the order they were written, with their cause.
fn contents(rig: &Rig, team: &TeamId) -> Vec<(CheckpointCause, CheckpointContent)> {
    let reader = Reader::open(&rig.home.history_file()).unwrap();
    reader
        .team_records(team)
        .unwrap()
        .into_iter()
        .filter_map(|stored| match stored.record {
            Record::Event(event) => match event.event {
                HistoryEvent::Checkpoint {
                    cause,
                    content: Some(content),
                    ..
                } => Some((cause, content)),
                _ => None,
            },
            _ => None,
        })
        .collect()
}

/// Writes the script `name` of the stand-in: it writes a plan whose step 1.1 is done for R1 and whose step 1.2 is to do
/// for K1, commits once with `Step: 1.1`, and waits for the file `finish`. Returns the directory of the files it writes
/// and waits for.
fn plan_script(rig: &Rig, name: &str) -> std::path::PathBuf {
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
         touch \"$sig/one\"\n\
         while [ ! -e \"$sig/finish\" ]; do sleep 0.05; done\n",
        signals.display()
    );
    std::fs::write(scripts.join(format!("{name}.sh")), text).unwrap();
    signals
}

#[tokio::test]
async fn a_checkpoint_after_a_commit_tells_what_is_meant_what_is_done_and_what_is_left_against_the_requirements() {
    let mut rig = Rig::new(
        &[
            normal("claude-1", "script:plan"),
            investigator("inv-r1", APPROVING),
            investigator("inv-r2", APPROVING),
        ],
        5,
        false,
    )
    .await;
    let signals = plan_script(&rig, "plan");
    rig.start().await;
    let (team, tasks) = rig.run(&["claude-1"], ONE, MergeMode::Autopilot, None).await;
    let worktree = rig.worktree(&tasks[0]);

    rig.wait_for("the commit", || signals.join("one").exists()).await;
    let head = git(&worktree, &["rev-parse", "HEAD"]);
    let short = head.chars().take(7).collect::<String>();
    rig.wait_for("the checkpoint of the commit with its content", || {
        contents(&rig, &team)
            .iter()
            .any(|(cause, content)| *cause == CheckpointCause::Commit && content.done.contains(&short))
    })
    .await;
    std::fs::write(signals.join("finish"), "").unwrap();
    rig.stop().await.unwrap();

    let told = contents(&rig, &team);
    let (_, content) = told
        .iter()
        .find(|(cause, content)| *cause == CheckpointCause::Commit && content.done.contains(&short))
        .unwrap();
    let description = TaskList::parse(ONE).unwrap().display_short_desc();
    assert_eq!(
        content.meant,
        format!("Task: {description}\nItem 1.1: The first item\nR1. the first thing works\nK1. run the first thing")
    );
    assert_eq!(
        content.done,
        format!("Totals: 1 commits, 1 steps\nR1 met — steps 1.1 (declared); commits {short}\nK1 not started")
    );
    assert_eq!(
        content.left,
        "Totals: 1 steps, 0 findings\nK1 not started — steps left: 1.2 (not done)\n\
         next: commit what is finished, run the gate and hand the round over"
    );
}

#[tokio::test]
async fn the_checkpoint_of_the_opening_of_a_round_tells_every_requirement_not_started() {
    let mut rig = Rig::new(
        &[
            normal("claude-1", "script:plan"),
            investigator("inv-r1", APPROVING),
            investigator("inv-r2", APPROVING),
        ],
        5,
        false,
    )
    .await;
    let signals = plan_script(&rig, "plan");
    rig.start().await;
    let (team, _) = rig.run(&["claude-1"], ONE, MergeMode::Autopilot, None).await;
    rig.wait_for("the commit", || signals.join("one").exists()).await;
    std::fs::write(signals.join("finish"), "").unwrap();
    rig.stop().await.unwrap();

    let told = contents(&rig, &team);
    let (cause, content) = told.first().unwrap();

    assert_eq!(*cause, CheckpointCause::RoundOpened);
    assert_eq!(
        content.done,
        "Totals: 0 commits, 0 steps\nR1 not started\nK1 not started"
    );
    assert_eq!(
        content.left,
        "Totals: 0 steps, 0 findings\nR1 not started — no step names it\nK1 not started — no step names it\n\
         next: commit what is finished, run the gate and hand the round over"
    );
}

/// The checkpoints of `team` with their cause and content, in the order they were written, whether they have a content
/// or not.
fn taken(rig: &Rig, team: &TeamId) -> Vec<(CheckpointCause, Option<CheckpointContent>)> {
    let reader = Reader::open(&rig.home.history_file()).unwrap();
    reader
        .team_records(team)
        .unwrap()
        .into_iter()
        .filter_map(|stored| match stored.record {
            Record::Event(event) => match event.event {
                HistoryEvent::Checkpoint { cause, content, .. } => Some((cause, content)),
                _ => None,
            },
            _ => None,
        })
        .collect()
}

#[tokio::test]
async fn a_plan_that_does_not_read_leaves_a_checkpoint_without_content_and_a_checkpoint_failed_entry() {
    let mut rig = Rig::new(
        &[
            normal("claude-1", "script:broken"),
            investigator("inv-r1", APPROVING),
            investigator("inv-r2", APPROVING),
        ],
        5,
        false,
    )
    .await;
    let signals = plan_script(&rig, "broken");
    let script = rig.home.root().join("fake-scripts").join("broken.sh");
    let text = std::fs::read_to_string(&script).unwrap();
    std::fs::write(&script, text.replace("1. Work", "0. Work")).unwrap();
    rig.start().await;
    let (team, _) = rig.run(&["claude-1"], ONE, MergeMode::Autopilot, None).await;
    rig.wait_for("the commit", || signals.join("one").exists()).await;
    rig.wait_for("the checkpoint of the commit", || {
        taken(&rig, &team)
            .iter()
            .any(|(cause, _)| *cause == CheckpointCause::Commit)
    })
    .await;
    std::fs::write(signals.join("finish"), "").unwrap();
    rig.stop().await.unwrap();

    let after_commit = taken(&rig, &team)
        .into_iter()
        .find(|(cause, _)| *cause == CheckpointCause::Commit)
        .unwrap();
    assert_eq!(after_commit.1, None);
    let failed = Reader::open(&rig.home.history_file())
        .unwrap()
        .daemon_logs()
        .unwrap()
        .into_iter()
        .filter(|entry| entry.event() == LogEvent::CheckpointFailed)
        .count();
    assert_ne!(failed, 0);
}

/// A reviewer that asks for changes the first time and approves afterwards.
const CHANGES_ONCE: &str = "verify:VERDICT: REQUEST_CHANGES\\nFINDINGS\\n1. [MAJOR] TDD-2 src/lib.rs:7 — the case is untested → add coverage\\n   CHECK: read src/lib.rs:7,verify:VERDICT: APPROVE\\nSUMMARY: fixed";

/// The listing line of every checkpoint of `team`, told from the records the daemon wrote, oldest first.
fn expected_listing(rig: &Rig, team: &TeamId) -> Vec<String> {
    let reader = Reader::open(&rig.home.history_file()).unwrap();
    reader
        .team_records(team)
        .unwrap()
        .into_iter()
        .filter_map(|stored| match stored.record {
            Record::Event(event) => match event.event {
                HistoryEvent::Checkpoint {
                    cause,
                    pull_request,
                    head,
                    draft,
                    review_pass,
                    snapshot,
                    content,
                    ..
                } => {
                    let pull = pull_request.map_or("none".to_owned(), |number| {
                        format!("#{} {}", number.get(), if draft { "draft" } else { "ready" })
                    });
                    let content = content.expect("the daemon wrote a content");
                    let totals = |text: &str| {
                        text.lines()
                            .next()
                            .unwrap()
                            .strip_prefix("Totals: ")
                            .unwrap()
                            .to_owned()
                    };
                    Some(format!(
                        "{}  {cause}  PR {pull}  head {}  pass {}  done {}  left {}  snapshot {}",
                        event.at,
                        head.map_or("none".to_owned(), |head| head.as_str().chars().take(7).collect()),
                        review_pass.unwrap_or(0),
                        totals(&content.done),
                        totals(&content.left),
                        snapshot.unwrap_or_else(|| "none".to_owned()),
                    ))
                }
                _ => None,
            },
            _ => None,
        })
        .collect()
}

#[tokio::test]
async fn agent_checkpoints_lists_the_round_of_a_whole_run_as_the_daemon_wrote_it() {
    let mut rig = Rig::new(
        &[
            normal("claude-1", "work,fix-work"),
            investigator("inv-r1", CHANGES_ONCE),
            investigator("inv-r2", APPROVING),
        ],
        5,
        false,
    )
    .await;
    rig.start().await;
    let (team, tasks) = rig.run(&["claude-1"], ONE, MergeMode::Autopilot, None).await;
    rig.item_where("1.1", "done", |phase| matches!(phase, ItemPhase::Done { .. }))
        .await;
    rig.stop().await.unwrap();
    let request = CheckpointsRequest {
        target: Target::Task(tasks[0].clone()),
        mode: Mode::Listing,
    };
    let mut out = Vec::new();

    checkpoints_in(
        &rig.home.history_file(),
        &rig.home.root().join("prompts"),
        &request,
        &mut out,
    )
    .await
    .unwrap();

    let listing = String::from_utf8(out).unwrap();
    let lines: Vec<&str> = listing.lines().collect();
    assert_eq!(lines, expected_listing(&rig, &team));
    assert!(
        lines.first().unwrap().contains("  round_opened  PR none  "),
        "{listing}"
    );
    assert!(
        lines.iter().any(|line| line.contains("  pull_ready  PR #1 ready  ")),
        "{listing}"
    );
    assert!(
        lines
            .iter()
            .any(|line| line.contains("  fix_pass_started  PR #1 ready  ")),
        "{listing}"
    );
    assert!(
        lines.last().unwrap().contains("  pull_merged  PR #1 ready  "),
        "{listing}"
    );
}
