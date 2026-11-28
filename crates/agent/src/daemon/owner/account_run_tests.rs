//! `max_runs` of an account: a task whose account has its most runs going waits in the queue, not failed, and starts when
//! one of those runs ends.

use agent_core::state::{RunEnd, State};
use agent_core::status::TaskStatus;

use super::fixture::{Daemon, pair_state};
use crate::fixtures::{at, run, task};

/// The pair of tasks of the fixture with the run of task 1 going.
fn one_running() -> State {
    let mut state = pair_state();
    {
        let (_, mut member) = state.find_task_mut(&task(1)).unwrap();
        member.transition(TaskStatus::Running).unwrap();
        member.begin_run(run(1), at(0), None).unwrap();
    }
    state
}

/// Gives both agents of the fixture the account `claude-main` of owner `claude`, with `extra` in its table.
fn share_an_account(daemon: &Daemon, extra: &str) {
    let path = daemon.home.config_file();
    let text = std::fs::read_to_string(&path).unwrap();
    let text = ["claude-1", "claude-2"].iter().fold(text, |text, agent| {
        let header = format!("[agents.{agent}]\ncli = \"claude\"\n");
        assert!(text.contains(&header), "{text}");
        text.replace(&header, &format!("{header}account = \"claude-main\"\n"))
    });
    std::fs::write(
        &path,
        format!("{text}\n[accounts.claude-main]\nowner = \"claude\"\n{extra}\n"),
    )
    .unwrap();
}

/// The status of the task `n` in the state of `owner`.
fn status_of(owner: &super::Owner, n: u32) -> TaskStatus {
    owner.parts.state.find_task(&task(n)).unwrap().member.status()
}

// Rule: the second run of an account whose `max_runs` is 1 waits, queued and with no run of its own, while the first goes;
// when the first has ended it starts.
#[tokio::test]
async fn max_runs_one_second_run_waits_then_starts() {
    let daemon = Daemon::new(&one_running(), None);
    share_an_account(&daemon, "max_runs = 1");
    daemon.scenario(2, "done");
    let (mut owner, _sinks) = daemon.bare_owner().await;

    owner.start_queued().await.unwrap();

    assert_eq!(status_of(&owner, 2), TaskStatus::Queued);
    assert_eq!(owner.parts.state.find_task(&task(2)).unwrap().member.runs().len(), 0);
    {
        let (_, mut first) = owner.parts.state.find_task_mut(&task(1)).unwrap();
        first.end_run(at(1), RunEnd::Stopped).unwrap();
        first.transition(TaskStatus::Done).unwrap();
    }

    owner.start_queued().await.unwrap();

    assert_eq!(owner.parts.state.find_task(&task(2)).unwrap().member.runs().len(), 1);
}

// Rule: without `max_runs` the account has no limit: both agents run at once.
#[tokio::test]
async fn an_account_without_max_runs_starts_a_run_beside_a_running_one() {
    let daemon = Daemon::new(&one_running(), None);
    share_an_account(&daemon, "");
    daemon.scenario(2, "done");
    let (mut owner, _sinks) = daemon.bare_owner().await;

    owner.start_queued().await.unwrap();

    assert_eq!(owner.parts.state.find_task(&task(2)).unwrap().member.runs().len(), 1);
}

// Rule: a limit of two lets the second run start beside the running one.
#[tokio::test]
async fn max_runs_two_lets_the_second_run_start() {
    let daemon = Daemon::new(&one_running(), None);
    share_an_account(&daemon, "max_runs = 2");
    daemon.scenario(2, "done");
    let (mut owner, _sinks) = daemon.bare_owner().await;

    owner.start_queued().await.unwrap();

    assert_eq!(owner.parts.state.find_task(&task(2)).unwrap().member.runs().len(), 1);
}
