//! `agent status` tells the account of an agent that has one in brackets after its name, and writes the lines of the
//! agents without an account as it always did.

use std::collections::BTreeMap;

use agent_core::config::AccountName;
use agent_core::ids::AgentName;
use agent_core::protocol::TeamSummary;
use time::UtcOffset;

use super::{Detail, Extras, Under, View, render_with_accounts};
use crate::fixtures::team_state;

/// The sample team: a coordinator, a member, a worker and a reviewer, all queued.
fn sample() -> Vec<TeamSummary> {
    vec![TeamSummary::from(&team_state().teams()[0])]
}

/// The status of `teams` in the full view with the `accounts` the agents run with.
fn shown(teams: &[TeamSummary], accounts: &BTreeMap<AgentName, AccountName>) -> String {
    render_with_accounts(
        teams,
        View::Full,
        Detail::Tasks,
        &|_| None,
        &|_| String::new(),
        Extras {
            under: Under {
                requests: &|_| Vec::new(),
                workers: &|_| Vec::new(),
            },
            accounts,
        },
        agent_core::local_time::Zone::Fixed(UtcOffset::UTC),
    )
}

#[test]
fn status_line_shows_account() {
    let accounts = BTreeMap::from([("claude-1".parse().unwrap(), "comp-claude".parse().unwrap())]);

    assert_eq!(
        shown(&sample(), &accounts),
        "\
team-20261111-153000-t5m8  team-agents  autopilot  2 pending, 0 in progress, 0 in review, 0 approved, 0 done
  task-20261111-153001-k3x9  coordinator  inv-opus                queued
  task-20261111-153002-k3x9  member       claude-1 [comp-claude]  queued
  task-20261111-153003-k3x9  worker       glm-1                   queued
  task-20261111-153004-k3x9  reviewer     inv-codex               queued
"
    );
}

#[test]
fn status_line_without_account_unchanged() {
    assert_eq!(
        shown(&sample(), &BTreeMap::new()),
        "\
team-20261111-153000-t5m8  team-agents  autopilot  2 pending, 0 in progress, 0 in review, 0 approved, 0 done
  task-20261111-153001-k3x9  coordinator  inv-opus   queued
  task-20261111-153002-k3x9  member       claude-1   queued
  task-20261111-153003-k3x9  worker       glm-1      queued
  task-20261111-153004-k3x9  reviewer     inv-codex  queued
"
    );
}

#[test]
fn a_queued_task_that_waits_for_its_agent_names_the_agent_without_the_account() {
    let accounts = BTreeMap::from([("claude-1".parse().unwrap(), "comp-claude".parse().unwrap())]);
    let mut teams = sample();
    teams[0].members[1].waits_for = Some(agent_core::protocol::RunElsewhere {
        team: "team-20261111-153000-t5m8".parse().unwrap(),
        task: "task-20261111-153001-k3x9".parse().unwrap(),
    });

    let text = shown(&teams, &accounts);

    assert!(
        text.contains("claude-1 [comp-claude]  queued, waits for claude-1, running in team-20261111-153000-t5m8"),
        "{text}"
    );
}
