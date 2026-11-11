//! A subscription is the summaries of the teams as they are (`Reply::Subscribed`) and the events that follow: a
//! client that applies the events to the summaries must end with the summaries of the state, whatever changed. The
//! events that only say what happened (`team_started`, `item_status`, ...) carry too little to update a summary
//! (which count goes down? what does a new team look like?), so `team_updated` carries the whole summary.

#[cfg(test)]
mod subscription {
    use agent_core::config::RepoName;
    use agent_core::ids::{AgentName, BranchName, ItemId, TaskId, TeamId};
    use agent_core::protocol::{EventKind, TeamSummary};
    use agent_core::state::{Crew, ItemPhase, Member, MergeMode, Participant, State, TaskListText, Team, Timestamp};
    use agent_core::status::{ItemStatus, TaskStatus};
    use serde_json::json;

    /// The state of the fixture: team `a1b2` (items `1.1` done, `1.2` in review, `2.1` pending) and team `c3d4`.
    const FIXTURE: &str = include_str!("fixtures/state-v1.json");

    /// The state and the summaries of its teams, as `subscribe` hands them over.
    fn subscribe() -> (State, Vec<TeamSummary>) {
        let state = State::from_json(FIXTURE).unwrap();
        let teams = state.teams().iter().map(TeamSummary::from).collect();
        (state, teams)
    }

    /// The summaries of the teams of `state`.
    fn summaries(state: &State) -> Vec<TeamSummary> {
        state.teams().iter().map(TeamSummary::from).collect()
    }

    /// The event that tells a client the new summary of team `id` of `state`.
    fn updated(state: &State, id: &TeamId) -> EventKind {
        EventKind::TeamUpdated {
            summary: Box::new(TeamSummary::from(state.team(id).unwrap())),
        }
    }

    /// The ID of the first team of the fixture.
    fn team_a() -> TeamId {
        "team-20261111-193000-a1b2".parse().unwrap()
    }

    #[test]
    fn an_item_that_starts_while_others_wait_or_are_in_review_changes_the_right_counts() {
        let (mut state, mut teams) = subscribe();
        let pending: ItemId = "2.1".parse().unwrap();
        state
            .team_mut(&team_a())
            .unwrap()
            .item_mut(pending)
            .unwrap()
            .advance(ItemPhase::InProgress {
                assignee: "codex-1".parse::<AgentName>().unwrap(),
                branch: "agents/codex-1/item-2-1".parse::<BranchName>().unwrap(),
            })
            .unwrap();

        updated(&state, &team_a()).apply_to(&mut teams);

        assert_eq!(teams, summaries(&state));
        let counts = teams[0].items;
        assert_eq!(
            (counts.pending, counts.in_progress, counts.in_review, counts.done),
            (0, 1, 1, 1)
        );
    }

    #[test]
    fn a_started_team_appears_in_the_summaries_in_its_place() {
        let (mut state, mut teams) = subscribe();
        let repo: RepoName = state.teams()[0].repo().clone();
        let member = Member::new(
            "task-20261111-200001-k3x9".parse::<TaskId>().unwrap(),
            Participant::Normal("codex-2".parse().unwrap()),
        );
        let team = Team::new(
            "team-20261111-200000-e5f6".parse().unwrap(),
            repo,
            "2026-11-11T20:00:00Z".parse::<Timestamp>().unwrap(),
            MergeMode::Autopilot,
            TaskListText::new("Do it.\n\n1. Work\n[ ] 1.1 Do it\n".to_owned()).unwrap(),
            Crew::solo(member).unwrap(),
        );
        let id = team.id().clone();
        state.insert_team(team).unwrap();

        updated(&state, &id).apply_to(&mut teams);

        assert_eq!(teams, summaries(&state));
        assert_eq!(teams.len(), 3);
    }

    #[test]
    fn a_change_of_a_task_replaces_that_teams_summary_only() {
        let (mut state, mut teams) = subscribe();
        let before_other = teams[1].clone();
        let task: TaskId = "task-20261111-193002-k3x9".parse().unwrap();
        let (_, mut member) = state.find_task_mut(&task).unwrap();
        member.transition(TaskStatus::Waiting).unwrap();

        updated(&state, &team_a()).apply_to(&mut teams);

        assert_eq!(teams, summaries(&state));
        assert_eq!(teams[1], before_other);
        assert!(
            teams[0]
                .members
                .iter()
                .any(|m| m.task == task && m.status == TaskStatus::Waiting)
        );
        assert_eq!(ItemStatus::InReview, state.team(&team_a()).unwrap().items()[1].status());
    }

    #[test]
    fn an_update_applied_twice_or_for_an_unchanged_team_changes_nothing() {
        let (state, mut teams) = subscribe();
        let before = teams.clone();

        updated(&state, &team_a()).apply_to(&mut teams);
        updated(&state, &team_a()).apply_to(&mut teams);

        assert_eq!(teams, before);
    }

    #[test]
    fn the_events_that_only_say_what_happened_leave_the_summaries_as_they_are() {
        let (_, mut teams) = subscribe();
        let before = teams.clone();
        let item: ItemId = "1.2".parse().unwrap();

        for event in [
            EventKind::TeamStarted { team: team_a() },
            EventKind::ItemStatus {
                team: team_a(),
                item,
                status: ItemStatus::Approved,
            },
        ] {
            event.apply_to(&mut teams);
        }

        assert_eq!(teams, before);
    }

    #[test]
    fn the_update_has_its_json_and_reads_back_equal_and_refuses_an_unknown_field() {
        let (state, _) = subscribe();
        let event = updated(&state, &team_a());

        let value = serde_json::to_value(&event).unwrap();

        assert_eq!(value["kind"], json!("team_updated"));
        assert_eq!(value["summary"]["id"], json!("team-20261111-193000-a1b2"));
        assert_eq!(serde_json::from_value::<EventKind>(value.clone()).unwrap(), event);
        let mut extra = value;
        extra["summary"]["extra"] = json!(1);
        assert!(serde_json::from_value::<EventKind>(extra).is_err());
    }
}
