//! A step of the owner that GitHub or git fails never ends the daemon: the step waits with a growing pause, or for the
//! human's `agent resume` when the failure will not pass, the round says what it waits for, the other teams go on,
//! and the step is done by itself when it can be. The evidence of 20.11.2026: a read of a pull request timed out three
//! times and the daemon stopped for the night.
use agent_core::config::Secrets;
use agent_core::ids::{RoundId, TeamId};
use agent_core::log::{LogEvent, LogLevel};
use agent_core::protocol::{Client, Command, ControlRequest, Reply, ShowRequest, Target};
use agent_core::state::{ItemPhase, MergeMode, State};
use agent_runtime::history::{HistoryLayer, Reader};

use super::rig::{ONE, Rig, git, investigator, normal};

/// The pull request of the first team, as the daemon reads it.
const FIRST_PULL: &str = "/repos/octo/example/pulls/1";

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

/// Asks the daemon to resume `team`, as `agent resume <team>` does; whatever it answers about the team's tasks: one whose
/// tasks all run is refused for them, and its steps that wait for GitHub are tried again all the same.
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

/// Whether the first item of `team` is approved in `state`: its pull request waits for the human's merge.
fn approved(state: &State, team: &TeamId) -> bool {
    state
        .team(team)
        .and_then(|found| found.items().first())
        .is_some_and(|item| matches!(item.phase(), ItemPhase::Approved { .. }))
}

/// A rig of two leads and four investigators that approve.
async fn two_teams() -> Rig {
    Rig::new(
        &[
            normal("claude-1", "work"),
            normal("claude-2", "work"),
            investigator("inv-r1", "done:VERDICT: APPROVE\\nSUMMARY: fine"),
            investigator("inv-r2", "done:VERDICT: APPROVE\\nSUMMARY: fine"),
            investigator("inv-r3", "done:VERDICT: APPROVE\\nSUMMARY: fine"),
            investigator("inv-r4", "done:VERDICT: APPROVE\\nSUMMARY: fine"),
        ],
        5,
        false,
    )
    .await
}

/// Starts the first team of `rig`, whose pull request is approved and waits for the human's merge, and from then on
/// makes GitHub answer the reads of that pull request with `status`.
async fn approved_then_failing(rig: &mut Rig, status: u16) -> TeamId {
    start(rig).await;
    let (first, _) = rig.run(&["claude-1"], ONE, MergeMode::NoMerge, None).await;
    rig.state_where("the first pull request approved", |state| approved(state, &first))
        .await;
    rig.github.fail_calls_to("GET", FIRST_PULL, status);
    first
}

#[tokio::test]
async fn a_pull_request_github_does_not_answer_waits_while_another_team_goes_on_and_goes_through_when_it_answers() {
    let mut rig = two_teams().await;
    let first = approved_then_failing(&mut rig, 503).await;
    rig.wait_for("a step that waits for GitHub", || {
        times(&rig, LogEvent::GithubStepWaiting) > 0
    })
    .await;

    let waiting = waits_for(&rig, round_of(&rig, &first)).await.unwrap_or_default();
    assert!(
        waiting.starts_with("GitHub: refresh the description of the pull requests, tried "),
        "{waiting}"
    );
    assert!(waiting.contains(", next try "), "{waiting}");

    let (second, _) = rig.run(&["claude-2"], ONE, MergeMode::NoMerge, None).await;
    rig.state_where("the other team approved while GitHub fails the first", |state| {
        approved(state, &second)
    })
    .await;

    rig.github.answer_calls_to("GET", FIRST_PULL);
    rig.wait_for("the step that waited goes through", || {
        times(&rig, LogEvent::GithubStepResumed) > 0
    })
    .await;
    rig.stop().await.unwrap();

    assert_eq!(times(&rig, LogEvent::DaemonTaskFailed), 0);
    assert_eq!(times(&rig, LogEvent::GithubStepWaiting), 1);
    assert_eq!(times(&rig, LogEvent::GithubStepResumed), 1);
}

#[tokio::test]
async fn a_refusal_of_github_tells_the_human_once_and_agent_resume_of_the_team_tries_again() {
    let mut rig = two_teams().await;
    let first = approved_then_failing(&mut rig, 404).await;
    rig.notification_with("GitHub refuses").await;
    let reads = rig.github.calls("GET", FIRST_PULL).len();
    rig.wait_for("more looks at the team", || {
        rig.github.calls("GET", FIRST_PULL).len() >= reads + 3
    })
    .await;

    let refused: Vec<String> = rig
        .notifications()
        .into_iter()
        .filter(|text| text.contains("GitHub refuses"))
        .collect();
    assert_eq!(refused.len(), 1, "{refused:?}");
    assert!(refused[0].contains(&format!("agent resume {first}")), "{}", refused[0]);
    let waiting = waits_for(&rig, round_of(&rig, &first)).await.unwrap_or_default();
    assert!(
        waiting.starts_with("GitHub: refresh the description of the pull requests, refused: "),
        "{waiting}"
    );
    rig.wait_for("the one event of the refused step in the history", || {
        times(&rig, LogEvent::GithubStepWaiting) == 1
    })
    .await;

    resume_team(&rig, &first).await;
    rig.wait_for("the refused step tried again", || {
        times(&rig, LogEvent::GithubStepWaiting) == 2
    })
    .await;
    rig.github.answer_calls_to("GET", FIRST_PULL);
    resume_team(&rig, &first).await;
    rig.wait_for("the step goes through once GitHub answers", || {
        times(&rig, LogEvent::GithubStepResumed) == 1
    })
    .await;
    rig.stop().await.unwrap();

    assert_eq!(times(&rig, LogEvent::DaemonTaskFailed), 0);
    assert_eq!(
        rig.notifications()
            .iter()
            .filter(|text| text.contains("GitHub refuses"))
            .count(),
        1
    );
}

#[tokio::test]
async fn a_start_of_the_daemon_tries_a_waiting_step_at_once() {
    let mut rig = two_teams().await;
    approved_then_failing(&mut rig, 404).await;
    rig.wait_for("a refused step", || times(&rig, LogEvent::GithubStepWaiting) == 1)
        .await;
    rig.stop().await.unwrap();

    start(&mut rig).await;
    rig.wait_for("the step tried at the first look", || {
        times(&rig, LogEvent::GithubStepWaiting) == 2
    })
    .await;
    rig.stop().await.unwrap();

    assert_eq!(times(&rig, LogEvent::DaemonTaskFailed), 0);
}

#[tokio::test]
async fn a_second_opinion_whose_worktree_git_cannot_make_waits_for_the_human_and_is_asked_after_resume() {
    let mut rig = Rig::new(
        &[
            normal("claude-1", "work,opinion-ask"),
            investigator("inv-r1", "opinion-probe:NOT CONFIRMED"),
            investigator("inv-r2", "opinion-probe:NOT CONFIRMED"),
            investigator("inv-r3", "opinion-probe:NOT CONFIRMED"),
            investigator("inv-r4", "opinion-probe:NOT CONFIRMED"),
        ],
        1,
        false,
    )
    .await;
    start(&mut rig).await;
    let (team, tasks) = rig.run(&["claude-1"], ONE, MergeMode::Autopilot, None).await;
    rig.item_where("1.1", "the fix pass", |phase| matches!(phase, ItemPhase::Fixing { .. }))
        .await;
    // The investigators of the second opinions have no worktree yet: git cannot add one to a repository whose list of
    // worktrees it may not write.
    let common = git(
        &rig.worktree(&tasks[0]),
        &["rev-parse", "--path-format=absolute", "--git-common-dir"],
    );
    let worktrees = std::path::Path::new(&common).join("worktrees");
    let writable = std::fs::metadata(&worktrees).unwrap().permissions();
    let mut locked = writable.clone();
    locked.set_readonly(true);
    std::fs::set_permissions(&worktrees, locked).unwrap();
    rig.wait_for("a second opinion that waits", || {
        times(&rig, LogEvent::GithubStepWaiting) > 0
    })
    .await;

    let waiting = waits_for(&rig, round_of(&rig, &team)).await.unwrap_or_default();
    std::fs::set_permissions(&worktrees, writable).unwrap();
    assert!(
        waiting.starts_with("GitHub: prepare a second opinion #1, refused: "),
        "{waiting}"
    );
    assert_eq!(times(&rig, LogEvent::DaemonTaskFailed), 0);

    resume_team(&rig, &team).await;
    rig.item_where("1.1", "done after the opinions", |phase| {
        matches!(phase, ItemPhase::Done { .. })
    })
    .await;
    rig.stop().await.unwrap();

    assert_eq!(times(&rig, LogEvent::DaemonTaskFailed), 0);
    assert_eq!(times(&rig, LogEvent::GithubStepResumed), 1);
}

#[tokio::test]
async fn a_github_that_answers_no_call_ends_nothing_and_the_daemon_answers_status_until_it_is_back() {
    let mut rig = two_teams().await;
    let first = approved_then_failing(&mut rig, 503).await;
    rig.github.fail_every_call(503);
    let calls = rig.github.requests().len();
    rig.wait_for("many looks while GitHub is down", || {
        rig.github.requests().len() >= calls + 20
    })
    .await;

    let status = crate::client::Connection::connect(&rig.home.socket_file())
        .await
        .unwrap()
        .request(
            Client::Cli,
            Command::Status(agent_core::protocol::StatusRequest { target: None }),
        )
        .await
        .unwrap();
    assert!(matches!(status, Reply::Teams { .. }), "{status:?}");
    let waiting = waits_for(&rig, round_of(&rig, &first)).await.unwrap_or_default();
    assert!(waiting.starts_with("GitHub: "), "{waiting}");
    assert_eq!(times(&rig, LogEvent::DaemonTaskFailed), 0);

    rig.github.answer_again();
    rig.github.answer_calls_to("GET", FIRST_PULL);
    rig.wait_for("the step that waited goes through", || {
        times(&rig, LogEvent::GithubStepResumed) > 0
    })
    .await;
    rig.stop().await.unwrap();

    assert_eq!(times(&rig, LogEvent::DaemonTaskFailed), 0);
}
