//! `agent status` as a person reads it while a round goes through its reviews: the lines of the reviewers say what they
//! did, from the record of the team's activities, as the daemon records them between the runs.

use std::time::{Duration, Instant};

use agent_core::ids::TeamId;
use agent_core::protocol::{Client, Command, Reply, StatusRequest, TeamSummary};
use agent_core::state::{ActivityOutcome, ActivityRequest, ItemPhase, MergeMode};
use agent_core::status::TaskStatus;
use time::UtcOffset;

use super::rig::{ONE, PATIENCE, Rig, investigator, normal};
use crate::client::Connection;
use crate::commands::status::{clock_of, render_active};

/// The answer of the daemon to a status request for every team, in short.
async fn listed(rig: &Rig) -> Vec<TeamSummary> {
    let mut connection = Connection::connect(&rig.home.socket_file()).await.unwrap();
    let reply = connection
        .request(Client::Cli, Command::Status(StatusRequest { target: None }))
        .await
        .unwrap();
    let (Reply::Teams { teams } | Reply::Overview { teams, .. }) = reply else {
        panic!("the status answers with teams: {reply:?}");
    };
    teams
}

/// The daemon's answer to a status request once the team `team` in it is as `holds` wants, asked again every few
/// milliseconds for at most [`PATIENCE`]: the lines are then read from the very answer the condition saw, not from a later
/// one in which the daemon has moved on.
async fn listed_when(rig: &Rig, team: &TeamId, what: &str, holds: impl Fn(&TeamSummary) -> bool) -> Vec<TeamSummary> {
    let deadline = Instant::now() + PATIENCE;
    loop {
        let teams = listed(rig).await;
        if teams.iter().any(|listed| &listed.id == team && holds(listed)) {
            return teams;
        }
        assert!(Instant::now() < deadline, "timed out waiting for {what}: {teams:?}");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

/// Whether `team` recorded exactly `count` reviews, and every one of them gave its report: the reviewers wait between
/// two passes.
fn reviews_reported(team: &TeamSummary, count: usize) -> bool {
    let reviews: Vec<_> = team
        .activities
        .iter()
        .filter(|activity| matches!(activity.request().kind(), agent_core::kinds::ActivityKind::Reviewer))
        .collect();
    reviews.len() == count
        && reviews.iter().all(|activity| {
            matches!(
                activity.outcome(),
                ActivityOutcome::Finished {
                    end: agent_core::state::ActivityEnd::Reported,
                    ..
                }
            )
        })
}

/// What `agent status` says of the team `team` of `teams`, as `--running` prints it.
fn printed(teams: &[TeamSummary], team: &TeamId) -> String {
    let shown: Vec<TeamSummary> = crate::commands::status::running(teams.to_vec())
        .into_iter()
        .filter(|shown| &shown.id == team)
        .collect();
    render_active(&shown, &|_| None)
}

/// The state of the activity of `task` in `text`: the words after the task, the kind and the investigator on its line.
fn state_of(text: &str, task: &str) -> String {
    let line = text
        .lines()
        .find(|line| line.trim_start().starts_with(&format!("{task}  reviewer")))
        .unwrap_or_else(|| panic!("{task} has a line: {text}"));
    line.split_whitespace().skip(3).collect::<Vec<_>>().join(" ")
}

/// When the record in `teams` says the newest review of `task` was reported, as `agent status` tells the time.
fn reported(teams: &[TeamSummary], task: &str) -> Option<String> {
    teams
        .iter()
        .flat_map(|team| &team.activities)
        .filter(|activity| activity.task().as_str() == task)
        .filter_map(|activity| match activity.outcome() {
            ActivityOutcome::Finished { at, .. } => Some((activity.asked_at(), *at)),
            ActivityOutcome::Working => None,
        })
        .max()
        .map(|(_, at)| clock_of(at, UtcOffset::UTC))
}

/// The tasks of the reviewers of `team` in `state`.
fn reviewers(state: &agent_core::state::State, team: &TeamId) -> Vec<String> {
    state
        .team(team)
        .unwrap()
        .reviewers()
        .iter()
        .map(|member| member.task().to_string())
        .collect()
}

// Rule: while a round goes through its first reviews, the line of each reviewer says what the record says it did —
// `done, 1st review finished <time>` — and no reviewer's line says `waiting`; the round then goes to its fix pass.
#[tokio::test]
async fn while_a_round_goes_through_its_reviews_the_reviewers_lines_say_what_they_did() {
    let mut rig = Rig::new(
        &[
            normal("claude-1", "work,fix-work-pushed"),
            investigator("inv-r1", "structured-commit:REQUEST_CHANGES"),
            investigator("inv-r2", "late-structured-commit:REQUEST_CHANGES"),
        ],
        5,
        false,
    )
    .await;
    rig.start().await;
    let (team, _) = rig.run(&["claude-1"], ONE, MergeMode::Autopilot, None).await;
    let state = rig
        .item_where("1.1", "a fix pass", |phase| matches!(phase, ItemPhase::Fixing { .. }))
        .await;
    let teams = listed_when(&rig, &team, "the two first reviews in", |listed| {
        reviews_reported(listed, 2)
    })
    .await;
    rig.stop().await.unwrap();

    let found = state.team(&team).unwrap();
    let text = printed(&teams, &team);
    let reviewers: Vec<_> = found
        .reviewers()
        .iter()
        .map(|member| (member.task().clone(), member.status()))
        .collect();
    assert_eq!(reviewers.len(), 2, "{text}");
    for (task, status) in reviewers {
        // The reviewer waits, with no run of its own: only the record tells what it did.
        assert_eq!(status, TaskStatus::Waiting, "{text}");
        let team_summary = teams.iter().find(|listed| listed.id == team).unwrap();
        let line = team_summary
            .activities
            .iter()
            .find(|activity| activity.task() == &task)
            .unwrap();
        let at = match line.outcome() {
            agent_core::state::ActivityOutcome::Finished { at, .. } => *at,
            agent_core::state::ActivityOutcome::Working => panic!("{task} reported: {text}"),
        };
        assert_eq!(
            state_of(&text, task.as_str()),
            format!("done, 1st review finished {}", clock_of(at, UtcOffset::UTC)),
            "{text}"
        );
    }
    assert!(!text.contains("  waiting"), "{text}");
}

/// What `agent status` says of the team `team` of `teams`, its closed rounds included: once the verification approves,
/// the daemon may merge and close the round before the answer is read.
fn printed_all(teams: &[TeamSummary], team: &TeamId) -> String {
    let shown: Vec<TeamSummary> = teams.iter().filter(|shown| &shown.id == team).cloned().collect();
    render_active(&shown, &|_| None)
}

// Rule: the record follows a round through its first reviews and the verification after the fix pass, and, as
// `agent status` prints the lines, each report of each reviewer says its ordinal in the round: `done, 1st review
// finished <t>` while the fix pass goes, `done, 2nd review finished <t>` once the verifier answered the findings.
#[tokio::test]
async fn after_the_fix_pass_the_verifiers_line_says_the_second_review_finished() {
    let mut rig = Rig::new(
        &[
            normal("claude-1", "work,fix-work-pushed"),
            investigator("inv-r1", "rotating-verify"),
            investigator("inv-r2", "rotating-verify"),
        ],
        5,
        false,
    )
    .await;
    rig.start().await;
    let (team, _) = rig.run(&["claude-1"], ONE, MergeMode::Autopilot, None).await;
    let first = rig
        .item_where("1.1", "a fix pass", |phase| matches!(phase, ItemPhase::Fixing { .. }))
        .await;
    let first_teams = listed_when(&rig, &team, "the two first reviews in", |listed| {
        reviews_reported(listed, 2)
    })
    .await;
    // With two investigators, the verification of the fix is the pair of the first pass again; it is in when both
    // reports of it are, and the daemon may ask for the next pass right after.
    let later_teams = listed_when(&rig, &team, "the two reports of the verification in", |listed| {
        reviews_reported(listed, 4)
    })
    .await;
    rig.stop().await.unwrap();

    let asked = reviewers(&first, &team);
    assert_eq!(asked.len(), 2, "{first_teams:?}");
    let first_text = printed(&first_teams, &team);
    for task in &asked {
        let time = reported(&first_teams, task).unwrap_or_else(|| panic!("{task} reported"));
        assert_eq!(
            state_of(&first_text, task),
            format!("done, 1st review finished {time}"),
            "{first_text}"
        );
    }
    let later_text = printed_all(&later_teams, &team);
    let summary = later_teams.iter().find(|listed| listed.id == team).unwrap();
    // The investigators of the first pass: a verifier among them gives its second review of the round.
    let first_reviewers: Vec<_> = summary
        .activities
        .iter()
        .filter(|activity| matches!(activity.request(), ActivityRequest::FirstReview))
        .map(|activity| activity.investigator().clone())
        .collect();
    let verifications: Vec<_> = summary
        .activities
        .iter()
        .filter(|activity| matches!(activity.request(), ActivityRequest::Verification { .. }))
        .collect();
    assert_eq!(verifications.len(), 2, "{later_text}");
    assert!(
        verifications
            .iter()
            .any(|activity| first_reviewers.contains(activity.investigator())),
        "the verification reuses an investigator of the first pass: {later_text}"
    );
    for activity in verifications {
        let task = activity.task().as_str();
        let time = reported(&later_teams, task).unwrap_or_else(|| panic!("{task} reported"));
        let ordinal = if first_reviewers.contains(activity.investigator()) {
            "2nd"
        } else {
            "1st"
        };
        assert_eq!(
            state_of(&later_text, task),
            format!("done, {ordinal} review finished {time}"),
            "{later_text}"
        );
    }
    assert!(!later_text.contains("  waiting"), "{later_text}");
}
