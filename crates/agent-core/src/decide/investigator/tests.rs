//! The human's examples and each independent ordering/exclusion rule.

use super::*;
use crate::state::Timestamp;
use rand::SeedableRng;

fn name(raw: &str) -> InvestigatorName {
    raw.parse().unwrap()
}
fn team(n: u32) -> TeamId {
    format!("team-20261119-120000-{n:04}").parse().unwrap()
}
fn at(n: u32) -> Timestamp {
    format!("2026-11-19T12:00:{n:02}Z").parse().unwrap()
}

fn select(
    records: &[(&str, u32, ActivityKind, u32)],
    available: &[&str],
    excluded: &[&str],
    count: usize,
) -> Option<Vec<String>> {
    let mut rng = rand::rngs::StdRng::seed_from_u64(0);
    select_with_rng(records, available, excluded, count, &mut rng)
}

fn select_with_rng(
    records: &[(&str, u32, ActivityKind, u32)],
    available: &[&str],
    excluded: &[&str],
    count: usize,
    rng: &mut impl rand::Rng,
) -> Option<Vec<String>> {
    let names: Vec<_> = records.iter().map(|(n, _, _, _)| name(n)).collect();
    let teams: Vec<_> = records.iter().map(|(_, t, _, _)| team(*t)).collect();
    let activities: Vec<_> = records
        .iter()
        .zip(names.iter().zip(&teams))
        .map(|((_, _, kind, time), (name, team))| Activity {
            investigator: name,
            team,
            kind: *kind,
            asked_at: at(*time),
        })
        .collect();
    choose(
        &["a", "b", "c", "d"].map(name).into(),
        &available.iter().map(|n| name(n)).collect(),
        &activities,
        &team(1),
        &excluded.iter().map(|n| name(n)).collect(),
        NonZeroUsize::new(count).unwrap(),
        rng,
    )
    .map(|names| names.into_iter().map(|n| n.to_string()).collect())
}

#[test]
fn human_examples_and_global_tie_break() {
    use ActivityKind::{Investigator, Reviewer, TaskAllocator, TeamMember};
    let cases = [
        (vec![("a", 1, TeamMember, 20), ("b", 1, Reviewer, 10)], vec!["c"]),
        (vec![("c", 1, Investigator, 20)], vec!["a"]),
        (
            vec![
                ("a", 1, TeamMember, 40),
                ("b", 1, Reviewer, 30),
                ("c", 1, Investigator, 20),
                ("d", 1, TaskAllocator, 10),
            ],
            vec!["d"],
        ),
        (
            vec![
                ("a", 1, TeamMember, 20),
                ("b", 1, Reviewer, 10),
                ("c", 2, Investigator, 30),
                ("d", 2, TaskAllocator, 15),
            ],
            vec!["d"],
        ),
    ];
    for (records, expected) in cases {
        assert_eq!(
            select(&records, &["a", "b", "c", "d"], &[], 1),
            Some(expected.into_iter().map(str::to_owned).collect())
        );
    }
}

#[test]
fn rules_are_independent_of_input_order_or_activity_kind() {
    use ActivityKind::{Investigator, Reviewer, TaskAllocator, TeamMember};
    let cases = [
        (vec![], vec!["d", "b", "c", "a"], vec![], 2, Some(vec!["a", "b"])),
        (vec![], vec!["unknown", "b"], vec![], 2, None),
        (vec![("a", 1, TeamMember, 1)], vec!["a"], vec![], 1, None),
        (vec![], vec!["a", "b"], vec!["a", "b"], 1, None),
        (
            vec![("a", 1, Reviewer, 1), ("b", 1, Investigator, 2)],
            vec!["a", "b", "c"],
            vec![],
            1,
            Some(vec!["c"]),
        ),
        (
            vec![
                ("a", 1, Reviewer, 1),
                ("b", 1, Investigator, 2),
                ("c", 1, TaskAllocator, 3),
            ],
            vec!["a", "b", "c"],
            vec![],
            2,
            Some(vec!["a", "b"]),
        ),
        (
            vec![("a", 1, Reviewer, 1), ("b", 1, Reviewer, 1), ("a", 2, Investigator, 4)],
            vec!["a", "b", "c"],
            vec![],
            2,
            Some(vec!["c", "b"]),
        ),
        (
            vec![
                ("a", 1, Reviewer, 3),
                ("b", 1, Reviewer, 3),
                ("c", 1, Investigator, 2),
                ("d", 1, TaskAllocator, 1),
            ],
            vec!["a", "b", "c", "d"],
            vec![],
            2,
            Some(vec!["d", "c"]),
        ),
        (
            vec![
                ("a", 1, Reviewer, 3),
                ("b", 1, Reviewer, 3),
                ("c", 1, Investigator, 2),
                ("d", 1, TaskAllocator, 1),
            ],
            vec!["a", "b", "c", "d"],
            vec![],
            4,
            Some(vec!["d", "c", "a", "b"]),
        ),
        (
            vec![("a", 1, Reviewer, 3), ("b", 1, Reviewer, 3)],
            vec!["a", "b"],
            vec!["a"],
            2,
            None,
        ),
    ];
    for (records, free, excluded, count, expected) in cases {
        let expected = expected.map(|v| v.into_iter().map(str::to_owned).collect());
        assert_eq!(select(&records, &free, &excluded, count), expected, "{records:?}");
        let reversed: Vec<_> = records.iter().copied().rev().collect();
        assert_eq!(select(&reversed, &free, &excluded, count), expected);
    }
}

#[test]
fn the_pair_of_the_latest_request_comes_back_only_after_the_others_and_in_order() {
    use ActivityKind::Reviewer;
    // `a` and `b` were asked together last; `c` worked for the team earliest but latest anywhere.
    let records = vec![
        ("a", 1, Reviewer, 30),
        ("b", 1, Reviewer, 30),
        ("c", 1, Reviewer, 10),
        ("c", 2, Reviewer, 50),
        ("d", 1, Reviewer, 20),
    ];
    let all = ["a", "b", "c", "d"];
    let order = |count| select(&records, &all, &[], count);

    assert_eq!(order(1), Some(vec!["c".to_owned()]));
    assert_eq!(order(2), Some(vec!["c".to_owned(), "d".to_owned()]));
    // Rule 5: too few others are left, so the latest pair is relaxed, after them.
    assert_eq!(order(3), Some(vec!["c".to_owned(), "d".to_owned(), "a".to_owned()]));
    assert_eq!(
        order(4),
        Some(vec!["c".to_owned(), "d".to_owned(), "a".to_owned(), "b".to_owned()])
    );
    // When the others are busy, the latest pair is still chosen rather than leaving the request unfilled.
    assert_eq!(
        select(&records, &["a", "b"], &[], 2),
        Some(vec!["a".to_owned(), "b".to_owned()])
    );
    // ...but never beyond the permanent exclusions.
    assert_eq!(select(&records, &["a", "b"], &["a"], 2), None);
}

#[test]
fn two_investigators_tied_on_oldest_activity_are_both_chosen_over_seeds() {
    use std::collections::BTreeSet;
    let records = vec![("a", 1, ActivityKind::Reviewer, 10), ("b", 1, ActivityKind::Reviewer, 10)];
    let mut chosen_names = BTreeSet::new();
    for seed in 1..=32 {
        let mut rng = rand::rngs::StdRng::seed_from_u64(seed);
        let chosen = select_with_rng(&records, &["a", "b"], &[], 1, &mut rng).unwrap();
        assert_eq!(chosen.len(), 1);
        chosen_names.insert(chosen[0].clone());
    }
    assert!(chosen_names.contains("a"), "seed 1..=32 should choose a");
    assert!(chosen_names.contains("b"), "seed 1..=32 should choose b");
}

#[test]
fn the_least_recently_used_investigator_is_chosen_whatever_the_seed() {
    let records = vec![("a", 1, ActivityKind::Reviewer, 10), ("b", 1, ActivityKind::Reviewer, 20)];
    for seed in 1..=32 {
        let mut rng = rand::rngs::StdRng::seed_from_u64(seed);
        let chosen = select_with_rng(&records, &["a", "b"], &[], 1, &mut rng).unwrap();
        assert_eq!(chosen, vec!["a"]);
    }
}

#[test]
fn an_investigator_never_used_is_chosen_before_one_used() {
    let records = vec![("b", 1, ActivityKind::Reviewer, 10)];
    for seed in 1..=32 {
        let mut rng = rand::rngs::StdRng::seed_from_u64(seed);
        let chosen = select_with_rng(&records, &["a", "b"], &[], 1, &mut rng).unwrap();
        assert_eq!(chosen, vec!["a"]);
    }
}

#[test]
fn an_investigator_with_a_paused_task_is_not_free_for_a_request() {
    use crate::config::Config;
    use crate::state::{Crew, Member, MergeMode, Participant, State, TaskListText, Team};
    use crate::status::TaskStatus;

    let config = Config::from_toml_str(
        r#"
[investigators.inv-a]
cli = "claude"

[investigators.inv-b]
cli = "claude"
"#,
    )
    .unwrap();

    let team_id = team(1);
    let lead = Member::new(
        "task-20261119-120000-0001".parse().unwrap(),
        Participant::Normal("claude-1".parse().unwrap()),
    );
    let team = Team::new(
        team_id.clone(),
        "team-agents".parse().unwrap(),
        at(1),
        MergeMode::Autopilot,
        TaskListText::new("Brief.\n\n1. Feature\n[ ] 1.1 Item\n".into()).unwrap(),
        Crew::solo(lead).unwrap(),
    );
    let mut state = State::new();
    state.insert_team(team).unwrap();

    let mut paused_member = Member::new(
        "task-20261119-120000-0002".parse().unwrap(),
        Participant::Investigator("inv-a".parse().unwrap()),
    );
    paused_member.transition(TaskStatus::Paused).unwrap();
    state.add_reviewer(&team_id, paused_member).unwrap();

    let free = free_for_request(&state, &config, at(10));
    assert!(!free.contains(&"inv-a".parse().unwrap()));
    assert!(free.contains(&"inv-b".parse().unwrap()));
}

