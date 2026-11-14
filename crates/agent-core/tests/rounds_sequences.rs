//! Any sequence of the operations of the state — opening and closing rounds, moving items on, starting and ending
//! runs that name rounds — leaves a state that saves and loads to the same text, and an operation that is refused
//! leaves the state as it was. The sequences are drawn from a seeded source: a failure names its seed.

#[cfg(test)]
#[path = "support/world.rs"]
mod world;

#[cfg(test)]
mod rounds_sequences {
    use agent_core::state::{ItemPhase, NewRun, ReviewOutcome, RoundEnd, RunEnd, State};
    use agent_core::status::{ReviewVerdict, TaskStatus};
    use rand::rngs::Xoshiro256PlusPlus;
    use rand::{Rng, RngExt, SeedableRng};

    use super::world::{at, item, pr, review, round, run, state, task, team_id};

    /// How many sequences are drawn.
    const SEEDS: u64 = 300;
    /// How many operations a sequence has.
    const STEPS: u32 = 60;
    /// The items of the three teams of the world, by team number.
    const ITEMS: [&[&str]; 3] = [&["1.1", "1.2", "1.3", "2.1"], &["1.1"], &[]];
    /// The tasks of the three teams, by team number: the crew, then the reviewers of team 1.
    const TASKS: [&[u32]; 3] = [&[1, 2, 3, 7, 8], &[11], &[21]];
    /// How many different round IDs the operations draw: few, so that they meet.
    const ROUND_POOL: u32 = 8;

    /// What one operation did, for the message of a failure.
    #[derive(Debug)]
    enum Operation {
        Open {
            team: u32,
            round: u32,
            item: Option<&'static str>,
        },
        Close {
            round: u32,
            how: RoundEnd,
        },
        Advance {
            team: u32,
            item: &'static str,
        },
        Begin {
            task: u32,
            run: u32,
            rounds: Vec<u32>,
        },
        End {
            task: u32,
        },
    }

    /// A random operation; `runs` counts the runs started so far, for their IDs.
    fn operation(rng: &mut impl Rng, runs: &mut u32) -> Operation {
        let team = rng.random_range(1..=3_u32);
        let items = ITEMS[usize::try_from(team - 1).unwrap()];
        let tasks = TASKS[usize::try_from(team - 1).unwrap()];
        match rng.random_range(0..5) {
            0 => Operation::Open {
                team,
                round: rng.random_range(1..=ROUND_POOL),
                item: (rng.random_range(0..5) != 0)
                    .then(|| items.get(rng.random_range(0..4)).copied())
                    .flatten(),
            },
            1 => Operation::Close {
                round: rng.random_range(1..=ROUND_POOL),
                how: RoundEnd::ALL[rng.random_range(0..RoundEnd::ALL.len())],
            },
            2 => Operation::Advance {
                team,
                item: items.get(rng.random_range(0..4)).copied().unwrap_or("1.1"),
            },
            3 => {
                *runs += 1;
                let named = rng.random_range(0..3);
                Operation::Begin {
                    task: tasks[rng.random_range(0..tasks.len())],
                    run: *runs,
                    rounds: (0..named).map(|_| rng.random_range(1..=ROUND_POOL)).collect(),
                }
            }
            _ => Operation::End {
                task: tasks[rng.random_range(0..tasks.len())],
            },
        }
    }

    /// Carries out `operation`: whether the state accepted it. An operation of several steps stops at the first the
    /// state refuses, and every step it takes leaves a valid state.
    fn apply(state: &mut State, operation: &Operation, step: u32) -> bool {
        match operation {
            Operation::Open {
                team,
                round: id,
                item: for_item,
            } => state
                .open_round(&team_id(*team), round(*id), for_item.map(item), at(step))
                .is_ok(),
            Operation::Close {
                round: id,
                how: RoundEnd::Abandoned,
            } => close_abandoned(state, *id, step),
            Operation::Close { round: id, how } => state.close_round(&round(*id), at(step + 100), *how).is_ok(),
            Operation::Advance { team, item: id } => advance(state, *team, id, step),
            Operation::Begin {
                task: owner,
                run: id,
                rounds,
            } => state
                .begin_run(
                    &task(*owner),
                    NewRun {
                        id: run(*id),
                        at: at(step),
                        session: None,
                        rounds: rounds.iter().map(|&n| round(n)).collect(),
                    },
                )
                .is_ok(),
            Operation::End { task: owner } => state
                .find_task_mut(&task(*owner))
                .is_some_and(|(_, mut member)| member.end_run(at(step + 100), RunEnd::Stopped).is_ok()),
        }
    }

    /// Stops the crew before abandoning its round `id`; a refusal leaves `state` untouched.
    fn close_abandoned(state: &mut State, id: u32, step: u32) -> bool {
        let mut next = state.clone();
        let Some(team) = next.teams().iter().find(|team| team.round(&round(id)).is_some()) else {
            return false;
        };
        let tasks: Vec<_> = team
            .crew()
            .roster()
            .into_iter()
            .map(|(_, member)| member.task().clone())
            .collect();
        for task in tasks {
            let Some((_, mut member)) = next.find_task_mut(&task) else {
                return false;
            };
            if matches!(
                member.status(),
                TaskStatus::Stopped | TaskStatus::Failed | TaskStatus::Done
            ) {
                continue;
            }
            if member.transition(TaskStatus::Stopped).is_err() {
                return false;
            }
        }
        if next
            .close_round(&round(id), at(step + 100), RoundEnd::Abandoned)
            .is_err()
        {
            return false;
        }
        *state = next;
        true
    }

    /// Moves item `id` of team `team` one phase on, along the phases an item takes to be merged.
    fn advance(state: &mut State, team: u32, id: &str, step: u32) -> bool {
        let Some(phase) = state
            .team(&team_id(team))
            .and_then(|found| found.item(item(id)))
            .map(|found| found.phase().clone())
        else {
            return false;
        };
        let mut handle = state.team_mut(&team_id(team)).unwrap();
        match phase {
            ItemPhase::Pending => handle
                .advance_item(
                    item(id),
                    ItemPhase::InProgress {
                        assignee: "codex-1".parse().unwrap(),
                        branch: "agents/codex-1/item-1-2".parse().unwrap(),
                    },
                )
                .is_ok(),
            ItemPhase::InProgress { .. } => handle
                .item_mut(item(id))
                .unwrap()
                .advance(ItemPhase::InReview {
                    pr: pr(7),
                    reviews: Vec::new(),
                })
                .is_ok(),
            ItemPhase::InReview { pr: opened, reviews } if reviews.is_empty() => {
                let asked = vec![
                    review("inv-codex", 7, ReviewOutcome::Pending),
                    review("inv-gemini", 8, ReviewOutcome::Pending),
                ];
                let mut handle = handle.item_mut(item(id)).unwrap();
                handle.ask_reviews(asked).is_ok() && {
                    let submitted = [7, 8]
                        .iter()
                        .all(|&n| handle.submit_review(&task(n), ReviewVerdict::Approve, at(step)).is_ok());
                    submitted && {
                        let ItemPhase::InReview { reviews, .. } = handle.phase().clone() else {
                            return false;
                        };
                        handle.advance(ItemPhase::Approved { pr: opened, reviews }).is_ok()
                    }
                }
            }
            ItemPhase::Approved { pr: opened, .. } => handle
                .item_mut(item(id))
                .unwrap()
                .advance(ItemPhase::Done {
                    completion: agent_core::state::Completion::Merged {
                        pr: opened,
                        at: at(step),
                    },
                })
                .is_ok(),
            ItemPhase::InReview { .. } | ItemPhase::Fixing { .. } | ItemPhase::Done { .. } => false,
        }
    }

    #[test]
    fn any_sequence_of_operations_leaves_a_state_that_saves_and_loads_to_the_same_text() {
        for seed in 0..SEEDS {
            let mut rng = Xoshiro256PlusPlus::seed_from_u64(seed);
            let mut state = state();
            let mut runs = 0;
            let mut accepted = 0;
            for step in 0..STEPS {
                let before = state.clone();
                let operation = operation(&mut rng, &mut runs);

                let done = apply(&mut state, &operation, step);

                if !done {
                    assert_eq!(
                        state, before,
                        "seed {seed}, step {step}: {operation:?} was refused but changed the state"
                    );
                    continue;
                }
                accepted += 1;
                let text = state.to_json().unwrap();
                let loaded = State::from_json(&text).unwrap_or_else(|error| {
                    panic!("seed {seed}, step {step}: {operation:?} left a state that is not read: {error}")
                });
                assert_eq!(loaded, state, "seed {seed}, step {step}: {operation:?}");
                assert_eq!(
                    loaded.to_json().unwrap(),
                    text,
                    "seed {seed}, step {step}: {operation:?}"
                );
            }
            assert!(accepted > 0, "seed {seed}: no operation was accepted");
        }
    }

    #[test]
    fn the_sequences_open_close_and_name_rounds_often_enough_to_mean_something() {
        let (mut opened, mut closed, mut named) = (0, 0, 0);
        for seed in 0..SEEDS {
            let mut rng = Xoshiro256PlusPlus::seed_from_u64(seed);
            let mut state = state();
            let mut runs = 0;
            for step in 0..STEPS {
                let operation = operation(&mut rng, &mut runs);
                if apply(&mut state, &operation, step) {
                    match operation {
                        Operation::Open { .. } => opened += 1,
                        Operation::Close { .. } => closed += 1,
                        Operation::Begin { ref rounds, .. } if !rounds.is_empty() => named += 1,
                        Operation::Begin { .. } | Operation::Advance { .. } | Operation::End { .. } => {}
                    }
                }
            }
        }

        assert!(
            opened > 500 && closed > 100 && named > 50,
            "{opened} opened, {closed} closed, {named} named"
        );
    }
}
