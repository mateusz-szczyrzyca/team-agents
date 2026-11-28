//! `agent status` reads the accounts from `agents.toml` and writes them after the agent of a task, in the listing and in
//! the detail of a round.

use std::collections::BTreeMap;

use agent_core::config::AbsolutePath;
use agent_core::ids::RoundId;
use agent_core::local_time::Zone;
use agent_core::protocol::{Frame, Reply, Response, RoundDetail, RoundTask, Target, TeamSummary};
use agent_core::state::State;
use time::UtcOffset;

use super::{Activity, render_round_with_accounts, status_active};
use crate::client::Connection;
use crate::fixtures::{at, task, team_id, team_state};
use crate::testing::{Answer, FakeDaemon};
use agent_runtime::store::Home;

/// The configuration: the member `claude-1` of the sample team runs with the account `claude-main`.
const CONFIG: &str = "\
[accounts.claude-main]
owner = \"claude\"

[agents.claude-1]
cli = \"claude\"
account = \"claude-main\"

[tiers]
normal = [\"claude-1\"]
";

/// The ID of the only round of the sample.
fn round_id() -> RoundId {
    "round-20261111-153002-q1w2".parse().unwrap()
}

/// The sample team with one open round.
fn state_with_round() -> State {
    let mut state = team_state();
    state
        .team_mut(&team_id(0))
        .unwrap()
        .open_round(round_id(), Some("1.2".parse().unwrap()), at(20))
        .unwrap();
    state
}

/// The detail of the round of [`state_with_round`], with the task of the member.
fn detail(state: &State) -> RoundDetail {
    let team = &state.teams()[0];
    let (_, member) = team.find(&task(2)).unwrap();
    let tasks = vec![RoundTask {
        task: task(2),
        role: agent_core::kinds::Role::Member,
        agent: member.agent().clone(),
        worktree: AbsolutePath::try_from("/srv/agents/worktrees/team-agents/task-20261111-153002-k3x9".to_owned())
            .unwrap(),
        branch: "agents/claude-1/task-20261111-153002-k3x9-1.2".parse().unwrap(),
        runs: Vec::new(),
    }];
    RoundDetail::new(team, team.round(&round_id()).unwrap(), tasks)
}

/// A home in a temporary directory with `config` as its `agents.toml`, and a daemon that answers `reply` to anything.
fn home_and_daemon(dir: &tempfile::TempDir, config: &str, reply: Reply) -> (Activity, FakeDaemon) {
    let home = Home::new(&AbsolutePath::try_from(dir.path().to_str().unwrap().to_owned()).unwrap());
    std::fs::write(home.config_file(), config).unwrap();
    let activity = Activity::new(home, Zone::Fixed(UtcOffset::UTC));
    let daemon = FakeDaemon::serve(&dir.path().join("agent.sock"), move |request| {
        Answer::Lines(vec![
            Frame::Response(Response::ok(request.id, reply.clone()))
                .to_line()
                .unwrap(),
        ])
    });
    (activity, daemon)
}

#[test]
fn a_round_detail_names_the_account_of_the_agent_of_a_task() {
    let accounts = BTreeMap::from([("claude-1".parse().unwrap(), "claude-main".parse().unwrap())]);

    let text = render_round_with_accounts(&detail(&state_with_round()), Zone::Fixed(UtcOffset::UTC), &accounts);

    assert!(
        text.contains("  task-20261111-153002-k3x9  member  claude-1 [claude-main]  /srv/agents/worktrees/"),
        "{text}"
    );
}

#[test]
fn a_round_detail_without_accounts_names_the_agent_alone() {
    let text = render_round_with_accounts(
        &detail(&state_with_round()),
        Zone::Fixed(UtcOffset::UTC),
        &BTreeMap::new(),
    );

    assert!(
        text.contains("  task-20261111-153002-k3x9  member  claude-1  /srv/agents/worktrees/"),
        "{text}"
    );
}

#[tokio::test]
async fn the_status_of_a_round_reads_the_accounts_of_the_configuration() {
    let dir = tempfile::tempdir().unwrap();
    let reply = Reply::Round {
        round: Box::new(detail(&state_with_round())),
    };
    let (activity, daemon) = home_and_daemon(&dir, CONFIG, reply);
    let mut connection = Connection::connect(daemon.socket()).await.unwrap();
    let mut out = Vec::new();

    status_active(&mut connection, Some(Target::Round(round_id())), &activity, &mut out)
        .await
        .unwrap();

    let text = String::from_utf8(out).unwrap();
    assert!(text.contains("member  claude-1 [claude-main]  /srv/agents/"), "{text}");
}

#[tokio::test]
async fn the_status_of_the_teams_reads_the_accounts_of_the_configuration() {
    let dir = tempfile::tempdir().unwrap();
    let teams = vec![TeamSummary::from(&team_state().teams()[0])];
    let (activity, daemon) = home_and_daemon(&dir, CONFIG, Reply::Teams { teams });
    let mut connection = Connection::connect(daemon.socket()).await.unwrap();
    let mut out = Vec::new();

    status_active(&mut connection, None, &activity, &mut out).await.unwrap();

    let text = String::from_utf8(out).unwrap();
    assert!(text.contains("member       claude-1 [claude-main]  queued"), "{text}");
    assert!(text.contains("worker       glm-1   "), "{text}");
}
