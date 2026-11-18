//! Regression contracts for collected item and worktree facts.
use super::*;
use agent_core::state::{Crew, MergeMode, Participant, TaskListText};

fn team() -> Team {
    Team::new(
        "team-20261117-120000-a1b2".parse().unwrap(),
        "example".parse().unwrap(),
        "2026-11-17T12:00:00Z".parse().unwrap(),
        MergeMode::Autopilot,
        TaskListText::new("Build.\n\n1. Feature\n[ ] 1.1 Diagnose café\n".to_owned()).unwrap(),
        Crew::solo(Member::new(
            "task-20261117-120000-a1b2".parse().unwrap(),
            Participant::Normal("codex-1".parse().unwrap()),
        ))
        .unwrap(),
    )
}

#[test]
fn item_summary_uses_the_task_list_title() {
    let team = team();
    let collection = Collection::new(&team, "2026-11-17T12:00:00Z".parse().unwrap(), Duration::from_secs(120)).unwrap();
    assert_eq!(collection.summary.team.items[0].title, "Diagnose café");
}
