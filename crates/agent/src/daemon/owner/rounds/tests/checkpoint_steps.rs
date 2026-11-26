//! The steps of a round that leave a checkpoint where no record of the history names them: the wait for the human after
//! "not published", a hand-over to another agent, a restart with a draft pull request, a commit of a whole round, and the
//! checkpoint of the start of a fix pass, which is durable before the fix agent runs.

use agent_core::history::CheckpointCause;
use agent_core::history::{HistoryEvent, Record};
use agent_core::ids::{TaskId, TeamId};
use agent_core::state::{CommitId, ItemPhase, MergeMode};
use agent_core::status::TaskStatus;
use agent_runtime::history::Reader;

use super::rig::{ONE, Rig, git, investigator, normal};
use crate::client::Connection;
use crate::daemon::owner::fixture::{answers, n};
use agent_core::protocol::{AnswerRequest, Client, Command, ControlRequest, Target};

/// A reviewer that approves.
const APPROVING: &str = "verify:VERDICT: APPROVE\\nSUMMARY: fine";

/// A reviewer that asks for changes the first time and approves afterwards.
const CHANGES_ONCE: &str = "verify:VERDICT: REQUEST_CHANGES\\nFINDINGS\\n1. [MAJOR] TDD-2 src/lib.rs:7 — the case is untested → add coverage\\n   CHECK: read src/lib.rs:7,verify:VERDICT: APPROVE\\nSUMMARY: fixed";

/// What a checkpoint says that these tests look at.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Seen {
    /// The step it follows.
    cause: CheckpointCause,
    /// The task whose worktree it keeps.
    task: TaskId,
    /// The number of the pull request it names.
    pull_request: Option<u32>,
    /// Whether the pull request was a draft.
    draft: bool,
    /// The tip of the worktree's branch.
    head: Option<CommitId>,
    /// The fix pass of the round then.
    fix_pass: Option<u32>,
}

/// The checkpoints of `team`, in the order they were written.
fn seen(rig: &Rig, team: &TeamId) -> Vec<Seen> {
    let reader = Reader::open(&rig.home.history_file()).unwrap();
    let mut found = Vec::new();
    for stored in reader.team_records(team).unwrap() {
        if let Record::Event(event) = stored.record
            && let HistoryEvent::Checkpoint {
                cause,
                task,
                pull_request,
                draft,
                head,
                fix_pass,
                ..
            } = event.event
        {
            found.push(Seen {
                cause,
                task,
                pull_request: pull_request.map(|number| number.get().get()),
                draft,
                head,
                fix_pass,
            });
        }
    }
    found
}

/// Whether `team` has a checkpoint of `cause`.
fn has(rig: &Rig, team: &TeamId, cause: CheckpointCause) -> bool {
    seen(rig, team).iter().any(|seen| seen.cause == cause)
}

/// Makes the stand-in script `name` of `rig`, which runs `body` in the worktree after a prelude that commits as the test
/// says, and returns the directory of the files the script and the test signal each other with.
fn script(rig: &Rig, name: &str, body: &str) -> std::path::PathBuf {
    let signals = rig.home.root().join("signals");
    std::fs::create_dir_all(&signals).unwrap();
    let scripts = rig.home.root().join("fake-scripts");
    std::fs::create_dir_all(&scripts).unwrap();
    let text = format!(
        "set -eu\nsig='{}'\n\
         g() {{ git -c user.name=Fake -c user.email=fake@example.com -c commit.gpgsign=false \"$@\"; }}\n\
         wait_for() {{ while [ ! -e \"$sig/$1\" ]; do sleep 0.05; done; }}\n{body}",
        signals.display()
    );
    std::fs::write(scripts.join(format!("{name}.sh")), text).unwrap();
    signals
}

#[tokio::test]
async fn a_lead_sent_back_for_its_gate_too_often_leaves_a_checkpoint_that_the_round_waits_for_the_human() {
    let mut rig = Rig::new(
        &[
            normal("claude-1", "commit,commit,commit,commit,commit"),
            investigator("inv-r1", APPROVING),
            investigator("inv-r2", APPROVING),
        ],
        5,
        false,
    )
    .await;
    rig.start().await;
    let (team, _) = rig.run(&["claude-1"], ONE, MergeMode::Autopilot, None).await;

    rig.wait_for("the checkpoint of the wait for the human", || {
        has(&rig, &team, CheckpointCause::WaitsForHuman)
    })
    .await;

    rig.stop().await.unwrap();
}

#[tokio::test]
async fn a_task_handed_over_because_its_agent_is_busy_leaves_a_checkpoint_of_the_change_of_holder() {
    let mut rig = Rig::new(
        &[
            investigator("inv-lead", "sleep,integrate"),
            normal("claude-1", "hold"),
            normal("claude-2", "work"),
            investigator("inv-r1", APPROVING),
            investigator("inv-r2", APPROVING),
        ],
        5,
        true,
    )
    .await;
    let mut config = std::fs::read_to_string(rig.home.config_file()).unwrap();
    config.push_str("\n[replacement]\nagents = [\"claude-2\"]\nbusy_after = 3600\n");
    std::fs::write(rig.home.config_file(), config).unwrap();
    rig.start().await;
    let (_, busy) = rig.run(&["claude-1"], ONE, MergeMode::NoMerge, None).await;
    rig.status_is(&busy[0], TaskStatus::Running).await;
    let (team, tasks) = rig
        .run(&["inv-lead", "claude-1"], ONE, MergeMode::Autopilot, Some("inv-lead"))
        .await;
    rig.state_where("the member waits for claude-1", |state| {
        state
            .find_task(&tasks[1])
            .is_some_and(|found| found.member.status() == TaskStatus::Queued && found.member.waiting_since().is_some())
    })
    .await;

    rig.advance_history_days(1);

    rig.wait_for("the checkpoint of the change of holder", || {
        seen(&rig, &team)
            .iter()
            .any(|seen| seen.cause == CheckpointCause::HolderChanged && seen.task == tasks[1])
    })
    .await;
    rig.stop().await.unwrap();
}

#[tokio::test]
async fn a_split_part_given_to_a_replacement_leaves_a_checkpoint_of_the_change_of_holder() {
    let mut rig = Rig::new(
        &[
            investigator("inv-lead", "ask,integrate"),
            normal("claude-1", "work,sleep"),
            normal("claude-2", "work"),
            investigator("inv-r1", "done:VERDICT: APPROVE"),
            investigator("inv-r2", "done:VERDICT: APPROVE"),
        ],
        5,
        true,
    )
    .await;
    let mut config = std::fs::read_to_string(rig.home.config_file()).unwrap();
    config.push_str("\n[replacement]\nagents = [\"claude-2\"]\n");
    std::fs::write(rig.home.config_file(), config).unwrap();
    rig.start().await;
    let (team, tasks) = rig.run(&["inv-lead", "claude-1"], ONE, MergeMode::NoMerge, None).await;
    rig.status_is(&tasks[0], TaskStatus::Questions).await;
    rig.status_is(&tasks[1], TaskStatus::Waiting).await;
    let plan = serde_json::json!({"parent": "1.1", "parts": [
        {"title": "Document 0", "delivers": "Independent document 0", "checks": "Read document 0", "depends_on": []},
        {"title": "Document 1", "delivers": "Independent document 1", "checks": "Read document 1", "depends_on": ["a"]},
    ]});
    Connection::connect(&rig.home.socket_file())
        .await
        .unwrap()
        .request(
            Client::Cli,
            Command::Answer(AnswerRequest {
                task: tasks[0].clone(),
                number: n(1),
                answers: answers(&[(1, &format!("split\n```json\n{plan}\n```"))]),
            }),
        )
        .await
        .unwrap();
    rig.wait_for("part a approved", || {
        let wire: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(rig.home.state_file()).unwrap()).unwrap();
        wire["teams"][0]["items"][0]["phase"]["phase"] == "approved"
    })
    .await;
    Connection::connect(&rig.home.socket_file())
        .await
        .unwrap()
        .request(
            Client::Cli,
            Command::Stop(ControlRequest {
                target: Target::Task(tasks[1].clone()),
            }),
        )
        .await
        .unwrap();
    rig.status_is(&tasks[1], TaskStatus::Stopped).await;

    rig.github.merged_by_a_human(1);

    rig.wait_for("the checkpoint of the change of holder", || {
        seen(&rig, &team)
            .iter()
            .any(|seen| seen.cause == CheckpointCause::HolderChanged && seen.task == tasks[1])
    })
    .await;
    rig.stop().await.unwrap();
}

#[tokio::test]
async fn a_draft_pull_request_and_its_number_are_kept_by_a_restart_before_it_is_marked_ready() {
    let mut rig = Rig::new(
        &[
            normal("claude-1", "work"),
            investigator("inv-r1", APPROVING),
            investigator("inv-r2", APPROVING),
        ],
        5,
        false,
    )
    .await;
    // GitHub refuses to mark the pull request ready: it stays a draft while the daemon stops and starts again.
    rig.github.fail_calls_to("POST", "/graphql", 503);
    rig.start().await;
    let (team, tasks) = rig.run(&["claude-1"], ONE, MergeMode::Autopilot, None).await;
    rig.wait_for("the draft pull request", || {
        has(&rig, &team, CheckpointCause::PullOpened)
    })
    .await;
    rig.wait_for("the refused marking as ready", || {
        !rig.github.calls("POST", "/graphql").is_empty()
    })
    .await;
    rig.stop().await.unwrap();
    // A start of the daemon takes a checkpoint of a worktree that is not as its last checkpoint kept it.
    std::fs::write(
        rig.worktree(&tasks[0]).join("notes.txt"),
        "written while the daemon was down",
    )
    .unwrap();
    let before = seen(&rig, &team).len();

    rig.start().await;
    rig.wait_for("the checkpoint of the start", || {
        seen(&rig, &team)
            .iter()
            .skip(before)
            .any(|seen| seen.cause == CheckpointCause::DaemonStarted)
    })
    .await;
    let number = rig.github.pulls().remove(0).number;
    let after = seen(&rig, &team).split_off(before);
    rig.stop().await.unwrap();

    let started = after
        .iter()
        .find(|seen| seen.cause == CheckpointCause::DaemonStarted)
        .unwrap();
    assert_eq!(
        (started.pull_request, started.draft),
        (Some(number), true),
        "{after:#?}"
    );
}

#[tokio::test]
async fn a_round_leaves_its_commits_with_the_heads_the_pushes_carry_in_the_order_of_its_steps() {
    let mut rig = Rig::new(
        &[
            normal("claude-1", "script:first-pass,fix-script:second-pass"),
            investigator("inv-r1", CHANGES_ONCE),
            investigator("inv-r2", APPROVING),
        ],
        5,
        false,
    )
    .await;
    let signals = script(
        &rig,
        "first-pass",
        "g commit -q --allow-empty -m one\ntouch \"$sig/one\"\nwait_for go-one\n",
    );
    script(
        &rig,
        "second-pass",
        "g commit -q --allow-empty -m two\ntouch \"$sig/two\"\nwait_for go-two\n",
    );
    rig.start().await;
    let (team, tasks) = rig.run(&["claude-1"], ONE, MergeMode::Autopilot, None).await;
    let worktree = rig.worktree(&tasks[0]);

    rig.wait_for("the first commit", || signals.join("one").exists()).await;
    let one = git(&worktree, &["rev-parse", "HEAD"]);
    rig.wait_for("the checkpoint of the first commit", || {
        seen(&rig, &team)
            .iter()
            .any(|seen| seen.cause == CheckpointCause::Commit && seen.head.as_ref().map(CommitId::as_str) == Some(&one))
    })
    .await;
    std::fs::write(signals.join("go-one"), "").unwrap();
    rig.wait_for("the second commit", || signals.join("two").exists()).await;
    let two = git(&worktree, &["rev-parse", "HEAD"]);
    rig.wait_for("the checkpoint of the second commit", || {
        seen(&rig, &team)
            .iter()
            .any(|seen| seen.cause == CheckpointCause::Commit && seen.head.as_ref().map(CommitId::as_str) == Some(&two))
    })
    .await;
    std::fs::write(signals.join("go-two"), "").unwrap();
    rig.item_where("1.1", "done", |phase| matches!(phase, ItemPhase::Done { .. }))
        .await;
    rig.stop().await.unwrap();

    let found = seen(&rig, &team);
    let position = |cause: CheckpointCause, head: &str| {
        found
            .iter()
            .position(|seen| seen.cause == cause && seen.head.as_ref().map(CommitId::as_str) == Some(head))
            .unwrap_or_else(|| panic!("no {cause:?} at {head}: {found:#?}"))
    };
    let kind = |cause: CheckpointCause| found.iter().position(|seen| seen.cause == cause).unwrap();
    // The first commit comes before the pull request opens, the second in the fix pass and before the push that carries it.
    assert!(
        position(CheckpointCause::Commit, &one) < kind(CheckpointCause::PullOpened),
        "{found:#?}"
    );
    let fix = kind(CheckpointCause::FixPassStarted);
    let second = position(CheckpointCause::Commit, &two);
    assert!(fix < second, "{found:#?}");
    assert_eq!(found[second].fix_pass, Some(1), "{found:#?}");
    let pushed = position(CheckpointCause::PullPushed, &two);
    assert!(second < pushed, "{found:#?}");
    assert_eq!(rig.remote_tip(&rig.github.pulls().remove(0).head), two);
}

#[tokio::test]
async fn the_start_of_a_fix_pass_is_a_checkpoint_before_the_fix_agent_runs() {
    let mut rig = Rig::new(
        &[
            normal("claude-1", "work,fix-script:hold-fix"),
            investigator("inv-r1", CHANGES_ONCE),
            investigator("inv-r2", APPROVING),
        ],
        5,
        false,
    )
    .await;
    let signals = script(
        &rig,
        "hold-fix",
        "g commit -q --allow-empty -m fix\ntouch \"$sig/fix-started\"\nwait_for go\n",
    );
    rig.start().await;
    let (team, _) = rig.run(&["claude-1"], ONE, MergeMode::Autopilot, None).await;

    rig.wait_for("the fix agent", || signals.join("fix-started").exists())
        .await;
    let started = has(&rig, &team, CheckpointCause::FixPassStarted);
    std::fs::write(signals.join("go"), "").unwrap();
    rig.stop().await.unwrap();

    assert!(
        started,
        "the fix agent ran before the checkpoint of its pass was in the history"
    );
}
