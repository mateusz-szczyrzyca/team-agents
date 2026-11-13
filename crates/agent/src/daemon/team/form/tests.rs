use std::str::FromStr;

use agent_core::config::AbsolutePath;
use agent_core::ids::AgentName;
use agent_core::kinds::Role;
use agent_core::protocol::{AgentList, FailureKind};
use agent_core::state::{ItemPhase, MergeMode, Participant, TaskListText};
use agent_runtime::agentbus::{BusTier, MemberState};
use rand::SeedableRng;
use rand::rngs::Xoshiro256PlusPlus;

use super::*;
use crate::daemon::team::support::{ALL_DONE, CHAIN, PARALLEL, PLAIN, config};
use crate::testing::ManualClock;

fn home() -> Home {
    Home::new(&AbsolutePath::from_str("/srv/agents").unwrap())
}

fn clock() -> ManualClock {
    ManualClock::new(crate::fixtures::at(0))
}

fn request(agents: &[&str], list: &str) -> RunRequest {
    RunRequest {
        repo: "example".parse().unwrap(),
        checkout: AbsolutePath::from_str("/work/example").unwrap(),
        agents: AgentList::try_from(
            agents
                .iter()
                .map(|name| name.parse::<AgentName>().unwrap())
                .collect::<Vec<_>>(),
        )
        .unwrap(),
        task_list: TaskListText::new(list.to_owned()).unwrap(),
        coordinator: None,
        merge: MergeMode::Autopilot,
    }
}

fn formed(request: &RunRequest) -> Result<Formed, FormError> {
    let mut rng = Xoshiro256PlusPlus::seed_from_u64(11);
    form(request, &config(), &home(), &clock(), &mut rng)
}

fn roster(formed: &Formed) -> Vec<(Role, String)> {
    formed
        .team
        .crew()
        .roster()
        .into_iter()
        .map(|(role, member)| (role, member.agent().name().to_owned()))
        .collect()
}

// ---- a task alone ------------------------------------------------------------------------------------------------

#[test]
fn one_agent_is_a_solo_task_with_no_team_on_the_bus() {
    let formed = formed(&request(&["claude-1"], PLAIN)).unwrap();

    assert_eq!(roster(&formed), [(Role::Solo, "claude-1".to_owned())]);
    assert_eq!(formed.bus, None);
    assert_eq!(formed.workspaces.len(), 1);
    assert_eq!(formed.team.repo().as_str(), "example");
    assert_eq!(formed.team.merge(), MergeMode::Autopilot);
    assert_eq!(formed.team.created_at(), crate::fixtures::at(0));
    assert_eq!(formed.base.as_str(), "main");
}

#[test]
fn a_solo_task_without_items_works_on_a_branch_of_its_task() {
    let formed = formed(&request(&["claude-1"], PLAIN)).unwrap();

    let workspace = &formed.workspaces[0];
    let task = formed.team.crew().roster()[0].1.task().clone();
    assert_eq!(workspace.task, task);
    assert_eq!(workspace.agent.as_str(), "claude-1");
    assert_eq!(workspace.branch.as_str(), format!("agents/claude-1/{task}"));
    assert_eq!(workspace.path.as_path(), home().worktree_dir(formed.team.repo(), &task));
}

#[test]
fn a_solo_task_with_items_starts_the_first_item_on_its_own_branch() {
    let formed = formed(&request(&["claude-1"], CHAIN)).unwrap();

    let task = formed.team.crew().roster()[0].1.task().clone();
    let branch = format!("agents/claude-1/{task}-1.1");
    assert_eq!(formed.workspaces[0].branch.as_str(), branch);
    let phase = formed.team.items()[0].phase();
    assert!(
        matches!(phase, ItemPhase::InProgress { assignee, branch: b } if assignee.as_str() == "claude-1" && b.as_str() == branch),
        "{phase:?}"
    );
    assert!(matches!(formed.team.items()[1].phase(), ItemPhase::Pending));
}

#[test]
fn an_investigator_alone_is_a_solo_task() {
    let formed = formed(&request(&["inv-opus"], PLAIN)).unwrap();

    assert!(matches!(
        formed.team.crew().roster()[0].1.agent(),
        Participant::Investigator(_)
    ));
}

// ---- a team ------------------------------------------------------------------------------------------------------

#[test]
fn a_team_has_its_coordinator_first_then_its_members_and_its_workers_in_the_configured_order() {
    let mut asked = request(&["claude-1", "glm-2", "codex-1", "glm-1"], PARALLEL);
    asked.coordinator = Some("claude-1".parse().unwrap());

    let formed = formed(&asked).unwrap();

    assert_eq!(
        roster(&formed),
        [
            (Role::Coordinator, "claude-1".to_owned()),
            (Role::Member, "codex-1".to_owned()),
            (Role::Worker, "glm-1".to_owned()),
            (Role::Worker, "glm-2".to_owned()),
        ]
    );
}

#[test]
fn the_coordinator_asked_for_wins_over_the_one_the_list_names() {
    let list = format!("Coordinator: codex-1\n{PARALLEL}");
    let mut asked = request(&["claude-1", "codex-1"], &list);
    assert_eq!(roster(&formed(&asked).unwrap())[0].1, "codex-1", "the list's line");

    asked.coordinator = Some("claude-1".parse().unwrap());
    assert_eq!(roster(&formed(&asked).unwrap())[0].1, "claude-1", "the flag");
}

#[test]
fn an_investigator_in_the_team_coordinates() {
    let formed = formed(&request(&["claude-1", "inv-opus"], PARALLEL)).unwrap();

    assert_eq!(roster(&formed)[0], (Role::Coordinator, "inv-opus".to_owned()));
}

#[test]
fn the_members_get_the_first_items_each_on_a_branch_and_the_others_a_task_branch() {
    let formed = formed(&request(&["inv-opus", "claude-1", "claude-2", "codex-1"], PARALLEL)).unwrap();

    let branches: Vec<_> = formed
        .workspaces
        .iter()
        .map(|w| (w.agent.to_string(), w.branch.to_string()))
        .collect();
    let tasks: Vec<_> = formed
        .team
        .crew()
        .roster()
        .iter()
        .map(|(_, m)| m.task().clone())
        .collect();
    assert_eq!(
        branches,
        [
            ("inv-opus".to_owned(), format!("agents/inv-opus/{}", tasks[0])),
            ("claude-1".to_owned(), format!("agents/claude-1/{}-1.1", tasks[1])),
            ("claude-2".to_owned(), format!("agents/claude-2/{}-2.1", tasks[2])),
            ("codex-1".to_owned(), format!("agents/codex-1/{}", tasks[3])),
        ]
    );
    let in_progress = formed
        .team
        .items()
        .iter()
        .filter(|item| matches!(item.phase(), ItemPhase::InProgress { .. }))
        .count();
    assert_eq!(in_progress, 2);
}

#[test]
fn every_task_has_its_own_worktree_in_the_home_and_a_distinct_id() {
    let formed = formed(&request(&["inv-opus", "claude-1", "glm-1"], CHAIN)).unwrap();

    let tasks: Vec<_> = formed
        .team
        .crew()
        .roster()
        .iter()
        .map(|(_, m)| m.task().clone())
        .collect();
    assert_eq!(tasks.iter().collect::<std::collections::BTreeSet<_>>().len(), 3);
    for (workspace, task) in formed.workspaces.iter().zip(&tasks) {
        assert_eq!(&workspace.task, task);
        assert_eq!(workspace.path.as_path(), home().worktree_dir(formed.team.repo(), task));
    }
}

#[test]
fn a_team_is_created_on_the_bus_with_its_lead_its_members_their_tiers_and_states() {
    let formed = formed(&request(&["claude-1", "claude-2", "glm-1"], PARALLEL)).unwrap();

    let bus = formed.bus.unwrap();
    assert_eq!(&bus.id, formed.team.id());
    assert_eq!(bus.repo.as_str(), "example");
    let lead = bus.lead.unwrap();
    let members: Vec<_> = bus
        .members
        .iter()
        .map(|(name, m)| (name.to_string(), m.tier, m.state, m.provider.clone()))
        .collect();
    let coordinator = lead.to_string();
    let other = if coordinator == "claude-1" {
        "claude-2"
    } else {
        "claude-1"
    };
    let mut expected = vec![
        (
            coordinator,
            BusTier::Normal,
            MemberState::Active,
            Some("claude".to_owned()),
        ),
        (
            other.to_owned(),
            BusTier::Normal,
            MemberState::Active,
            Some("claude".to_owned()),
        ),
        (
            "glm-1".to_owned(),
            BusTier::Worker,
            MemberState::Active,
            Some("opencode".to_owned()),
        ),
    ];
    expected.sort();
    let mut found = members;
    found.sort();
    assert_eq!(found, expected);
}

#[test]
fn every_member_is_active_on_the_bus_at_the_start_because_every_task_starts_at_once() {
    let formed = formed(&request(&["inv-opus", "claude-1", "claude-2", "glm-1"], CHAIN)).unwrap();

    let bus = formed.bus.unwrap();
    assert!(bus.members.values().all(|member| member.state == MemberState::Active));
}

#[test]
fn the_bus_members_have_the_worktree_and_branch_of_their_workspace() {
    let formed = formed(&request(&["inv-opus", "claude-1"], CHAIN)).unwrap();

    let bus = formed.bus.unwrap();
    for workspace in &formed.workspaces {
        let member = &bus.members[&workspace.agent];
        assert_eq!(member.task, workspace.task);
        assert_eq!(member.branch, workspace.branch);
        assert_eq!(member.worktree, workspace.path);
    }
}

// ---- refusals ----------------------------------------------------------------------------------------------------

fn refusal(request: &RunRequest) -> (FailureKind, FormError) {
    let error = formed(request).unwrap_err();
    (error.failure().kind, error)
}

#[test]
fn a_repository_that_is_not_configured_is_invalid() {
    let mut asked = request(&["claude-1"], PLAIN);
    asked.repo = "elsewhere".parse().unwrap();

    let (kind, error) = refusal(&asked);

    assert_eq!(kind, FailureKind::Invalid);
    assert!(matches!(error, FormError::UnknownRepo { repo } if repo == "elsewhere"));
}

#[test]
fn an_agent_that_is_not_configured_is_invalid() {
    let (kind, error) = refusal(&request(&["claude-1", "nobody"], PLAIN));

    assert_eq!(kind, FailureKind::Invalid);
    assert!(
        matches!(error, FormError::Plan(TeamPlanError::UnknownAgent { .. })),
        "{error:?}"
    );
}

#[test]
fn a_worker_as_coordinator_and_workers_alone_are_invalid() {
    let mut asked = request(&["claude-1", "glm-1"], PLAIN);
    asked.coordinator = Some("glm-1".parse().unwrap());
    let (kind, error) = refusal(&asked);
    assert_eq!(kind, FailureKind::Invalid);
    assert!(
        matches!(error, FormError::Plan(TeamPlanError::WorkerCoordinator { .. })),
        "{error:?}"
    );

    let (kind, error) = refusal(&request(&["glm-1", "glm-2"], PLAIN));
    assert_eq!(kind, FailureKind::Invalid);
    assert!(
        matches!(error, FormError::Plan(TeamPlanError::OnlyWorkers { .. })),
        "{error:?}"
    );
}

#[test]
fn too_many_investigators_in_the_team_is_refused() {
    let (kind, error) = refusal(&request(&["inv-opus", "inv-codex", "claude-1"], PLAIN));

    assert_eq!(kind, FailureKind::Refused);
    assert!(
        matches!(error, FormError::Plan(TeamPlanError::TooManyInvestigators { .. })),
        "{error:?}"
    );
}

#[test]
fn a_list_whose_items_are_all_done_is_invalid() {
    let (kind, error) = refusal(&request(&["claude-1"], ALL_DONE));

    assert_eq!(kind, FailureKind::Invalid);
    assert!(matches!(error, FormError::NothingToDo));
}

#[test]
fn a_refusal_names_what_is_wrong() {
    let (_, error) = refusal(&request(&["inv-opus", "inv-codex", "claude-1"], PLAIN));

    assert!(
        error.failure().message.contains("too many investigators in the team"),
        "{}",
        error.failure().message
    );
}

/// A generator that gives the same number every time: every ID it makes in one second is the same.
struct Constant;

impl rand::TryRng for Constant {
    type Error = std::convert::Infallible;

    fn try_next_u32(&mut self) -> Result<u32, Self::Error> {
        Ok(7)
    }

    fn try_next_u64(&mut self) -> Result<u64, Self::Error> {
        Ok(7)
    }

    fn try_fill_bytes(&mut self, dst: &mut [u8]) -> Result<(), Self::Error> {
        dst.fill(7);
        Ok(())
    }
}

#[test]
fn a_team_whose_tasks_cannot_get_ids_of_their_own_is_not_formed() {
    let found = form(
        &request(&["inv-opus", "claude-1"], PLAIN),
        &config(),
        &home(),
        &clock(),
        &mut Constant,
    );

    assert!(matches!(found, Err(FormError::IdCollision)), "{found:?}");
}
