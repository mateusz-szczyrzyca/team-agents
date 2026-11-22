//! Tests for owner actions after a task's last step.

use std::num::NonZeroU32;

use agent_core::questions::QuestionsFile;
use agent_core::state::{Crew, ItemPhase, State, TaskListText, Team, TeamMembers};
use agent_core::status::TaskStatus;

use crate::daemon::owner::fixture::Daemon;
use crate::fixtures::{at, member, normal, repo, task, team_id};

fn nz(n: u32) -> NonZeroU32 {
    NonZeroU32::new(n).unwrap()
}

/// Restores a legacy question without exposing branchless construction in production.
fn with_legacy_publish_question(
    team: Team,
    item: agent_core::ids::ItemId,
    task: agent_core::ids::TaskId,
    number: NonZeroU32,
    at: agent_core::state::Timestamp,
) -> Team {
    let mut fixture = serde_json::to_value(team).unwrap();
    let progress = fixture["items"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|progress| progress["id"] == serde_json::to_value(item).unwrap())
        .unwrap();
    progress["publish_question"] = serde_json::json!({ "task": task, "number": number, "at": at });
    serde_json::from_value(fixture).unwrap()
}

#[tokio::test]
async fn after_answer_ignores_unrelated_member_and_wrong_question_number() {
    let item_id = "1.1".parse().unwrap();
    let team_i = team_id(1);
    let task_lead = task(1);
    let task_other = task(2);

    let mut team = Team::new(
        team_i.clone(),
        repo(),
        at(0),
        agent_core::state::MergeMode::Autopilot,
        TaskListText::new("Brief.\n\n1. Feature\n[ ] 1.1 Item\n".into()).unwrap(),
        Crew::team(TeamMembers {
            coordinator: member(1, normal("lead")),
            members: vec![member(2, normal("worker"))],
            workers: vec![],
        })
        .unwrap(),
    );

    team.open_round("round-20261111-153001-test".parse().unwrap(), Some(item_id), at(1))
        .unwrap();

    team.advance_item(
        item_id,
        ItemPhase::InProgress {
            assignee: "lead".parse().unwrap(),
            branch: "agents/lead/task-1-1.1".parse().unwrap(),
        },
    )
    .unwrap();

    team = with_legacy_publish_question(team, item_id, task_lead.clone(), nz(2), at(100));

    let mut state = State::new();
    state.insert_team(team).unwrap();

    let daemon = Daemon::new(&state, None);
    let (mut owner, _sinks) = daemon.bare_owner().await;

    let file = QuestionsFile::parse("# Questions\n\n## 1. Action?\n**Verdict:** b\n").unwrap();

    // 1. Calling after_answer for task_other (worker) must NOT clear item 1.1's question:
    let located_other = owner.locate(&task_other).unwrap();
    let status_other = owner
        .after_answer(&task_other, &located_other, nz(1), &file)
        .expect("after_answer for unrelated member must succeed");
    assert_eq!(
        status_other,
        TaskStatus::Queued,
        "unrelated member without last step question resumes as queued"
    );

    let item = owner.parts.state.team(&team_i).unwrap().item(item_id).unwrap();
    assert!(
        item.publish_question().is_some(),
        "unrelated member answer must not clear publish_question"
    );

    // 2. Calling after_answer for task_lead with WRONG number nz(1) must NOT clear item 1.1's question:
    let located_lead = owner.locate(&task_lead).unwrap();
    let status_wrong_num = owner
        .after_answer(&task_lead, &located_lead, nz(1), &file)
        .expect("after_answer for wrong question number must succeed");
    assert_eq!(
        status_wrong_num,
        TaskStatus::Queued,
        "wrong question number resumes as queued"
    );

    let item = owner.parts.state.team(&team_i).unwrap().item(item_id).unwrap();
    assert!(
        item.publish_question().is_some(),
        "wrong question number must not clear publish_question"
    );

    // 3. Calling after_answer for task_lead with matching question nz(2) and verdict 'b' stops the task:
    let status_matching = owner
        .after_answer(&task_lead, &located_lead, nz(2), &file)
        .expect("after_answer for matching publication question must succeed");
    assert_eq!(
        status_matching,
        TaskStatus::Stopped,
        "matching publication question with verdict 'b' stops task"
    );

    let item = owner.parts.state.team(&team_i).unwrap().item(item_id).unwrap();
    assert!(
        item.publish_question().is_none(),
        "matching publication question answer must clear publish_question"
    );
}

#[tokio::test]
async fn after_answer_clears_retry_timers_only_for_matching_branches() {
    let item_id = "1.1".parse().unwrap();
    let team_i = team_id(1);
    let task_lead = task(1);
    let branch_lead = "agents/lead/task-1-1.1".parse().unwrap();
    let branch_other = "agents/other/task-2".parse().unwrap();

    let mut team = Team::new(
        team_i.clone(),
        repo(),
        at(0),
        agent_core::state::MergeMode::Autopilot,
        TaskListText::new("Brief.\n\n1. Feature\n[ ] 1.1 Item\n".into()).unwrap(),
        Crew::solo(member(1, normal("lead"))).unwrap(),
    );

    team.open_round("round-20261111-153001-test".parse().unwrap(), Some(item_id), at(1))
        .unwrap();

    team.advance_item(
        item_id,
        ItemPhase::InProgress {
            assignee: "lead".parse().unwrap(),
            branch: branch_lead,
        },
    )
    .unwrap();

    team = with_legacy_publish_question(team, item_id, task_lead.clone(), nz(1), at(100));

    let mut state = State::new();
    state.insert_team(team).unwrap();

    let daemon = Daemon::new(&state, None);
    let (mut owner, _sinks) = daemon.bare_owner().await;

    // Populate retry timers for both this branch and an unrelated branch in the same team:
    owner
        .watch
        .retry_at
        .insert((team_i.clone(), "agents/lead/task-1-1.1".parse().unwrap()), at(200));
    owner.watch.retry_at.insert((team_i.clone(), branch_other), at(300));

    let located_lead = owner.locate(&task_lead).unwrap();
    let file = QuestionsFile::parse("# Questions\n\n## 1. Action?\n**Verdict:** a\n").unwrap();

    let status = owner
        .after_answer(&task_lead, &located_lead, nz(1), &file)
        .expect("after_answer for retry must succeed");
    assert_eq!(status, TaskStatus::Queued);

    // Matching branch retry timer was cleared:
    assert!(
        !owner
            .watch
            .retry_at
            .contains_key(&(team_i.clone(), "agents/lead/task-1-1.1".parse().unwrap())),
        "matching branch retry timer must be cleared"
    );

    // Unrelated branch retry timer in same team MUST BE PRESERVED:
    assert!(
        owner
            .watch
            .retry_at
            .contains_key(&(team_i.clone(), "agents/other/task-2".parse().unwrap())),
        "unrelated branch retry timer in same team must NOT be cleared"
    );
}
