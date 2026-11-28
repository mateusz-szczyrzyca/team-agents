//! `agent status` shows the readings of the limit windows that the state keeps, after the pauses of the accounts.

use agent_core::config::AbsolutePath;
use agent_core::limit_reading::{LimitReading, LimitWindow};
use agent_core::local_time::Zone;
use agent_core::protocol::{Frame, Reply, Response, TeamSummary};
use agent_core::state::{ReadingOwner, Timestamp};
use agent_runtime::clock::SystemClock;
use agent_runtime::store::Home;
use time::UtcOffset;

use super::{Activity, status_active};
use crate::client::Connection;
use crate::fixtures::team_state;
use crate::testing::{Answer, FakeDaemon};

/// The configuration of the sample: the member `claude-1` runs with the account `claude-main`.
const CONFIG: &str = "\
[accounts.claude-main]
owner = \"claude\"

[agents.claude-1]
cli = \"claude\"
account = \"claude-main\"

[tiers]
normal = [\"claude-1\"]
";

// Rule: the status of the teams has a line for the account whose reading the state keeps.
#[tokio::test]
async fn the_status_of_the_teams_shows_the_reading_the_state_keeps() {
    let dir = tempfile::tempdir().unwrap();
    let home = Home::new(&AbsolutePath::try_from(dir.path().to_str().unwrap().to_owned()).unwrap());
    std::fs::write(home.config_file(), CONFIG).unwrap();
    let mut state = team_state();
    let now = Timestamp::now(&SystemClock).unwrap();
    let reading = LimitReading::new(LimitWindow::FiveHour, 0.62, "2099-12-31T15:30:00Z".parse().unwrap()).unwrap();
    state.keep_limit_readings(
        &ReadingOwner::Account("claude-main".parse().unwrap()),
        &[reading],
        now,
        now,
    );
    std::fs::write(home.state_file(), state.to_json().unwrap()).unwrap();
    let activity = Activity::new(home, Zone::Fixed(UtcOffset::UTC));
    let teams = vec![TeamSummary::from(&state.teams()[0])];
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
        text.contains("claude-main: 5h 62% (resets 2099-12-31 15:30, read just now)\n"),
        "{text}"
    );
}
