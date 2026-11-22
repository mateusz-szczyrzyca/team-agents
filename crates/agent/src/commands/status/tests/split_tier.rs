//! Short-view rendering of tier waits on split items and exhaustive task-status counting.

use agent_core::ids::RoundId;
use agent_core::kinds::Tier;
use agent_core::local_time::Zone;
use agent_core::protocol::TeamSummary;
use agent_core::status::TaskStatus;
use time::UtcOffset;

use super::{Detail, View, render_in};
use crate::fixtures::{at, team_id, team_state};

const HEADER: &str =
    "team-20261111-153000-t5m8  team-agents  autopilot  2 pending, 0 in progress, 0 in review, 0 approved, 0 done";

fn team_with_split_and_tier_wait() -> TeamSummary {
    let r1: RoundId = "round-20261111-153001-q1w2".parse().unwrap();
    let mut state = team_state();
    state
        .team_mut(&team_id(0))
        .unwrap()
        .open_round(r1, Some("1.1".parse().unwrap()), at(10))
        .unwrap();
    let base = TeamSummary::from(&state.teams()[0]);
    let mut split_wire = serde_json::to_value(base).unwrap();
    split_wire["splits"] = serde_json::json!([{
        "parent": "1.1",
        "approved_by": "harness",
        "parts": [
            {"title": "First", "delivers": "Guide", "checks": "Read", "depends_on": []},
            {"title": "Second", "delivers": "Bundle", "checks": "Read", "depends_on": ["a"]}
        ]
    }]);
    let mut team: TeamSummary = serde_json::from_value(split_wire).unwrap();
    team.tier_waits.insert("1.1".parse().unwrap(), Tier::Investigator);
    team
}

#[test]
fn short_view_prints_tier_wait_for_an_item_that_has_a_split() {
    let team = team_with_split_and_tier_wait();

    let rendered = render_in(
        &[team],
        View::Full,
        Detail::Short,
        &|_| None,
        &|_| String::new(),
        Zone::Fixed(UtcOffset::UTC),
    );

    assert_eq!(
        rendered,
        format!(
            "{HEADER}\n  round-20261111-153001-q1w2  item 1.1  open\n    1.1  waiting for investigator agent\n  tasks: 4 queued\n"
        )
    );
}

#[test]
fn every_task_status_variant_is_counted_and_their_sum_equals_the_members() {
    let mut team = TeamSummary::from(&team_state().teams()[0]);
    team.members.clear();
    let template = TeamSummary::from(&team_state().teams()[0]).members[0].clone();
    for status in TaskStatus::ALL {
        let mut member = template.clone();
        member.status = *status;
        team.members.push(member);
    }

    let rendered = render_in(
        std::slice::from_ref(&team),
        View::Full,
        Detail::Short,
        &|_| None,
        &|_| String::new(),
        Zone::Fixed(UtcOffset::UTC),
    );
    let counts_line = rendered
        .lines()
        .find_map(|line| line.strip_prefix("  tasks: "))
        .expect("tasks line is present");
    let total: usize = counts_line
        .split(", ")
        .map(|entry| {
            entry
                .split_whitespace()
                .next()
                .expect("count token")
                .parse::<usize>()
                .expect("numeric count")
        })
        .sum();

    assert_eq!(total, team.members.len());
}
