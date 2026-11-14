//! A history on disk for the tests of `agent log`: a team of two rounds, the first with a run, its launch, steps, end and
//! changes and the life of its pull request from its opening to its merge, the second with a run only, written through the
//! writer the daemon uses.

use std::num::NonZeroU32;
use std::path::Path;

use agent_core::cli::ActivityEvent;
use agent_core::decide::checks::{Change, FileChange, Lines};
use agent_core::history::{
    AgentIdentity, CommitRecord, EventRecord, HistoryEvent, HowChosen, ItemText, Launch, MaxTextBytes, PlanEntry,
    Record, RoundOpening, RoundSubject, RunChanges, RunEndRecord, RunStart, RunWhy, Step, TeamStart,
};
use agent_core::ids::{RoundId, TaskId, TeamId};
use agent_core::kinds::{CliKind, Role, Tier};
use agent_core::state::{CommitId, MergeMode, PrNumber, RoundEnd, RunEnd, Timestamp};
use agent_core::status::{ReviewVerdict, RunOutcome};
use agent_runtime::history::schema::MIGRATIONS;
use agent_runtime::history::{HistoryDb, Store};

/// A prompt of seven lines.
pub(crate) const PROMPT: &str = "line 1\nline 2\nline 3\nline 4\nline 5\nline 6\nline 7";

/// What a history has: the ids a test asks for.
#[derive(Debug, Clone)]
pub(crate) struct Written {
    /// The team.
    pub(crate) team: TeamId,
    /// Its rounds, in the order they were opened.
    pub(crate) rounds: [RoundId; 2],
}

/// `seconds` after 19:30:00 UTC on 2026-11-11.
fn at(seconds: i64) -> Timestamp {
    let start: Timestamp = "2026-11-11T19:30:00Z".parse().unwrap();
    Timestamp::from_utc(start.as_utc() + time::Duration::seconds(seconds)).unwrap()
}

/// A commit of forty copies of `digit`.
fn head(digit: char) -> CommitId {
    digit.to_string().repeat(40).parse().unwrap()
}

fn pr() -> PrNumber {
    PrNumber::new(NonZeroU32::new(7).unwrap())
}

fn number(n: u32) -> NonZeroU32 {
    NonZeroU32::new(n).unwrap()
}

fn event(team: &TeamId, rounds: &[RoundId], seconds: i64, event: HistoryEvent) -> Record {
    Record::Event(Box::new(EventRecord {
        at: at(seconds),
        team: team.clone(),
        rounds: rounds.to_vec(),
        event,
    }))
}

/// The run `n` of `team` for `rounds`, starting at `seconds`.
fn run_records(team: &TeamId, rounds: &[RoundId], n: u32, seconds: i64) -> Vec<Record> {
    let run: agent_core::ids::RunId = format!("run-20261111-1930{n:02}-r7q2").parse().unwrap();
    let task: TaskId = format!("task-20261111-1930{n:02}-k3x9").parse().unwrap();
    let mut records = vec![Record::Run(Box::new(RunStart {
        run: run.clone(),
        team: team.clone(),
        task,
        at: at(seconds),
        agent: AgentIdentity {
            name: "dev-1".to_owned(),
            tier: Tier::Normal,
            cli: CliKind::Claude,
            model: None,
        },
        role: Role::Member,
        rounds: rounds.to_vec(),
        why: RunWhy::Item {
            item: "1.1".parse().unwrap(),
        },
        prompt: PROMPT.to_owned(),
        worktree: "/work/tree".parse().unwrap(),
        branch: "agents/dev-1/1".parse().unwrap(),
        session: None,
    }))];
    records.push(Record::Launch(Box::new(Launch {
        run: run.clone(),
        at: at(seconds + 1),
        program: "claude".to_owned(),
        args: vec!["-p".to_owned(), PROMPT.to_owned()],
        working_dir: "/work/tree".parse().unwrap(),
        head: Some(head('1')),
        env_names: Vec::new(),
        prompt_in_file: false,
    })));
    for index in 1..=2 {
        records.push(Record::Step(Box::new(Step {
            run: run.clone(),
            number: number(index),
            at: at(seconds + 1 + i64::from(index)),
            event: ActivityEvent::Text(format!("step {index} of the run")),
        })));
    }
    records.push(Record::RunEnd(Box::new(RunEndRecord {
        run: run.clone(),
        at: at(seconds + 12),
        end: RunEnd::Reported {
            outcome: RunOutcome::Done,
        },
        final_message: Some("All done.\nSTATUS: DONE".to_owned()),
    })));
    records.push(Record::Changes(Box::new(RunChanges {
        run,
        head_before: Some(head('1')),
        head_after: Some(head('2')),
        commits: vec![CommitRecord {
            id: head('2'),
            subject: "Add the thing".to_owned(),
            authored_at: at(seconds + 12),
        }],
        files: vec![FileChange {
            path: "src/new.rs".parse().unwrap(),
            change: Change::Added,
            lines: Lines::Text { added: 10, removed: 0 },
        }],
        uncommitted: Vec::new(),
    })));
    records
}

/// Writes the history to `path`, with the pull request of the first round, and returns what a test asks for.
pub(crate) fn write_history(path: &Path) -> Written {
    let (written, records) = records();
    let mut db = HistoryDb::open(path, MaxTextBytes::default()).unwrap();
    db.write(&records).unwrap();
    written
}

/// Writes the history as a daemon of item 8.4 left it: version 2, no ledger, so no pull request and no event.
pub(crate) fn write_old_history(path: &Path) -> Written {
    let (written, records) = records();
    let plain: Vec<Record> = records
        .into_iter()
        .filter(|record| !matches!(record, Record::Event(_)))
        .collect();
    let mut db = HistoryDb::open_with(path, &MIGRATIONS[..2], MaxTextBytes::default()).unwrap();
    db.write(&plain).unwrap();
    written
}

/// The records of the history, in the order they are written, and the ids.
fn records() -> (Written, Vec<Record>) {
    let team: TeamId = "team-20261111-193001-t5m8".parse().unwrap();
    let rounds: [RoundId; 2] = [
        "round-20261111-193001-q1w2".parse().unwrap(),
        "round-20261111-193002-q1w2".parse().unwrap(),
    ];
    let mut records = vec![Record::Team(Box::new(TeamStart {
        team: team.clone(),
        at: at(0),
        repo: "demo".to_owned(),
        base_branch: "main".parse().unwrap(),
        merge: MergeMode::Autopilot,
        task_list: "Build.\n".to_owned(),
        plan: vec![PlanEntry {
            agent: "dev-1".parse().unwrap(),
            position: number(1),
            role: Role::Solo,
            how: HowChosen::Named,
        }],
    }))];
    for (index, round) in rounds.iter().enumerate() {
        records.push(Record::Round(Box::new(RoundOpening {
            round: round.clone(),
            team: team.clone(),
            at: at(1 + i64::try_from(index).unwrap() * 100),
            subject: RoundSubject::Item(ItemText {
                item: format!("1.{}", index + 1).parse().unwrap(),
                title: format!("Item {} — zażółć", index + 1),
                description: "Do the thing.".to_owned(),
                expected_result: None,
                checks: None,
            }),
        })));
    }
    records.extend(run_records(&team, &rounds[..1], 1, 2));
    let first = &rounds[..1];
    let reviewer = "inv-c".parse().unwrap();
    let task: TaskId = "task-20261111-193020-k3x9".parse().unwrap();
    for (seconds, happening) in [
        (
            20,
            HistoryEvent::PullOpened {
                number: pr(),
                branch: "agents/dev-1/1".parse().unwrap(),
                head: head('2'),
                title: "Add the thing".to_owned(),
                body: "The body.\nSecond line.".to_owned(),
            },
        ),
        (
            21,
            HistoryEvent::ReviewAsked {
                pr: pr(),
                pass: 0,
                head: head('2'),
                reviewer: "inv-c".parse().unwrap(),
                task: task.clone(),
            },
        ),
        (
            22,
            HistoryEvent::ReviewSubmitted {
                pr: pr(),
                pass: 0,
                reviewer,
                task,
                verdict: ReviewVerdict::Approve,
                report: "No findings.\nVERDICT: APPROVE".to_owned(),
            },
        ),
        (
            23,
            HistoryEvent::PullMerged {
                pr: pr(),
                commit: Some(head('4')),
            },
        ),
        (
            24,
            HistoryEvent::RoundClosed {
                round: rounds[0].clone(),
                end: RoundEnd::Merged,
            },
        ),
    ] {
        records.push(event(&team, first, seconds, happening));
    }
    records.extend(run_records(&team, &rounds[1..], 2, 102));
    records.push(event(&team, &[], 200, HistoryEvent::TeamEnded {}));
    (Written { team, rounds }, records)
}
