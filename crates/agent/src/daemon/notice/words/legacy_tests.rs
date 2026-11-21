//! The notifications about a task of a state written before the `Short desc:` line carry a derived description.
use agent_core::ids::{TaskId, TeamId};
use agent_core::kinds::CliKind;
use agent_core::state::State;

use crate::daemon::notice::{NoticeContext, NoticeKind, layout_notification};
use crate::fixtures::at;

#[test]
fn a_notification_about_a_task_of_a_state_from_before_carries_a_derived_description_and_the_brief() {
    let state = State::from_json(include_str!(
        "../../../../../agent-core/tests/fixtures/state-before-short-desc.json"
    ))
    .unwrap();
    let team = &state.teams()[0];
    let task_id: TaskId = "task-20261111-193001-k3x9".parse().unwrap();
    let team_id: TeamId = team.id().clone();
    let context = NoticeContext {
        repo: "team-agents",
        agent: "claude-1",
        cli: Some(CliKind::Claude),
        model: None,
        time: at(0),
        offset: time::UtcOffset::UTC,
        task: &task_id,
        team: Some(&team_id),
        pr: None,
        words: crate::daemon::notice::sample::words_for(team.task_list().list(), None, &task_id),
    };

    let notification = layout_notification(NoticeKind::Failed, &context, "It failed").unwrap();

    let lines: Vec<&str> = notification.body().lines().collect();
    assert_eq!(lines.get(2).copied(), Some("Task: Original brief"), "{lines:?}");
    assert!(
        notification
            .body()
            .ends_with("TASK (task-20261111-193001-k3x9):\nBuild the thing."),
        "{}",
        notification.body()
    );
}
