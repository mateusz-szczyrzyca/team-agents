use rand::rngs::Xoshiro256PlusPlus;
use rand::{RngExt, SeedableRng};

use super::*;
use crate::state::fixtures::round_id;

/// The IDs of the rounds numbered `numbers`.
fn ids(numbers: &[u32]) -> Vec<RoundId> {
    numbers.iter().map(|&n| round_id(n)).collect()
}

/// One case of the table: who starts a run, why, what its team's rounds look like, and the rounds the run gets.
struct Case {
    /// What the case shows, for the message of a failure.
    what: &'static str,
    /// The role of the task.
    role: Role,
    /// Why the run starts.
    start: Start,
    /// The round of the task's item, as a number.
    item: Option<u32>,
    /// The open rounds of the team, as numbers.
    open: &'static [u32],
    /// The rounds the run gets, as numbers.
    expected: &'static [u32],
}

/// [`RunStart`] with the rounds as numbers, to write the table.
enum Start {
    Work,
    Request(&'static [u32]),
    Review(Option<u32>),
    Continuation(&'static [u32]),
}

const CASES: &[Case] = &[
    Case {
        what: "a solo agent works for the round of its item",
        role: Role::Solo,
        start: Start::Work,
        item: Some(1),
        open: &[1],
        expected: &[1],
    },
    Case {
        what: "a solo agent without an item works for no round",
        role: Role::Solo,
        start: Start::Work,
        item: None,
        open: &[],
        expected: &[],
    },
    Case {
        what: "a member works for the round of its item and not for another open round",
        role: Role::Member,
        start: Start::Work,
        item: Some(2),
        open: &[1, 2],
        expected: &[2],
    },
    Case {
        what: "a member without an item works for no round, whatever is open",
        role: Role::Member,
        start: Start::Work,
        item: None,
        open: &[1, 2],
        expected: &[],
    },
    Case {
        what: "the coordinator works for every round of its team that is open",
        role: Role::Coordinator,
        start: Start::Work,
        item: None,
        open: &[3, 1, 2],
        expected: &[3, 1, 2],
    },
    Case {
        what: "the coordinator without an open round works for none",
        role: Role::Coordinator,
        start: Start::Work,
        item: None,
        open: &[],
        expected: &[],
    },
    Case {
        what: "a reviewer works for the round of the pull request it reviews",
        role: Role::Reviewer,
        start: Start::Review(Some(2)),
        item: None,
        open: &[1, 2],
        expected: &[2],
    },
    Case {
        what: "a reviewer asked again for the same pull request works for the same round",
        role: Role::Reviewer,
        start: Start::Review(Some(2)),
        item: None,
        open: &[2],
        expected: &[2],
    },
    Case {
        what: "a reviewer of a pull request no round has works for none",
        role: Role::Reviewer,
        start: Start::Review(None),
        item: None,
        open: &[1],
        expected: &[],
    },
    Case {
        what: "a worker asked by a run with rounds works for them",
        role: Role::Worker,
        start: Start::Request(&[4, 2]),
        item: None,
        open: &[1, 2, 4],
        expected: &[4, 2],
    },
    Case {
        what: "a worker asked by a run without rounds works for none",
        role: Role::Worker,
        start: Start::Request(&[]),
        item: None,
        open: &[1, 2],
        expected: &[],
    },
    Case {
        what: "a worker woken for no request works for none",
        role: Role::Worker,
        start: Start::Work,
        item: Some(1),
        open: &[1],
        expected: &[],
    },
    Case {
        what: "a reviewer started for no review works for none",
        role: Role::Reviewer,
        start: Start::Work,
        item: Some(1),
        open: &[1],
        expected: &[],
    },
    Case {
        what: "a fix pass of the coordinator stays in the rounds it worked for, not in all that are open",
        role: Role::Coordinator,
        start: Start::Continuation(&[1]),
        item: None,
        open: &[1, 2],
        expected: &[1],
    },
    Case {
        what: "a fix pass of a solo agent stays in its rounds",
        role: Role::Solo,
        start: Start::Continuation(&[1]),
        item: Some(2),
        open: &[2],
        expected: &[1],
    },
    Case {
        what: "the answers to the questions of a member stay in its rounds",
        role: Role::Member,
        start: Start::Continuation(&[2]),
        item: Some(2),
        open: &[2],
        expected: &[2],
    },
    Case {
        what: "a resume of a task that worked for no round works for none",
        role: Role::Member,
        start: Start::Continuation(&[]),
        item: Some(2),
        open: &[2],
        expected: &[],
    },
    Case {
        what: "a resume of a worker stays in the rounds of the run that asked",
        role: Role::Worker,
        start: Start::Continuation(&[3]),
        item: None,
        open: &[],
        expected: &[3],
    },
    Case {
        what: "a resume of a reviewer stays in its round",
        role: Role::Reviewer,
        start: Start::Continuation(&[2]),
        item: None,
        open: &[],
        expected: &[2],
    },
    Case {
        what: "a request is for workers: a member that gets one works for its item",
        role: Role::Member,
        start: Start::Request(&[9]),
        item: Some(2),
        open: &[2],
        expected: &[2],
    },
    Case {
        what: "a review is for reviewers: a coordinator that gets one works for the open rounds",
        role: Role::Coordinator,
        start: Start::Review(Some(9)),
        item: None,
        open: &[1],
        expected: &[1],
    },
];

#[test]
fn the_rounds_of_a_run_follow_its_role_and_why_it_starts() {
    for case in CASES {
        let (asked, previous, reviewed);
        let start = match &case.start {
            Start::Work => RunStart::Work,
            Start::Request(numbers) => {
                asked = ids(numbers);
                RunStart::Request { asked_by: &asked }
            }
            Start::Review(number) => {
                reviewed = number.map(round_id);
                RunStart::Review {
                    round: reviewed.as_ref(),
                }
            }
            Start::Continuation(numbers) => {
                previous = ids(numbers);
                RunStart::Continuation { previous: &previous }
            }
        };
        let item = case.item.map(round_id);
        let open = ids(case.open);

        let rounds = rounds_of_run(
            case.role,
            start,
            &TaskRounds {
                item: item.as_ref(),
                open: &open,
            },
        );

        assert_eq!(rounds, ids(case.expected), "{}", case.what);
    }
}

#[test]
fn every_role_is_in_the_table() {
    for role in [
        Role::Solo,
        Role::Coordinator,
        Role::Member,
        Role::Worker,
        Role::Reviewer,
    ] {
        assert!(CASES.iter().any(|case| case.role == role), "{role}");
    }
}

#[test]
fn a_continuation_is_the_previous_rounds_whatever_the_role_and_the_state_say_and_the_rest_is_drawn_from_the_state() {
    for seed in 0..200_u64 {
        let mut rng = Xoshiro256PlusPlus::seed_from_u64(seed);
        let pool: Vec<RoundId> = (1..=6).map(round_id).collect();
        let pick = |rng: &mut Xoshiro256PlusPlus| -> Vec<RoundId> {
            pool.iter().filter(|_| rng.random_range(0..2) == 0).cloned().collect()
        };
        let previous = pick(&mut rng);
        let open = pick(&mut rng);
        let item = (rng.random_range(0..2) == 0).then(|| pool[rng.random_range(0..pool.len())].clone());
        let facts = TaskRounds {
            item: item.as_ref(),
            open: &open,
        };
        for role in [
            Role::Solo,
            Role::Coordinator,
            Role::Member,
            Role::Worker,
            Role::Reviewer,
        ] {
            let gone_on = rounds_of_run(role, RunStart::Continuation { previous: &previous }, &facts);
            let fresh = rounds_of_run(role, RunStart::Work, &facts);

            assert_eq!(gone_on, previous, "seed {seed}, {role}");
            assert!(
                fresh
                    .iter()
                    .all(|round| open.contains(round) || item.as_ref() == Some(round)),
                "seed {seed}, {role}"
            );
            let distinct: std::collections::BTreeSet<&RoundId> = fresh.iter().collect();
            assert_eq!(distinct.len(), fresh.len(), "seed {seed}, {role}");
        }
    }
}
