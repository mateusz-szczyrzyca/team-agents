//! A verified delivery owns its wait even when no member can work.

use super::*;
use crate::daemon::owner::fixture::Daemon;
use crate::fixtures::{at, pr, task, team_id, team_state};

fn waiting_delivery() -> agent_core::state::State {
    let mut state = team_state();
    {
        let mut team = state.team_mut(&team_id(0)).unwrap();
        for (id, suffix) in [("1.1", "wait"), ("1.2", "ownr")] {
            team.open_round(
                format!("round-20261119-090000-{suffix}").parse().unwrap(),
                Some(id.parse().unwrap()),
                at(1),
            )
            .unwrap();
            team.advance_item(
                id.parse().unwrap(),
                ItemPhase::InProgress {
                    assignee: "claude-1".parse().unwrap(),
                    branch: "agents/claude-1/item".parse().unwrap(),
                },
            )
            .unwrap();
        }
        team.advance_item(
            "1.2".parse().unwrap(),
            ItemPhase::InReview {
                pr: pr(7),
                reviews: Vec::new(),
            },
        )
        .unwrap();
        team.deliver_round(&"round-20261119-090000-wait".parse().unwrap(), pr(7).number)
            .unwrap();
    }
    state
}

#[tokio::test]
async fn waiting_delivery_is_not_unfinished_work_for_the_asleep_watch() {
    let daemon = Daemon::new(&waiting_delivery(), None);
    let (owner, _sinks) = daemon.bare_owner().await;
    assert_eq!(
        owner.look_at_sleep(&team_id(0)).unwrap().unfinished,
        Vec::<String>::new()
    );
}

#[tokio::test]
async fn waiting_delivery_is_not_blocked_work_when_its_member_failed() {
    let mut state = waiting_delivery();
    {
        let mut team = state.team_mut(&team_id(0)).unwrap();
        let (_, mut member) = team.find_mut(&task(2)).unwrap();
        member.transition(TaskStatus::Running).unwrap();
        member.transition(TaskStatus::Failed).unwrap();
    }
    let daemon = Daemon::new(&state, None);
    let (owner, _sinks) = daemon.bare_owner().await;
    assert_eq!(
        owner.look_at_sleep(&team_id(0)).unwrap().standings,
        Vec::<Standing>::new()
    );
}

#[tokio::test]
async fn standing_item_line_formats_ended_run_time_in_the_daemon_zone_and_deduplicates_legacy_utc_notices() {
    use agent_core::local_time::Zone;
    use agent_core::state::{Participant, RunEnd};
    use agent_core::status::RunOutcome;

    let mut member = Member::new(task(2), Participant::Normal("claude-1".parse().unwrap()));
    member.begin_run(crate::fixtures::run(1), at(0), None).unwrap();
    member.transition(TaskStatus::Running).unwrap();
    member
        .end_run(
            at(10),
            RunEnd::Reported {
                outcome: RunOutcome::Failed,
            },
        )
        .unwrap();
    member.transition(TaskStatus::Failed).unwrap();
    let east = Zone::Fixed(time::UtcOffset::from_hms(1, 0, 0).expect("valid"));

    let rendered = line_in(Role::Member, &member, east);

    assert_eq!(
        rendered,
        format!(
            "- claude-1 (member, task {}): failed; its last run ended at 2026-11-11 20:15:10 with: STATUS: FAILED",
            task(2)
        )
    );
}

#[tokio::test]
async fn standing_item_saved_utc_notice_is_recognized_as_announced_by_non_utc_daemon() {
    use agent_core::local_time::Zone;
    use agent_core::state::{Participant, RunEnd};
    use agent_core::status::RunOutcome;

    let mut member = Member::new(task(2), Participant::Normal("claude-1".parse().unwrap()));
    member.begin_run(crate::fixtures::run(1), at(0), None).unwrap();
    member.transition(TaskStatus::Running).unwrap();
    member
        .end_run(
            at(10),
            RunEnd::Reported {
                outcome: RunOutcome::Failed,
            },
        )
        .unwrap();
    member.transition(TaskStatus::Failed).unwrap();
    let east = Zone::Fixed(time::UtcOffset::from_hms(1, 0, 0).expect("valid"));
    let standing = Standing {
        item: "1.1".to_owned(),
        member: "claude-1".to_owned(),
        task: task(2),
        line: format!(
            "{}; run {}",
            line_in(Role::Member, &member, east),
            crate::fixtures::run(1)
        ),
        legacy_line: format!(
            "{}; run {}",
            line_in(Role::Member, &member, Zone::Utc),
            crate::fixtures::run(1)
        ),
    };
    let dir = tempfile::tempdir().expect("tempdir");
    let saved = format!(
        "Team {} waits with item 1.1 behind failed member claude-1\n{}\n",
        team_id(0),
        standing.legacy_line
    );
    std::fs::write(dir.path().join("notice-1.md"), saved).expect("write saved notice");

    let announced = standing_was_announced(dir.path(), &team_id(0), &standing)
        .await
        .expect("inspect saved notices");

    assert!(announced);
}
