//! The rate-limit readings a run shows are kept by the daemon with the account of the agent, or with the agent when it has
//! none, and are in the saved state: a daemon that starts again on the same home finds them there.

use agent_core::limit_reading::LimitWindow;
use agent_core::state::{KeptReading, ReadingOwner, State};
use agent_core::status::TaskStatus;

use super::fixture::Daemon;
use crate::fixtures::{at, solo_state};

/// A daemon with the solo task 1 whose stand-in shows the readings of Claude Code 2.1 and ends; the repository the run
/// works in has one commit.
fn daemon_with_reading() -> Daemon {
    let daemon = Daemon::new(&solo_state(), None);
    let worktree = daemon.scenario(1, "reading");
    for args in [
        &["init", "-q", "-b", "main"][..],
        &["commit", "-q", "--allow-empty", "-m", "Start"],
    ] {
        let done = std::process::Command::new("git")
            .current_dir(&worktree)
            .args(["-c", "user.name=Fixture", "-c", "user.email=fixture@example.com"])
            .args(args)
            .output()
            .unwrap();
        assert!(done.status.success(), "{}", String::from_utf8_lossy(&done.stderr));
    }
    daemon
}

/// Gives the agent `claude-1` of the daemon the account `claude-main` of owner `claude`.
fn give_the_account(daemon: &Daemon) {
    let path = daemon.home.config_file();
    let text = std::fs::read_to_string(&path).unwrap();
    let header = "[agents.claude-1]\ncli = \"claude\"\n";
    assert!(text.contains(header), "{text}");
    let text = text.replace(header, &format!("{header}account = \"claude-main\"\n"));
    std::fs::write(&path, format!("{text}\n[accounts.claude-main]\nowner = \"claude\"\n")).unwrap();
}

/// What the readings of `state` are at the time of the fixtures' clock, as `(owner, window, used, read at)`.
fn kept(state: &State) -> Vec<(String, LimitWindow, f64, KeptReading)> {
    state
        .limit_readings()
        .live(at(0))
        .into_iter()
        .flat_map(|(owner, readings)| {
            readings
                .into_iter()
                .map(move |kept| (owner.to_string(), kept.reading().window(), kept.reading().used(), kept))
        })
        .collect()
}

// Rule: the readings of a run of an agent without an account are kept with the agent, at the time the run ended.
#[tokio::test]
async fn fake_cli_reading_is_shown() {
    let mut daemon = daemon_with_reading();
    daemon.start().await;

    daemon.status_is(1, TaskStatus::Waiting).await;
    daemon.stop().await.unwrap();

    let readings = kept(&daemon.state());
    let shown: Vec<(&str, LimitWindow, f64)> = readings
        .iter()
        .map(|(owner, window, used, _)| (owner.as_str(), *window, *used))
        .collect();
    assert_eq!(
        shown,
        vec![
            ("claude-1", LimitWindow::FiveHour, 0.62),
            ("claude-1", LimitWindow::SevenDay, 0.41)
        ]
    );
    assert!(readings.iter().all(|(_, _, _, kept)| kept.read_at() == at(0)));
}

// Rule: the readings of an agent that has an account are the account's, not the agent's.
#[tokio::test]
async fn a_reading_of_an_agent_with_an_account_is_kept_with_the_account() {
    let mut daemon = daemon_with_reading();
    give_the_account(&daemon);
    daemon.start().await;

    daemon.status_is(1, TaskStatus::Waiting).await;
    daemon.stop().await.unwrap();

    let state = daemon.state();
    let live = state.limit_readings().live(at(0));
    let owners: Vec<&ReadingOwner> = live.iter().map(|(owner, _)| *owner).collect();
    assert_eq!(owners, vec![&ReadingOwner::Account("claude-main".parse().unwrap())]);
    assert_eq!(live[0].1.len(), 2);
}
