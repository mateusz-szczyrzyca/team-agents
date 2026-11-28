//! `agent status` says that a queued task waits for its account when the account has its `max_runs` going, and writes the
//! lines of every other task as it always did.

use std::collections::BTreeMap;

use agent_core::config::{AbsolutePath, Config};
use agent_core::ids::TaskId;
use agent_core::local_time::Zone;
use agent_core::protocol::{Frame, Reply, Response, TeamSummary};
use agent_core::state::State;
use agent_core::status::TaskStatus;
use agent_runtime::store::Home;
use time::UtcOffset;

use super::accounts::waits;
use super::{Activity, Detail, Under, View, Waiting, render_with_waits, status_active};
use crate::client::Connection;
use crate::fixtures::{task, team_state};
use crate::testing::{Answer, FakeDaemon};

/// The configuration: the investigators `inv-opus` and `inv-codex` of the sample team share the account `inv-shared`
/// with `max_runs` as `max_runs` says.
fn config_text(max_runs: &str) -> String {
    format!(
        "[accounts.inv-shared]\nowner = \"inv\"\n{max_runs}\n\n\
         [agents.claude-1]\ncli = \"claude\"\n\n\
         [investigators.inv-opus]\ncli = \"claude\"\naccount = \"inv-shared\"\n\n\
         [investigators.inv-codex]\ncli = \"codex\"\naccount = \"inv-shared\"\n\n\
         [tiers]\nnormal = [\"claude-1\"]\nworkers = []\n\
         [binaries]\nclaude = \"claude\"\ncodex = \"codex\"\nagentbus = \"agentbus\"\n"
    )
}

/// The sample team with the run of its coordinator `inv-opus` going; the reviewer `inv-codex` is queued.
fn one_running() -> State {
    let mut state = team_state();
    {
        let (_, mut member) = state.find_task_mut(&task(1)).unwrap();
        member.transition(TaskStatus::Running).unwrap();
    }
    state
}

fn summaries(state: &State) -> Vec<TeamSummary> {
    state.teams().iter().map(TeamSummary::from).collect()
}

#[test]
fn a_queued_task_of_a_full_account_waits_for_it() {
    let state = one_running();
    let config = Config::from_toml_str(&config_text("max_runs = 1")).unwrap();

    assert_eq!(
        waits(&config, &state, &summaries(&state)),
        BTreeMap::from([(task(4), "waiting for account inv-shared (1/1 runs)".to_owned())])
    );
}

#[test]
fn a_queued_task_of_an_account_below_its_limit_does_not_wait() {
    let state = one_running();
    let config = Config::from_toml_str(&config_text("max_runs = 2")).unwrap();

    assert_eq!(waits(&config, &state, &summaries(&state)), BTreeMap::new());
}

#[test]
fn a_queued_task_of_an_account_without_a_limit_does_not_wait() {
    let state = one_running();
    let config = Config::from_toml_str(&config_text("")).unwrap();

    assert_eq!(waits(&config, &state, &summaries(&state)), BTreeMap::new());
}

#[test]
fn the_runs_of_tasks_that_are_not_listed_count_for_the_account() {
    let state = one_running();
    let config = Config::from_toml_str(&config_text("max_runs = 1")).unwrap();
    let mut only_reviewer = summaries(&state);
    only_reviewer[0].members.retain(|member| member.task == task(4));

    assert_eq!(
        waits(&config, &state, &only_reviewer).keys().collect::<Vec<&TaskId>>(),
        vec![&task(4)]
    );
}

/// The status of `teams` with `waits` as what the tasks that wait for their account say.
fn shown(teams: &[TeamSummary], waits: &BTreeMap<TaskId, String>) -> String {
    render_with_waits(
        teams,
        View::Full,
        Detail::Tasks,
        &|_| None,
        &|_| String::new(),
        Waiting {
            under: Under {
                requests: &|_| Vec::new(),
                workers: &|_| Vec::new(),
            },
            accounts: &|task| waits.get(task).cloned(),
        },
        Zone::Fixed(UtcOffset::UTC),
    )
}

#[test]
fn status_line_says_the_task_waits_for_its_account() {
    let state = one_running();
    let waits = BTreeMap::from([(task(4), "waiting for account inv-shared (1/1 runs)".to_owned())]);

    assert_eq!(
        shown(&summaries(&state), &waits),
        "\
team-20261111-153000-t5m8  team-agents  autopilot  2 pending, 0 in progress, 0 in review, 0 approved, 0 done
  task-20261111-153001-k3x9  coordinator  inv-opus   running
  task-20261111-153002-k3x9  member       claude-1   queued
  task-20261111-153003-k3x9  worker       glm-1      queued
  task-20261111-153004-k3x9  reviewer     inv-codex  queued, waiting for account inv-shared (1/1 runs)
"
    );
}

#[test]
fn status_line_of_a_task_that_runs_does_not_say_it_waits() {
    let state = one_running();
    let waits = BTreeMap::from([(task(1), "waiting for account inv-shared (1/1 runs)".to_owned())]);

    let text = shown(&summaries(&state), &waits);

    assert!(!text.contains("waiting for account"), "{text}");
}

// Rule: `agent status` reads the configuration and the state of the daemon, so that the account of a task that waits is
// said without the daemon telling it.
#[tokio::test]
async fn the_status_reads_the_accounts_that_hold_a_task() {
    let dir = tempfile::tempdir().unwrap();
    let home = Home::new(&AbsolutePath::try_from(dir.path().to_str().unwrap().to_owned()).unwrap());
    std::fs::write(home.config_file(), config_text("max_runs = 1")).unwrap();
    let state = one_running();
    std::fs::write(home.state_file(), state.to_json().unwrap()).unwrap();
    let activity = Activity::new(home, Zone::Fixed(UtcOffset::UTC));
    let teams = summaries(&state);
    let daemon = FakeDaemon::serve(&dir.path().join("agent.sock"), move |request| {
        Answer::Lines(vec![
            Frame::Response(Response::ok(request.id, Reply::Teams { teams: teams.clone() }))
                .to_line()
                .unwrap(),
        ])
    });
    let mut connection = Connection::connect(daemon.socket()).await.unwrap();
    let mut out = Vec::new();

    status_active(&mut connection, None, &activity, &mut out).await.unwrap();

    let text = String::from_utf8(out).unwrap();
    assert!(
        text.contains("reviewer     inv-codex [inv-shared]  queued, waiting for account inv-shared (1/1 runs)\n"),
        "{text}"
    );
}
