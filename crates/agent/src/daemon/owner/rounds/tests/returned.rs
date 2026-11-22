//! A lead that sends the work of its member back with `RETURNED: <item>` is counted: the count and the lead's reasons
//! are kept with the item, `agent status` shows the count, and a restart of the daemon loses none of it. A lead that only
//! answers its member, or that writes no such line, is never counted.

use agent_core::ids::{ItemId, TeamId};
use agent_core::protocol::{Client, Command, Reply, StatusRequest, TeamSummary};
use agent_core::state::{ItemPhase, MergeMode, State};

use super::rig::{ONE, Rig, investigator, normal};
use crate::client::Connection;

/// A reviewer that approves.
const APPROVE: &str = "done:VERDICT: APPROVE";

/// A team of `lead`, whose runs do what `lead_scenario` says, and `claude-1`, which commits its work and says DONE in
/// every run, started with `[replacement]` `extra` lines; the daemon is running and the team is started.
async fn team_of(lead_scenario: &'static str, extra: &str) -> (Rig, TeamId) {
    let mut rig = Rig::new(
        &[
            normal("lead", lead_scenario),
            normal("claude-1", "work"),
            investigator("inv-r1", APPROVE),
            investigator("inv-r2", APPROVE),
        ],
        5,
        true,
    )
    .await;
    if !extra.is_empty() {
        let mut config = std::fs::read_to_string(rig.home.config_file()).unwrap();
        config.push_str("[replacement]\n");
        config.push_str(extra);
        std::fs::write(rig.home.config_file(), config).unwrap();
    }
    rig.start().await;
    let (team, _) = rig
        .run(&["lead", "claude-1"], ONE, MergeMode::Autopilot, Some("lead"))
        .await;
    (rig, team)
}

/// The item 1.1.
fn item() -> ItemId {
    "1.1".parse().unwrap()
}

/// How many returns of item 1.1 the state of `team` counts.
fn counted(state: &State, team: &TeamId) -> u32 {
    state
        .team(team)
        .unwrap()
        .item(item())
        .unwrap()
        .counted_rejections(5)
        .get()
}

/// Whether the state of `team` counts `returns` returns of item 1.1 and its member `claude-1` has finished the run they
/// started: the status line of the member is then settled.
fn settled(state: &State, team: &TeamId, returns: u32) -> bool {
    counted(state, team) == returns
        && state.team(team).unwrap().crew().roster().iter().any(|(_, member)| {
            member.agent().name() == "claude-1" && member.status() == agent_core::status::TaskStatus::Waiting
        })
}

/// The status of `team` as the daemon answers it, rendered as `agent status` prints it.
async fn status_text(rig: &Rig, team: &TeamId) -> String {
    let reply = Connection::connect(&rig.home.socket_file())
        .await
        .unwrap()
        .request(Client::Cli, Command::Status(StatusRequest { target: None }))
        .await
        .unwrap();
    let (Reply::Teams { teams } | Reply::Overview { teams, .. }) = reply else {
        panic!("the status answers with teams: {reply:?}");
    };
    let team: TeamSummary = teams.into_iter().find(|listed| &listed.id == team).unwrap();
    crate::commands::status::render(std::slice::from_ref(&team))
}

// Rule: the lead's `RETURNED:` line after the member's work is a rejection once the member runs again; the count and
// the reasons from the bus stay with the item, `agent status` shows `returned 2 of 5`, and a restart keeps them.
#[tokio::test]
async fn a_lead_that_returns_the_work_twice_is_counted_twice_with_its_reasons_and_status_shows_it() {
    let (mut rig, team) = team_of(
        "sleep,return:claude-1:fix the reader first,return:claude-1:add the tests too,sleep",
        "",
    )
    .await;

    let state = rig
        .state_where("two returns counted", |state| settled(state, &team, 2))
        .await;
    let text = status_text(&rig, &team).await;
    rig.stop().await.unwrap();
    rig.start().await;
    let restarted = rig.state();
    let after = status_text(&rig, &team).await;
    rig.stop().await.unwrap();

    let progress = state.team(&team).unwrap().item(item()).unwrap();
    assert!(matches!(progress.phase(), ItemPhase::InProgress { .. }));
    let reasons: Vec<&str> = progress
        .rejections()
        .unwrap()
        .iter()
        .map(agent_core::state::Rejection::reason)
        .collect();
    assert_eq!(reasons.len(), 2, "{reasons:?}");
    assert!(reasons[0].contains("fix the reader first"), "{reasons:?}");
    assert!(reasons[1].contains("add the tests too"), "{reasons:?}");
    assert!(text.contains("claude-1  waiting, returned 2 of 5"), "{text}");
    assert_eq!(counted(&restarted, &team), 2);
    assert!(after.contains("returned 2 of 5"), "{after}");
}

// Rule: `[replacement] rejections = 0` never replaces an agent, but the returns are still counted and shown, without a
// limit.
#[tokio::test]
async fn with_no_limit_the_returns_are_counted_and_status_shows_the_count_only() {
    let (mut rig, team) = team_of("sleep,return:claude-1:fix the reader first,sleep", "rejections = 0\n").await;

    rig.state_where("one return counted", |state| settled(state, &team, 1))
        .await;
    let text = status_text(&rig, &team).await;
    rig.stop().await.unwrap();

    assert!(text.contains("claude-1  waiting, returned 1\n"), "{text}");
}

// Rule: a lead that answers its member without the line sends no work back: nothing is counted.
#[tokio::test]
async fn a_lead_that_only_answers_its_member_is_not_counted() {
    let (mut rig, team) = team_of("sleep,say:claude-1:also update the readme,sleep", "").await;

    let state = rig
        .state_where("the member ran twice", |state| {
            let team = state.team(&team).unwrap();
            team.crew()
                .roster()
                .iter()
                .any(|(_, member)| member.agent().name() == "claude-1" && member.runs().len() >= 2)
        })
        .await;
    let text = status_text(&rig, &team).await;
    rig.stop().await.unwrap();

    let progress = state.team(&team).unwrap().item(item()).unwrap();
    assert_eq!(progress.counted_rejections(5).get(), 0);
    assert_eq!(progress.rejections(), None);
    assert!(!text.contains("returned"), "{text}");
}

/// The reasons the state of `team` keeps for item 1.1, oldest first.
fn reasons(state: &State, team: &TeamId) -> Vec<String> {
    state
        .team(team)
        .unwrap()
        .item(item())
        .unwrap()
        .rejections()
        .unwrap_or_default()
        .iter()
        .map(|rejection| rejection.reason().to_owned())
        .collect()
}

// Rule: the reason of a rejection is what the lead told the member after the notice of this hand-over, directly; a
// remark it made after an earlier hand-over is not the reason.
#[tokio::test]
async fn the_reason_is_what_the_lead_said_after_the_notice_of_this_hand_over() {
    let (mut rig, team) = team_of(
        "sleep,say:claude-1:an unrelated remark,return:claude-1:the real reason,sleep",
        "",
    )
    .await;

    let state = rig
        .state_where("one return counted", |state| settled(state, &team, 1))
        .await;
    rig.stop().await.unwrap();

    let reasons = reasons(&state, &team);
    assert_eq!(reasons.len(), 1, "{reasons:?}");
    assert!(reasons[0].contains("the real reason"), "{reasons:?}");
    assert!(!reasons[0].contains("unrelated"), "{reasons:?}");
}

// Rule: the reason is kept whole, however long: only what is shown is shortened.
#[tokio::test]
async fn a_long_reason_is_kept_whole() {
    let long = "y".repeat(1600);
    let scenario: &'static str = Box::leak(format!("sleep,return:claude-1:{long},sleep").into_boxed_str());
    let (mut rig, team) = team_of(scenario, "").await;

    let state = rig
        .state_where("one return counted", |state| settled(state, &team, 1))
        .await;
    rig.stop().await.unwrap();

    let reasons = reasons(&state, &team);
    assert_eq!(reasons.len(), 1, "{reasons:?}");
    assert!(
        reasons[0].contains(&long),
        "the reason has {} characters",
        reasons[0].len()
    );
}

// Rule: the state that says the lead's run ended already holds the return: whenever the lead's second run is saved as
// ended, the first return is saved with it, and so for the third (best effort: the two saves were one a moment apart).
#[tokio::test]
async fn a_return_is_saved_with_the_end_of_the_run_that_made_it() {
    let (mut rig, team) = team_of(
        "sleep,return:claude-1:fix the reader first,return:claude-1:add the tests too,sleep",
        "",
    )
    .await;

    let mut seen = 0;
    while counted(&rig.state(), &team) < 2 {
        let state = rig.state();
        let team_state = state.team(&team).unwrap();
        let lead_ended = team_state
            .crew()
            .roster()
            .iter()
            .filter(|(role, _)| *role == agent_core::kinds::Role::Coordinator)
            .map(|(_, lead)| lead.runs().iter().filter(|run| !run.is_running()).count())
            .sum::<usize>();
        let recorded = team_state
            .item(item())
            .unwrap()
            .rejections()
            .map_or(0, <[agent_core::state::Rejection]>::len);
        // Runs 1 to 3 of the lead: nothing to return, the first return, the second.
        assert!(
            recorded + 1 >= lead_ended.min(3),
            "{lead_ended} runs ended, {recorded} returns saved"
        );
        seen += 1;
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    }
    rig.stop().await.unwrap();

    assert!(seen > 0);
}
