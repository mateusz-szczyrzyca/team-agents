//! The reads of a worktree's commit that the owner makes in a round (before a merge, after a fix, of the commit a
//! reviewer verified) wait when git fails them, as a step that GitHub fails does: the daemon goes on, the round says
//! what it waits for, and `agent resume` of the team tries the step again.
use std::path::{Path, PathBuf};
use std::time::Duration;

use agent_core::config::Secrets;
use agent_core::ids::{RoundId, TaskId, TeamId};
use agent_core::log::{LogEvent, LogLevel};
use agent_core::protocol::{Client, Command, ControlRequest, Reply, ShowRequest, Target};
use agent_core::state::{ItemPhase, MergeMode, OpinionStage};
use agent_runtime::history::{HistoryLayer, Reader};

use super::rig::{ONE, PATIENCE, Rig, investigator, normal};

/// The report of a first review that asks for one change.
const CHANGES: &str = "VERDICT: REQUEST_CHANGES\\nFINDINGS\\n1. [MAJOR] TDD-2 src/lib.rs:7 — missing regression → add it\\n   CHECK: read src/lib.rs:7";
/// The report of a first review that approves.
const APPROVE: &str = "VERDICT: APPROVE\\nSUMMARY: fine";
/// The report of a verification that finds the change made.
const VERIFIED: &str = "VERDICT: APPROVE\\nFINDING 1: RESOLVED\\nSUMMARY: verified";

/// Starts the daemon of `rig` with its log kept in the history, from `info` up.
async fn start(rig: &mut Rig) {
    let layer = HistoryLayer::deferred(LogLevel::Info, rig.history_test_clock(), Secrets::default());
    rig.start_with_log_layer(layer).await;
}

/// How many times the daemon logged `event`, a repeated entry counted as often as it was recorded.
fn times(rig: &Rig, event: LogEvent) -> u64 {
    let Ok(reader) = Reader::open(&rig.home.history_file()) else {
        return 0;
    };
    reader
        .daemon_logs()
        .unwrap()
        .iter()
        .filter(|row| row.event() == event)
        .map(|row| row.count().get())
        .sum()
}

/// What the daemon says `round` waits for.
async fn waits_for(rig: &Rig, round: RoundId) -> Option<String> {
    let reply = crate::client::Connection::connect(&rig.home.socket_file())
        .await
        .unwrap()
        .request(
            Client::Cli,
            Command::Show(ShowRequest {
                target: Target::Round(round),
            }),
        )
        .await
        .unwrap();
    let Reply::Round { round } = reply else {
        panic!("expected the detail of a round: {reply:?}");
    };
    round.waiting
}

/// Asks the daemon to resume `team`, as `agent resume <team>` does; a team whose tasks all run is refused for them.
async fn resume_team(rig: &Rig, team: &TeamId) {
    let answer = crate::client::Connection::connect(&rig.home.socket_file())
        .await
        .unwrap()
        .request(
            Client::Cli,
            Command::Resume(ControlRequest {
                target: Target::Team(team.clone()),
            }),
        )
        .await;
    assert!(
        matches!(
            answer,
            Ok(Reply::Applied { .. }) | Err(crate::client::ClientError::Refused(_))
        ),
        "{answer:?}"
    );
}

/// The round of the first item of `team`.
fn round_of(rig: &Rig, team: &TeamId) -> RoundId {
    rig.state().team(team).unwrap().rounds().last().unwrap().id().clone()
}

/// A rig of one lead acting `lead` and four investigators whose reviews the test gives one by one.
async fn held_rig(lead: &'static str) -> Rig {
    Rig::new(
        &[
            normal("claude-1", lead),
            investigator("inv-r1", "held-review,held-review"),
            investigator("inv-r2", "held-review,held-review"),
            investigator("inv-r3", "held-review,held-review"),
            investigator("inv-r4", "held-review,held-review"),
        ],
        5,
        false,
    )
    .await
}

/// A reviewer whose run waits for the report the test gives it.
struct Held {
    /// The reviewer's task.
    task: TaskId,
    /// The pipe its run reads the report from.
    pipe: PathBuf,
}

/// The `count` reviewers of `team` that wait for their report and are not among `before`.
async fn held_reviews(rig: &Rig, team: &TeamId, before: &[TaskId], count: usize) -> Vec<Held> {
    let deadline = std::time::Instant::now() + PATIENCE;
    loop {
        let state = rig.state();
        let mut held = Vec::new();
        for member in state.team(team).map_or(&[][..], |found| found.reviewers()) {
            if before.contains(member.task()) {
                continue;
            }
            let worktree = rig.worktree(member.task());
            let Ok(entries) = std::fs::read_dir(&worktree) else {
                continue;
            };
            for entry in entries {
                let name = entry.unwrap().file_name().into_string().unwrap();
                if let Some(n) = name.strip_prefix(".fake-review-ready-") {
                    held.push(Held {
                        task: member.task().clone(),
                        pipe: worktree.join(format!(".fake-review-result-{n}")),
                    });
                }
            }
        }
        if held.len() >= count {
            return held;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "timed out waiting for {count} held reviews"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

/// Gives `held` its report `report`, one line with `\n` for the line breaks.
async fn release(held: &Held, report: &'static str) {
    let pipe = held.pipe.clone();
    tokio::task::spawn_blocking(move || std::fs::write(pipe, format!("{report}\n")))
        .await
        .unwrap()
        .unwrap();
}

/// The `HEAD` of the git worktree at `worktree`, made to name a branch that does not exist until it is restored: git
/// cannot read the worktree's commit, and every other git call of the worktree still works.
struct BrokenHead {
    /// The `HEAD` file of the worktree.
    file: PathBuf,
    /// What it held.
    was: String,
}

impl BrokenHead {
    /// Breaks the `HEAD` of `worktree`.
    fn of(worktree: &Path) -> Self {
        let link = std::fs::read_to_string(worktree.join(".git")).unwrap();
        let gitdir = PathBuf::from(link.trim().strip_prefix("gitdir: ").unwrap());
        let file = gitdir.join("HEAD");
        let was = std::fs::read_to_string(&file).unwrap();
        std::fs::write(&file, "ref: refs/heads/gone-for-the-test\n").unwrap();
        Self { file, was }
    }

    /// Gives the worktree its `HEAD` back.
    fn restore(self) {
        std::fs::write(&self.file, &self.was).unwrap();
    }
}

/// Runs the first review of the item of `team`: one reviewer asks for a change, the other approves.
async fn first_review(rig: &Rig, team: &TeamId) -> Vec<TaskId> {
    let first = held_reviews(rig, team, &[], 2).await;
    release(&first[0], CHANGES).await;
    release(&first[1], APPROVE).await;
    first.into_iter().map(|held| held.task).collect()
}

/// Waits for a step that waits and returns what the round of `team` says it waits for.
async fn waiting_line(rig: &Rig, team: &TeamId) -> String {
    rig.wait_for("a step that waits", || times(rig, LogEvent::GithubStepWaiting) > 0)
        .await;
    waits_for(rig, round_of(rig, team)).await.unwrap_or_default()
}

#[tokio::test]
async fn a_merge_whose_read_of_the_lead_s_commit_fails_waits_and_merges_after_resume() {
    let mut rig = Rig::new(
        &[
            normal("claude-1", "work,opinion-ask"),
            investigator("inv-r1", "opinion-release"),
            investigator("inv-r2", "opinion-release"),
            investigator("inv-r3", "opinion-release"),
            investigator("inv-r4", "opinion-release"),
        ],
        1,
        false,
    )
    .await;
    start(&mut rig).await;
    let (team, tasks) = rig.run(&["claude-1"], ONE, MergeMode::Autopilot, None).await;
    rig.state_where("the second opinions under way", |state| {
        let found = state.team(&team).unwrap();
        found.items()[0].findings().is_some_and(|ledger| {
            ledger.opinions().len() == 2
                && ledger.opinions().values().all(|opinion| {
                    let OpinionStage::Assigned { task, .. } = &opinion.stage else {
                        return false;
                    };
                    found.find(task).unwrap().1.current_run().is_some()
                })
        })
    })
    .await;

    // The opinions do not confirm the findings, so the approval stands and the merge reads the lead's commit.
    let head = BrokenHead::of(&rig.worktree(&tasks[0]));
    std::fs::write(rig.home.root().join("opinion-release"), "ready").unwrap();
    let waiting = waiting_line(&rig, &team).await;
    assert!(
        waiting.starts_with("GitHub: read the lead's commit before the merge #1, refused: "),
        "{waiting}"
    );
    assert_eq!(times(&rig, LogEvent::DaemonTaskFailed), 0);

    head.restore();
    resume_team(&rig, &team).await;
    rig.item_where("1.1", "merged after the resume", |phase| {
        matches!(phase, ItemPhase::Done { .. })
    })
    .await;
    rig.stop().await.unwrap();

    assert_eq!(times(&rig, LogEvent::DaemonTaskFailed), 0);
    assert_eq!(times(&rig, LogEvent::GithubStepResumed), 1);
}

#[tokio::test]
async fn a_verdict_whose_read_of_the_reviewed_commit_fails_waits_and_is_taken_after_resume() {
    let mut rig = held_rig("work,fix-work-pushed").await;
    start(&mut rig).await;
    let (team, _) = rig.run(&["claude-1"], ONE, MergeMode::Autopilot, None).await;
    let first = first_review(&rig, &team).await;
    let verifiers = held_reviews(&rig, &team, &first, 2).await;

    let heads: Vec<BrokenHead> = verifiers
        .iter()
        .map(|held| BrokenHead::of(&rig.worktree(&held.task)))
        .collect();
    for held in &verifiers {
        release(held, VERIFIED).await;
    }
    let waiting = waiting_line(&rig, &team).await;
    assert!(
        waiting.starts_with("GitHub: read the commit a reviewer verified #1, refused: "),
        "{waiting}"
    );
    assert_eq!(times(&rig, LogEvent::DaemonTaskFailed), 0);

    for head in heads {
        head.restore();
    }
    resume_team(&rig, &team).await;
    rig.item_where("1.1", "merged after the resume", |phase| {
        matches!(phase, ItemPhase::Done { .. })
    })
    .await;
    rig.stop().await.unwrap();

    assert_eq!(times(&rig, LogEvent::DaemonTaskFailed), 0);
}

#[tokio::test]
async fn a_fix_response_whose_read_of_the_lead_s_commit_fails_waits_and_is_read_after_resume() {
    let mut rig = held_rig("work,fix-gated").await;
    start(&mut rig).await;
    let (team, tasks) = rig.run(&["claude-1"], ONE, MergeMode::Autopilot, None).await;
    first_review(&rig, &team).await;
    let lead = rig.worktree(&tasks[0]);
    rig.wait_for("the lead's fix committed", || lead.join(".fake-committed-2").exists())
        .await;

    let head = BrokenHead::of(&lead);
    let gate = lead.join(".fake-gate-done-2");
    tokio::task::spawn_blocking(move || std::fs::write(gate, "done\n"))
        .await
        .unwrap()
        .unwrap();
    let waiting = waiting_line(&rig, &team).await;
    assert!(
        waiting.starts_with("GitHub: read the lead's commit after its fix #1, refused: "),
        "{waiting}"
    );
    assert_eq!(times(&rig, LogEvent::DaemonTaskFailed), 0);

    head.restore();
    resume_team(&rig, &team).await;
    rig.wait_for("the read goes through", || times(&rig, LogEvent::GithubStepResumed) > 0)
        .await;
    assert_eq!(times(&rig, LogEvent::DaemonTaskFailed), 0);
    rig.stop().await.unwrap();
}
