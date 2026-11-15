//! Which records a story is made of and in which order: each record once, the order of time with a tie going to the
//! record written first, a round with the start of its team, and teams that run at once sharing nothing. The JSON of a
//! record carries all of it. Histories come from a seeded source: a failure names its seed.

#[cfg(test)]
#[path = "support/story_world.rs"]
mod story_world;

#[cfg(test)]
mod story_records {
    use std::collections::BTreeSet;

    use agent_core::history::{Cut, Record, SequencedRecord};
    use agent_core::ids::{RoundId, TeamId};
    use agent_core::story::{Selection, json_lines, selected};
    use rand::rngs::Xoshiro256PlusPlus;
    use rand::seq::SliceRandom;
    use rand::{RngExt, SeedableRng};
    use serde_json::Value;

    use super::story_world::{Written, at, round, team};

    /// The sequences of the records of `lines`.
    fn sequences(lines: &[String]) -> Vec<u64> {
        lines
            .iter()
            .map(|line| {
                serde_json::from_str::<Value>(line).unwrap()["sequence"]
                    .as_u64()
                    .unwrap()
            })
            .collect()
    }

    /// Two teams that ran at once, their records interleaved at random, with times that often tie: the order of the
    /// writes is the order of the vector.
    fn history(seed: u64) -> Vec<SequencedRecord> {
        let mut rng = Xoshiro256PlusPlus::seed_from_u64(seed);
        let mut pieces = Vec::new();
        for (index, id) in [1_u32, 2].into_iter().enumerate() {
            let mut written = Written::default();
            let rounds: Vec<RoundId> = (0..rng.random_range(1..3_u32)).map(|n| round(id * 10 + n)).collect();
            written.team(&team(id), 0);
            for (position, opened) in rounds.iter().enumerate() {
                written.round(&team(id), opened, 1);
                written.run(
                    &team(id),
                    std::slice::from_ref(opened),
                    id * 10 + u32::try_from(position).unwrap(),
                    2,
                    "prompt",
                    rng.random_range(0..4),
                );
                written.checked(&team(id), std::slice::from_ref(opened), 20);
                written.pull_request(&team(id), std::slice::from_ref(opened), 30);
            }
            if rounds.len() == 2 {
                written.run(&team(id), &rounds, id * 10 + 5, 3, "a coordinator for both rounds", 1);
            }
            pieces.push((index, written.records));
        }
        let mut merged: Vec<SequencedRecord> = Vec::new();
        let mut queues: Vec<std::collections::VecDeque<SequencedRecord>> =
            pieces.into_iter().map(|(_, records)| records.into()).collect();
        while queues.iter().any(|queue| !queue.is_empty()) {
            let live: Vec<usize> = (0..queues.len()).filter(|index| !queues[*index].is_empty()).collect();
            let from = live[rng.random_range(0..live.len())];
            let mut record = queues[from].pop_front().unwrap();
            record.sequence = u64::try_from(merged.len()).unwrap() + 1;
            record.at = at(rng.random_range(0..4));
            merged.push(record);
        }
        merged
    }

    /// The records of `team` as the story orders them: by time, then by the order of writing.
    fn expected_of(records: &[SequencedRecord], team: &TeamId) -> Vec<u64> {
        let mut own: Vec<&SequencedRecord> = records.iter().filter(|record| &record.team == team).collect();
        own.sort_by_key(|record| (record.at, record.sequence));
        own.iter().map(|record| record.sequence).collect()
    }

    #[test]
    fn the_json_of_a_team_has_each_of_its_records_once_in_the_order_of_time_then_of_writing() {
        for seed in 0..60 {
            let records = history(seed);
            for id in [1, 2] {
                let lines = json_lines(&records, &Selection::Team(team(id)));

                assert_eq!(
                    sequences(&lines),
                    expected_of(&records, &team(id)),
                    "seed={seed} team={id}"
                );
            }
        }
    }

    #[test]
    fn records_of_the_same_time_keep_the_order_they_were_written_in() {
        let mut written = Written::default();
        written.team(&team(1), 0);
        written.round(&team(1), &round(1), 0);
        written.run(&team(1), &[round(1)], 1, 0, "p", 2);
        for record in &mut written.records {
            record.at = at(5);
        }

        let lines = json_lines(&written.records, &Selection::Team(team(1)));

        let expected: Vec<u64> = (1..=u64::try_from(written.records.len()).unwrap()).collect();
        assert_eq!(sequences(&lines), expected);
    }

    #[test]
    fn two_teams_that_ran_at_once_give_stories_that_share_no_record_and_hold_every_record() {
        for seed in 0..60 {
            let records = history(seed);

            let first = sequences(&json_lines(&records, &Selection::Team(team(1))));
            let second = sequences(&json_lines(&records, &Selection::Team(team(2))));

            let first: BTreeSet<u64> = first.into_iter().collect();
            let second: BTreeSet<u64> = second.into_iter().collect();
            assert!(first.is_disjoint(&second), "seed={seed}");
            let all: BTreeSet<u64> = records.iter().map(|record| record.sequence).collect();
            assert_eq!(
                first.union(&second).copied().collect::<BTreeSet<_>>(),
                all,
                "seed={seed}"
            );
        }
    }

    #[test]
    fn the_input_in_any_order_gives_the_same_lines() {
        for seed in 0..40 {
            let records = history(seed);
            let mut shuffled = records.clone();
            shuffled.shuffle(&mut Xoshiro256PlusPlus::seed_from_u64(seed));

            for selection in [Selection::Team(team(1)), Selection::Round(round(10))] {
                assert_ne!(json_lines(&records, &selection).len(), 0, "seed={seed}");
                assert_eq!(
                    json_lines(&shuffled, &selection),
                    json_lines(&records, &selection),
                    "seed={seed}"
                );
            }
        }
    }

    #[test]
    fn a_round_has_what_is_linked_to_it_and_the_start_of_its_team() {
        for seed in 0..60 {
            let records = history(seed);
            let target = round(10);

            let chosen = selected(&records, &Selection::Round(target.clone()));

            let mut expected: Vec<&SequencedRecord> = records
                .iter()
                .filter(|record| {
                    record.rounds.contains(&target)
                        || matches!(&record.record, Record::Team(start) if start.team == team(1))
                })
                .collect();
            expected.sort_by_key(|record| (record.at, record.sequence));
            assert_eq!(chosen, expected, "seed={seed}");
        }
    }

    #[test]
    fn a_record_of_two_rounds_is_in_both_round_stories_and_once_in_the_team_story() {
        let mut records = Vec::new();
        for seed in 0..200 {
            records = history(seed);
            if records.iter().any(|record| record.rounds.len() == 2) {
                break;
            }
        }
        let owner = records.iter().find(|record| record.rounds.len() == 2).unwrap();
        let shared: Vec<u64> = records
            .iter()
            .filter(|record| record.rounds.len() == 2 && record.team == owner.team)
            .map(|record| record.sequence)
            .collect();
        assert_ne!(shared.len(), 0);
        let [first, second] = [owner.rounds[0].clone(), owner.rounds[1].clone()];

        let in_first = sequences(&json_lines(&records, &Selection::Round(first)));
        let in_second = sequences(&json_lines(&records, &Selection::Round(second)));
        let in_team = sequences(&json_lines(&records, &Selection::Team(owner.team.clone())));

        for sequence in &shared {
            assert!(in_first.contains(sequence) && in_second.contains(sequence));
            assert_eq!(in_team.iter().filter(|found| *found == sequence).count(), 1);
        }
    }

    #[test]
    fn a_round_and_a_team_the_history_does_not_have_select_nothing() {
        let records = history(1);

        assert_eq!(
            selected(&records, &Selection::Round(round(59))),
            Vec::<&SequencedRecord>::new()
        );
        assert_eq!(
            selected(&records, &Selection::Team(team(59))),
            Vec::<&SequencedRecord>::new()
        );
    }

    #[test]
    fn a_json_line_names_the_place_the_time_the_team_the_rounds_the_kind_and_the_cut() {
        let mut written = Written::default();
        written.team(&team(1), 0);
        written.round(&team(1), &round(1), 1);
        written.run(&team(1), &[round(1)], 1, 2, "the prompt", 1);
        let cut = {
            let mut cut = Cut::default();
            cut.keep("prompt", "too long", agent_core::history::MaxTextBytes::new(3).unwrap());
            cut
        };
        written.records[2].cut = cut;

        let lines = json_lines(&written.records, &Selection::Round(round(1)));

        let run: Value = serde_json::from_str(&lines[2]).unwrap();
        assert_eq!(run["sequence"], 3);
        assert_eq!(run["team"], team(1).as_str());
        assert_eq!(run["rounds"], serde_json::json!([round(1).as_str()]));
        assert_eq!(run["kind"], "run");
        assert_eq!(run["cut"], serde_json::json!(["prompt"]));
        assert_eq!(run["record"]["prompt"], "the prompt");
        assert_eq!(run["record"]["worktree"], "/work/tree");
        let launch: Value = serde_json::from_str(&lines[3]).unwrap();
        assert_eq!(
            launch["record"]["args"],
            serde_json::json!(["-p", "the prompt", "--verbose"])
        );
        let step: Value = serde_json::from_str(&lines[4]).unwrap();
        assert_eq!(step["kind"], "step");
        assert_eq!(step["record"]["number"], 1);
        assert_eq!(lines.iter().filter(|line| line.contains('\n')).count(), 0);
    }

    #[test]
    fn the_json_of_an_event_is_the_event_and_the_json_of_changes_has_every_file() {
        let mut written = Written::default();
        written.team(&team(1), 0);
        written.round(&team(1), &round(1), 1);
        written.run(&team(1), &[round(1)], 1, 2, "p", 0);
        written.pull_request(&team(1), &[round(1)], 30);

        let lines = json_lines(&written.records, &Selection::Team(team(1)));

        let values: Vec<Value> = lines.iter().map(|line| serde_json::from_str(line).unwrap()).collect();
        let opened = values.iter().find(|value| value["kind"] == "pull_opened").unwrap();
        assert_eq!(opened["record"]["event"], "pull_opened");
        assert_eq!(opened["record"]["number"], 7);
        assert_eq!(
            opened["record"]["body"],
            "The body of the pull request.\nSecond line — 日本語."
        );
        let changes = values.iter().find(|value| value["kind"] == "changes").unwrap();
        assert_eq!(changes["record"]["files"].as_array().unwrap().len(), 3);
        assert_eq!(changes["record"]["uncommitted"].as_array().unwrap().len(), 1);
        assert_eq!(changes["record"]["commits"][0]["subject"], "Add the thing");
        assert_eq!(changes["record"]["files"][1]["from"], "src/old.rs");
    }
}
