//! Every kind of record written and read back equal, the cut of a long text, a record written twice, and what the
//! database refuses.

use std::num::NonZeroU32;
use std::str::FromStr;

use agent_core::cli::ActivityEvent;
use agent_core::decide::checks::{Change, FileChange, Lines};
use agent_core::history::{
    CommitRecord, ItemText, Record, RoundOpening, RoundSubject, RunChanges, RunEndRecord, Step, TeamStart,
};
use agent_core::repo_path::RepoPath;
use agent_core::state::RunEnd;
use agent_runtime::history::{HistoryDb, Reader, Store, WriteError, WriteKind};
use rand::rngs::Xoshiro256PlusPlus;
use rand::{RngExt, SeedableRng};

use super::support::{World, at, commit, complete, database, limit, world};

/// Writes `records` in chunks of random size and returns how many chunks there were.
fn write_in_chunks(db: &mut HistoryDb, records: &[Record], seed: u64) -> usize {
    let mut rng = Xoshiro256PlusPlus::seed_from_u64(seed);
    let mut chunks = 0;
    let mut rest = records;
    while !rest.is_empty() {
        let take = rng.random_range(1..=rest.len().min(7));
        let (now, later) = rest.split_at(take);
        db.write(now).unwrap_or_else(|error| panic!("seed {seed}: {error:?}"));
        rest = later;
        chunks += 1;
    }
    chunks
}

/// What the reader gives for the world, in the shape of the world.
fn read_back(reader: &Reader, world: &World) -> World {
    let team = reader.team(&world.team.team).unwrap().unwrap().value;
    let rounds = reader
        .rounds_of_team(&world.team.team)
        .unwrap()
        .into_iter()
        .map(|stored| stored.value)
        .collect();
    let runs = reader
        .runs_of_team(&world.team.team)
        .unwrap()
        .into_iter()
        .map(|row| {
            let run = row.start.value.run.clone();
            super::support::RunWorld {
                steps: reader
                    .steps(&run)
                    .unwrap()
                    .into_iter()
                    .map(|stored| stored.value)
                    .collect(),
                changes: reader.changes(&run).unwrap().map(|stored| stored.value),
                launch: row.launch.map(|stored| stored.value),
                end: row.end.map(|stored| stored.value),
                start: row.start.value,
            }
        })
        .collect();
    World { team, rounds, runs }
}

#[test]
fn any_world_of_records_is_read_back_equal_whatever_the_size_of_the_batches() {
    for seed in 0..300_u64 {
        let (_dir, path, mut db) = database();
        let written = world(seed);
        write_in_chunks(&mut db, &written.records(), seed);

        let read = read_back(&Reader::open(&path).unwrap(), &written);

        assert_eq!(read, written, "seed {seed}");
    }
}

#[test]
fn the_runs_of_a_round_and_its_prompts_are_those_that_name_it_oldest_first() {
    for seed in 0..100_u64 {
        let (_dir, path, mut db) = database();
        let written = world(seed);
        db.write(&written.records()).unwrap();
        let reader = Reader::open(&path).unwrap();

        for round in &written.rounds {
            let expected: Vec<_> = written
                .runs
                .iter()
                .filter(|run| run.start.rounds.contains(&round.round))
                .map(|run| &run.start)
                .collect();
            let runs = reader.runs_of_round(&round.round).unwrap();
            let prompts = reader.prompts_of_round(&round.round).unwrap();

            assert_eq!(
                runs.iter().map(|row| &row.start.value).collect::<Vec<_>>(),
                expected,
                "seed {seed}"
            );
            assert_eq!(
                prompts
                    .iter()
                    .map(|each| (&each.run, &each.prompt, each.agent.as_str(), each.role, each.at))
                    .collect::<Vec<_>>(),
                expected
                    .iter()
                    .map(|run| (&run.run, &run.prompt, run.agent.name.as_str(), run.role, run.at))
                    .collect::<Vec<_>>(),
                "seed {seed}"
            );
            assert!(prompts.iter().all(|each| !each.was_cut));
            assert_eq!(
                reader.round(&round.round).unwrap().unwrap().value,
                *round,
                "seed {seed}"
            );
        }
    }
}

#[test]
fn what_was_not_written_is_not_there() {
    let (_dir, path, mut db) = database();
    let written = world(1);
    db.write(&written.records()).unwrap();
    let other = world(2);
    let reader = Reader::open(&path).unwrap();

    assert_eq!(reader.team(&written.team.team).unwrap().unwrap().value, written.team);
    assert_eq!(reader.team(&other.team.team).unwrap(), None);
    assert_eq!(reader.run(&other.runs[0].start.run).unwrap(), None);
    assert_eq!(reader.steps(&other.runs[0].start.run).unwrap(), []);
    assert_eq!(reader.changes(&other.runs[0].start.run).unwrap(), None);
    assert_eq!(reader.runs_of_team(&other.team.team).unwrap(), []);
    assert_eq!(reader.rounds_of_team(&other.team.team).unwrap(), []);
}

#[test]
fn text_in_every_script_and_with_awkward_characters_comes_back_as_it_went_in() {
    let (_dir, path, mut db) = database();
    let mut written = world(3);
    let nasty = "ü é € 😀 日本語 \u{0} ' \" ; -- /* */ \\ % _ \n\r\t DROP TABLE steps;";
    written.team.task_list = nasty.to_owned();
    written.runs[0].start.prompt = nasty.repeat(20);
    written.runs[0].steps = vec![Step {
        run: written.runs[0].start.run.clone(),
        number: NonZeroU32::new(1).unwrap(),
        at: at(1, 123_456_789),
        event: ActivityEvent::Text(nasty.to_owned()),
    }];
    written.runs[0].end = Some(RunEndRecord {
        run: written.runs[0].start.run.clone(),
        at: at(2000, 999_999_999),
        end: RunEnd::Hung,
        final_message: Some(nasty.to_owned()),
    });
    db.write(&written.records()).unwrap();

    let read = read_back(&Reader::open(&path).unwrap(), &written);

    assert_eq!(read, written);
}

/// A limit of 16 bytes, and texts around it.
const LIMIT: u32 = 16;

/// What a field of `LIMIT` bytes of text reads back as and whether it was cut: under, exactly and over; and over with
/// characters of two, three and four bytes where the limit falls inside one.
fn around_the_limit() -> Vec<(String, String, bool)> {
    let exactly = "a".repeat(16);
    let over = "b".repeat(17);
    let under = "c".repeat(15);
    // 7 × 2 bytes = 14, the 8th would end at 16: kept; 9 × 2 = 18: cut after 8.
    let two = "é".repeat(9);
    // 5 × 3 = 15, the 6th ends at 18: cut after 5.
    let three = "€".repeat(6);
    // 4 × 4 = 16: kept whole; 5 × 4 = 20: cut after 4.
    let four_exact = "😀".repeat(4);
    let four_over = "😀".repeat(5);
    vec![
        (under.clone(), under, false),
        (exactly.clone(), exactly, false),
        (over, "b".repeat(16), true),
        (two, "é".repeat(8), true),
        (three, "€".repeat(5), true),
        (four_exact.clone(), four_exact, false),
        (four_over, "😀".repeat(4), true),
    ]
}

#[test]
fn a_text_at_the_limit_one_under_and_over_is_kept_or_cut_at_a_character_boundary_and_marked() {
    for (given, kept, cut) in around_the_limit() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("history.db");
        let mut db = HistoryDb::open(&path, limit(LIMIT)).unwrap();
        let mut written = world(4);
        written.team.task_list = given.clone();
        written.rounds = vec![RoundOpening {
            round: written.rounds.first().map_or_else(
                || agent_core::ids::RoundId::from_str("round-20261114-081310-abcd").unwrap(),
                |round| round.round.clone(),
            ),
            team: written.team.team.clone(),
            at: at(1, 0),
            subject: RoundSubject::Item(ItemText {
                item: "1.1".parse().unwrap(),
                title: given.clone(),
                description: given.clone(),
                expected_result: Some(given.clone()),
                checks: Some(given.clone()),
            }),
        }];
        written.runs.truncate(1);
        written.runs[0].start.prompt = given.clone();
        written.runs[0].end = Some(RunEndRecord {
            run: written.runs[0].start.run.clone(),
            at: at(2000, 0),
            end: RunEnd::Stopped,
            final_message: Some(given.clone()),
        });
        let run = written.runs[0].start.run.clone();
        written.runs[0].steps = vec![Step {
            run: run.clone(),
            number: NonZeroU32::new(1).unwrap(),
            at: at(2, 0),
            event: ActivityEvent::BusCall {
                tool: given.clone(),
                arguments: given.clone(),
            },
        }];
        written.runs[0].changes = Some(RunChanges {
            run: run.clone(),
            head_before: None,
            head_after: None,
            commits: vec![CommitRecord {
                id: commit(&mut Xoshiro256PlusPlus::seed_from_u64(1)),
                subject: given.clone(),
                authored_at: at(3, 0),
            }],
            files: Vec::new(),
            uncommitted: Vec::new(),
        });
        db.write(&written.records()).unwrap();
        let reader = Reader::open(&path).unwrap();

        let team = reader.team(&written.team.team).unwrap().unwrap();
        let round = reader.round(&written.rounds[0].round).unwrap().unwrap();
        let row = reader.run(&run).unwrap().unwrap();
        let steps = reader.steps(&run).unwrap();
        let changes = reader.changes(&run).unwrap().unwrap();

        assert_eq!(team.value.task_list, kept, "{given:?}");
        assert_eq!(team.cut.has("task_list"), cut, "{given:?}");
        let RoundSubject::Item(item) = round.value.subject else {
            panic!("the round has an item")
        };
        assert_eq!(
            (item.title.as_str(), item.description.as_str()),
            (kept.as_str(), kept.as_str()),
            "{given:?}"
        );
        assert_eq!(item.expected_result.as_deref(), Some(kept.as_str()));
        assert_eq!(item.checks.as_deref(), Some(kept.as_str()));
        for field in ["title", "description", "expected_result", "checks"] {
            assert_eq!(round.cut.has(field), cut, "{field} of {given:?}");
        }
        assert_eq!(row.start.value.prompt, kept);
        assert_eq!(row.start.cut.has("prompt"), cut);
        assert_eq!(
            row.end.as_ref().unwrap().value.final_message.as_deref(),
            Some(kept.as_str())
        );
        assert_eq!(row.end.as_ref().unwrap().cut.has("final_message"), cut);
        assert_eq!(
            steps[0].value.event,
            ActivityEvent::BusCall {
                tool: kept.clone(),
                arguments: kept.clone()
            }
        );
        assert!(
            steps[0].cut.has("tool") == cut && steps[0].cut.has("detail") == cut,
            "{given:?}"
        );
        assert_eq!(changes.value.commits[0].subject, kept);
        assert_eq!(changes.cut.has("subject"), cut);
        assert_eq!(team.cut.is_empty(), !cut);
    }
}

#[test]
fn the_arguments_of_a_command_line_are_cut_one_by_one_so_the_list_stays_a_list() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("history.db");
    let mut db = HistoryDb::open(&path, limit(LIMIT)).unwrap();
    let mut written = world(5);
    written.runs.truncate(1);
    let run = written.runs[0].start.run.clone();
    let mut launch = written.runs[0]
        .launch
        .clone()
        .unwrap_or_else(|| agent_core::history::Launch {
            run: run.clone(),
            at: at(1, 0),
            program: "claude".to_owned(),
            args: Vec::new(),
            working_dir: written.runs[0].start.worktree.clone(),
            head: None,
            env_names: Vec::new(),
            prompt_in_file: false,
        });
    launch.args = vec!["-p".to_owned(), "x".repeat(40), "😀".repeat(9)];
    written.runs[0].launch = Some(launch);
    db.write(&written.records()).unwrap();

    let row = Reader::open(&path).unwrap().run(&run).unwrap().unwrap();

    let stored = row.launch.unwrap();
    assert_eq!(stored.value.args, ["-p".to_owned(), "x".repeat(16), "😀".repeat(4)]);
    assert!(stored.cut.has("args"));
}

#[test]
fn the_same_record_written_twice_is_one_row_and_the_first_write_wins() {
    let (_dir, path, mut db) = database();
    let written = world(6);
    let records = written.records();
    db.write(&records).unwrap();
    db.write(&records).unwrap();
    let mut changed = world(6);
    changed.team.task_list = "another list".to_owned();
    changed.runs[0].start.prompt = "another prompt".to_owned();
    db.write(&changed.records()).unwrap();

    let read = read_back(&Reader::open(&path).unwrap(), &written);

    assert_eq!(read, written);
    let raw = rusqlite::Connection::open(&path).unwrap();
    for (table, expected) in [
        ("teams", 1),
        ("plan", i64::try_from(written.team.plan.len()).unwrap()),
        ("rounds", i64::try_from(written.rounds.len()).unwrap()),
        ("runs", i64::try_from(written.runs.len()).unwrap()),
    ] {
        let rows: i64 = raw
            .query_row(&format!("SELECT count(*) FROM {table}"), [], |row| row.get(0))
            .unwrap();
        assert_eq!(rows, expected, "{table}");
    }
}

/// The error of writing `records` that the database is expected to refuse.
fn refused(db: &mut HistoryDb, records: &[Record]) -> WriteError {
    db.write(records).unwrap_err()
}

#[test]
fn a_record_that_belongs_to_a_run_the_history_never_saw_is_refused() {
    let (_dir, _path, mut db) = database();
    let world = complete(7);
    let orphan = &world.runs[0];
    let mut records = Vec::new();
    records.push(Record::Step(Box::new(orphan.steps[0].clone())));
    records.push(Record::Launch(Box::new(orphan.launch.clone().unwrap())));
    records.push(Record::RunEnd(Box::new(orphan.end.clone().unwrap())));
    if let Some(changes) = &orphan.changes {
        records.push(Record::Changes(Box::new(changes.clone())));
    }

    for record in records {
        let error = refused(&mut db, std::slice::from_ref(&record));

        assert_eq!(error.kind(), WriteKind::Refused, "{record:?}");
    }
}

#[test]
fn a_batch_with_one_refused_record_writes_none_of_its_records() {
    let (_dir, path, mut db) = database();
    let world = world(8);
    let orphan = world.runs.iter().find(|run| !run.steps.is_empty()).unwrap();
    let mut batch = world.records();
    batch.push(Record::Step(Box::new(Step {
        run: agent_core::ids::RunId::from_str("run-20260101-000000-zzzz").unwrap(),
        ..orphan.steps[0].clone()
    })));

    let error = refused(&mut db, &batch);

    assert_eq!(error.kind(), WriteKind::Refused);
    let raw = rusqlite::Connection::open(&path).unwrap();
    for table in [
        "teams",
        "plan",
        "rounds",
        "runs",
        "steps",
        "launches",
        "run_ends",
        "run_changes",
        "commits",
        "changed_files",
        "run_rounds",
    ] {
        let rows: i64 = raw
            .query_row(&format!("SELECT count(*) FROM {table}"), [], |row| row.get(0))
            .unwrap();
        assert_eq!(rows, 0, "{table} has rows of a batch that failed");
    }
}

/// How many rows the tables have, summed.
fn rows(path: &std::path::Path, tables: &[&str]) -> i64 {
    let raw = rusqlite::Connection::open(path).unwrap();
    tables
        .iter()
        .map(|table| {
            raw.query_row(&format!("SELECT count(*) FROM {table}"), [], |row| row.get::<_, i64>(0))
                .unwrap()
        })
        .sum()
}

#[test]
fn a_list_of_files_with_a_path_twice_is_refused_and_leaves_nothing_of_its_record() {
    let (_dir, path, mut db) = database();
    let world = world(9);
    let run = world.runs[0].clone();
    db.write(&[Record::Run(Box::new(run.start.clone()))]).unwrap();
    let twice = FileChange {
        path: RepoPath::from_str("src/a.rs").unwrap(),
        change: Change::Modified,
        lines: Lines::Text { added: 1, removed: 0 },
    };
    let changes = RunChanges {
        run: run.start.run.clone(),
        head_before: None,
        head_after: None,
        commits: Vec::new(),
        files: vec![twice.clone(), twice],
        uncommitted: Vec::new(),
    };

    let error = refused(&mut db, &[Record::Changes(Box::new(changes))]);

    assert_eq!(error.kind(), WriteKind::Refused);
    assert_eq!(rows(&path, &["run_changes", "commits", "changed_files"]), 0);
}

#[test]
fn a_run_that_names_a_round_twice_is_refused_and_leaves_no_row() {
    let (_dir, path, mut db) = database();
    let mut run = world(9).runs[0].start.clone();
    let round = agent_core::ids::RoundId::from_str("round-20261114-081310-abcd").unwrap();
    run.rounds = vec![round.clone(), round];

    let error = refused(&mut db, &[Record::Run(Box::new(run))]);

    assert_eq!(error.kind(), WriteKind::Refused);
    assert_eq!(rows(&path, &["runs", "run_rounds"]), 0);
}

#[test]
fn a_team_is_written_with_its_plan_and_a_plan_that_names_a_position_twice_is_refused() {
    let (_dir, _path, mut db) = database();
    let mut team: TeamStart = world(10).team;
    team.plan.push(team.plan[0].clone());

    let error = refused(&mut db, &[Record::Team(Box::new(team))]);

    assert_eq!(error.kind(), WriteKind::Refused);
}

#[test]
fn steps_come_back_in_their_order_whatever_the_order_they_were_written_in() {
    let (_dir, path, mut db) = database();
    let written = world(11);
    let run = written.runs.iter().find(|run| run.steps.len() >= 3).map_or_else(
        || {
            let mut run = written.runs[0].clone();
            run.steps = (1..=5)
                .map(|n| Step {
                    run: run.start.run.clone(),
                    number: NonZeroU32::new(n).unwrap(),
                    at: at(i64::from(n), 0),
                    event: ActivityEvent::Info(format!("step {n}")),
                })
                .collect();
            run
        },
        Clone::clone,
    );
    let mut records = vec![Record::Run(Box::new(run.start.clone()))];
    let mut steps = run.steps.clone();
    steps.reverse();
    records.extend(steps.into_iter().map(|step| Record::Step(Box::new(step))));
    db.write(&records).unwrap();

    let read = Reader::open(&path).unwrap().steps(&run.start.run).unwrap();

    assert_eq!(
        read.into_iter().map(|stored| stored.value).collect::<Vec<_>>(),
        run.steps
    );
}

#[test]
fn a_limit_of_one_byte_keeps_one_byte_and_a_wide_first_character_leaves_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("history.db");
    let mut db = HistoryDb::open(&path, limit(1)).unwrap();
    let mut written = world(12);
    written.team.task_list = "😀abc".to_owned();
    db.write(&[Record::Team(Box::new(written.team.clone()))]).unwrap();

    let team = Reader::open(&path).unwrap().team(&written.team.team).unwrap().unwrap();

    assert_eq!(team.value.task_list, "");
    assert!(team.cut.has("task_list"));
}
