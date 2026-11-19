//! The human's examples and each independent ordering/exclusion rule.

use super::*;
use crate::state::Timestamp;

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
