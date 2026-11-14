//! Values for the tests of the history: a world of records drawn from a seeded source, and small helpers.

use std::collections::BTreeMap;
use std::num::NonZeroU32;
use std::path::PathBuf;
use std::str::FromStr;

use agent_core::cli::ActivityEvent;
use agent_core::config::{AbsolutePath, EnvName, ModelName, TailscaleLogin};
use agent_core::decide::checks::{Change, FileChange, Lines};
use agent_core::history::{
    AgentIdentity, CommitRecord, HowChosen, ItemText, Launch, MaxTextBytes, PlanEntry, Record, RoundOpening,
    RoundSubject, RunChanges, RunEndRecord, RunStart, RunWhy, Step, TeamStart,
};
use agent_core::ids::{AgentName, BranchName, Clock, ItemId, RoundId, RunId, TaskId, TeamId};
use agent_core::kinds::{CliKind, Role, Tier};
use agent_core::protocol::{Client, ControlAction};
use agent_core::repo_path::RepoPath;
use agent_core::state::{CommitId, MergeMode, PrNumber, RunEnd, SessionId, Timestamp};
use agent_core::status::RunOutcome;
use agent_runtime::history::HistoryDb;
use rand::rngs::Xoshiro256PlusPlus;
use rand::{Rng, RngExt, SeedableRng};
use time::UtcDateTime;

/// A clock that stands still.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Still(pub i64);

impl Clock for Still {
    fn now(&self) -> UtcDateTime {
        UtcDateTime::from_unix_timestamp(self.0).unwrap()
    }
}

/// The limit most tests use: far above any text they write.
pub(crate) fn roomy() -> MaxTextBytes {
    MaxTextBytes::default()
}

/// A limit of `bytes`.
pub(crate) fn limit(bytes: u32) -> MaxTextBytes {
    MaxTextBytes::new(bytes).unwrap()
}

/// A history database in a new temporary directory, open for writing, and the directory.
pub(crate) fn database() -> (tempfile::TempDir, PathBuf, HistoryDb) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("history.db");
    let db = HistoryDb::open(&path, roomy()).unwrap();
    (dir, path, db)
}

/// A time `seconds` after a fixed start, with `nanos` nanoseconds.
pub(crate) fn at(seconds: i64, nanos: u32) -> Timestamp {
    let utc = UtcDateTime::from_unix_timestamp(1_795_000_000 + seconds)
        .unwrap()
        .replace_nanosecond(nanos)
        .unwrap();
    Timestamp::from_utc(utc).unwrap()
}

/// A character of one, two, three or four bytes, or a character that needs care in SQL or in text.
fn character(rng: &mut impl Rng) -> char {
    const AWKWARD: [char; 9] = ['\'', '"', '\\', '%', '_', '\n', '\t', '\r', '\u{0}'];
    match rng.random_range(0..8_u8) {
        0..=2 => char::from(rng.random_range(b'a'..=b'z')),
        3 => 'é',
        4 => '€',
        5 => '😀',
        6 => char::from(rng.random_range(b'A'..=b'Z')),
        _ => AWKWARD[rng.random_range(0..AWKWARD.len())],
    }
}

/// A text of up to `longest` characters, empty included.
pub(crate) fn text(rng: &mut impl Rng, longest: usize) -> String {
    let length = rng.random_range(0..=longest);
    (0..length).map(|_| character(rng)).collect()
}

/// A text that is not empty.
pub(crate) fn some_text(rng: &mut impl Rng, longest: usize) -> String {
    let mut drawn = text(rng, longest);
    if drawn.is_empty() {
        drawn.push('x');
    }
    drawn
}

/// An agent name of the shape the configuration allows.
fn agent_name(rng: &mut impl Rng) -> AgentName {
    AgentName::from_str(&format!("agent-{}", rng.random_range(0..40_u32))).unwrap()
}

/// A 40-digit commit ID.
pub(crate) fn commit(rng: &mut impl Rng) -> CommitId {
    let digits: String = (0..40)
        .map(|_| char::from(b"0123456789abcdef"[rng.random_range(0..16)]))
        .collect();
    CommitId::from_str(&digits).unwrap()
}

/// A path of a repository.
fn repo_path(rng: &mut impl Rng) -> RepoPath {
    let depth = rng.random_range(1..4_u8);
    let parts: Vec<String> = (0..depth)
        .map(|_| {
            format!(
                "{}{}",
                ["src", "docs", "tests", "é-dir", "😀"][rng.random_range(0..5)],
                rng.random_range(0..9_u8)
            )
        })
        .collect();
    RepoPath::from_str(&format!("{}/{}.rs", parts.join("/"), rng.random_range(0..9_u8))).unwrap()
}

/// One changed file: a change of any kind, text or binary.
fn file_change(rng: &mut impl Rng, path: RepoPath) -> FileChange {
    let change = match rng.random_range(0..4_u8) {
        0 => Change::Added,
        1 => Change::Deleted,
        2 => Change::Modified,
        _ => Change::Renamed {
            from: RepoPath::from_str(&format!("old/{}", path.as_str())).unwrap(),
        },
    };
    let lines = if rng.random_range(0..4_u8) == 0 {
        Lines::Binary
    } else {
        Lines::Text {
            added: rng.random_range(0..5000),
            removed: rng.random_range(0..5000),
        }
    };
    FileChange { path, change, lines }
}

/// A list of changed files with no path twice.
fn file_changes(rng: &mut impl Rng) -> Vec<FileChange> {
    let mut by_path = BTreeMap::new();
    for _ in 0..rng.random_range(0..6_u8) {
        let path = repo_path(rng);
        by_path.insert(path.as_str().to_owned(), file_change(rng, path));
    }
    by_path.into_values().collect()
}

/// Every record of one team, drawn from the seed: the team with its plan, its rounds, and for each of several runs its
/// start, its launch, its steps, its end and its changes. The records are in an order the database accepts.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct World {
    /// The team.
    pub(crate) team: TeamStart,
    /// Its rounds.
    pub(crate) rounds: Vec<RoundOpening>,
    /// Its runs.
    pub(crate) runs: Vec<RunWorld>,
}

/// Everything about one run.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct RunWorld {
    /// How it started.
    pub(crate) start: RunStart,
    /// How its process started.
    pub(crate) launch: Option<Launch>,
    /// Its steps, numbered 1, 2, 3, ...
    pub(crate) steps: Vec<Step>,
    /// How it ended.
    pub(crate) end: Option<RunEndRecord>,
    /// What it changed.
    pub(crate) changes: Option<RunChanges>,
}

impl World {
    /// The records, in the order a daemon hands them in.
    pub(crate) fn records(&self) -> Vec<Record> {
        let mut records = vec![Record::Team(Box::new(self.team.clone()))];
        records.extend(self.rounds.iter().map(|round| Record::Round(Box::new(round.clone()))));
        for run in &self.runs {
            records.push(Record::Run(Box::new(run.start.clone())));
            records.extend(run.launch.iter().map(|launch| Record::Launch(Box::new(launch.clone()))));
            records.extend(run.steps.iter().map(|step| Record::Step(Box::new(step.clone()))));
            records.extend(run.end.iter().map(|end| Record::RunEnd(Box::new(end.clone()))));
            records.extend(
                run.changes
                    .iter()
                    .map(|changes| Record::Changes(Box::new(changes.clone()))),
            );
        }
        records
    }
}

/// The world of `seed`.
pub(crate) fn world(seed: u64) -> World {
    let mut rng = Xoshiro256PlusPlus::seed_from_u64(seed);
    let clock = Still(1_795_000_000);
    let team_id = TeamId::generate(&clock, &mut rng).unwrap();
    let base = rng.random_range(0..1000_i64);
    let people = rng.random_range(1..5_u32);
    let plan: Vec<PlanEntry> = (1..=people)
        .map(|position| {
            let agent = AgentName::from_str(&format!("agent-{position}")).unwrap();
            let (role, how) = match (people, position) {
                (1, _) => (Role::Solo, HowChosen::Named),
                (_, 1) => match rng.random_range(0..3_u8) {
                    0 => (Role::Coordinator, HowChosen::NamedByFlag),
                    1 => (Role::Coordinator, HowChosen::NamedByLine),
                    _ => (
                        Role::Coordinator,
                        HowChosen::Drawn {
                            among: vec![agent.clone(), agent_name(&mut rng)],
                        },
                    ),
                },
                (_, 2) => (Role::Member, HowChosen::Named),
                _ => (Role::Worker, HowChosen::ConfiguredOrder),
            };
            PlanEntry {
                agent,
                position: NonZeroU32::new(position).unwrap(),
                role,
                how,
            }
        })
        .collect();
    let team = TeamStart {
        team: team_id.clone(),
        at: at(base, rng.random_range(0..1_000_000_000)),
        repo: format!("repo-{}", rng.random_range(0..9_u8)),
        base_branch: BranchName::from_str("main").unwrap(),
        merge: if rng.random_bool(0.5) {
            MergeMode::Autopilot
        } else {
            MergeMode::NoMerge
        },
        task_list: text(&mut rng, 300),
        plan,
    };
    let mut rounds = Vec::new();
    for number in 0..rng.random_range(0..4_u32) {
        let round = RoundId::generate(&Still(1_795_000_000 + i64::from(number)), &mut rng).unwrap();
        let subject = if rng.random_bool(0.7) {
            RoundSubject::Item(ItemText {
                item: ItemId::from_str(&format!("{}.{}", 1 + number, 1 + rng.random_range(0..3_u8))).unwrap(),
                title: some_text(&mut rng, 40),
                description: text(&mut rng, 200),
                expected_result: rng.random_bool(0.5).then(|| text(&mut rng, 100)),
                checks: rng.random_bool(0.5).then(|| text(&mut rng, 100)),
            })
        } else {
            RoundSubject::Brief(some_text(&mut rng, 100))
        };
        rounds.push(RoundOpening {
            round,
            team: team_id.clone(),
            at: at(base + i64::from(number), rng.random_range(0..1_000_000_000)),
            subject,
        });
    }
    let mut runs = Vec::new();
    for number in 0..rng.random_range(1..5_u32) {
        runs.push(run_world(&mut rng, &team_id, &rounds, base, number));
    }
    World { team, rounds, runs }
}

/// One run of the world, drawn.
fn run_world(rng: &mut impl Rng, team: &TeamId, rounds: &[RoundOpening], base: i64, number: u32) -> RunWorld {
    let begun = base + 10 + i64::from(number) * 100;
    let run = RunId::generate(&Still(begun), rng).unwrap();
    let tier = [Tier::Normal, Tier::Worker, Tier::Investigator][rng.random_range(0..3)];
    let cli = CliKind::ALL[rng.random_range(0..CliKind::ALL.len())];
    let role = [
        Role::Solo,
        Role::Coordinator,
        Role::Member,
        Role::Worker,
        Role::Reviewer,
    ][rng.random_range(0..5)];
    let worked_for: Vec<RoundId> = rounds
        .iter()
        .filter(|_| rng.random_bool(0.6))
        .map(|round| round.round.clone())
        .collect();
    let why = why_of(rng, begun);
    let worktree = format!("/home/you/agents/worktrees/example/{}", some_text_path(rng));
    let head_before = rng.random_bool(0.8).then(|| commit(rng));
    let start = RunStart {
        run: run.clone(),
        team: team.clone(),
        task: TaskId::generate(&Still(begun), rng).unwrap(),
        at: at(begun, rng.random_range(0..1_000_000_000)),
        agent: AgentIdentity {
            name: format!("agent-{}", rng.random_range(0..9_u8)),
            tier,
            cli,
            model: rng.random_bool(0.5).then(|| ModelName::from_str("sonnet").unwrap()),
        },
        role,
        rounds: worked_for,
        why,
        prompt: text(rng, 400),
        worktree: AbsolutePath::from_str(&worktree).unwrap(),
        branch: BranchName::from_str("agents/agent-1/task-1").unwrap(),
        session: rng
            .random_bool(0.5)
            .then(|| SessionId::from_str("0b6f2f56-6e83-4f68-a9d8-5a1bb43f6a11").unwrap()),
    };
    let launch = rng.random_bool(0.85).then(|| Launch {
        run: run.clone(),
        at: at(begun, rng.random_range(0..1_000_000_000)),
        program: "/usr/bin/claude".to_owned(),
        args: (0..rng.random_range(0..6_u8)).map(|_| text(rng, 60)).collect(),
        working_dir: AbsolutePath::from_str(&worktree).unwrap(),
        head: head_before.clone(),
        env_names: ["AGENTS_HOME", "PATH"]
            .iter()
            .filter(|_| rng.random_bool(0.8))
            .map(|name| EnvName::from_str(name).unwrap())
            .collect(),
        prompt_in_file: rng.random_bool(0.2),
    });
    let steps: Vec<Step> = (1..=rng.random_range(0..12_u32))
        .map(|n| Step {
            run: run.clone(),
            number: NonZeroU32::new(n).unwrap(),
            at: at(begun + i64::from(n), rng.random_range(0..1_000_000_000)),
            event: event(rng),
        })
        .collect();
    let end = rng.random_bool(0.85).then(|| RunEndRecord {
        run: run.clone(),
        at: at(begun + 50, rng.random_range(0..1_000_000_000)),
        end: run_end(rng, begun),
        final_message: rng.random_bool(0.8).then(|| text(rng, 300)),
    });
    let changes = rng.random_bool(0.7).then(|| changes_of(rng, &run, head_before, begun));
    RunWorld {
        start,
        launch,
        steps,
        end,
        changes,
    }
}

/// A reason a run starts, of any kind.
fn why_of(rng: &mut impl Rng, begun: i64) -> RunWhy {
    match rng.random_range(0..12_u8) {
        0 => RunWhy::FirstRun,
        1 => RunWhy::Item {
            item: ItemId::from_str("2.3").unwrap(),
        },
        2 => RunWhy::WakeUp,
        3 => RunWhy::Request {
            asked_by: rng
                .random_bool(0.5)
                .then(|| RunId::generate(&Still(begun - 5), rng).unwrap()),
        },
        4 => RunWhy::FixPass {
            number: NonZeroU32::new(rng.random_range(1..6)).unwrap(),
        },
        5 => RunWhy::Review {
            pr: PrNumber::new(NonZeroU32::new(rng.random_range(1..500)).unwrap()),
            pass: rng.random_range(0..6),
        },
        6 => RunWhy::AskHuman,
        7 => RunWhy::VerdictMissing,
        8 => RunWhy::Checks,
        9 => RunWhy::Answers,
        10 => RunWhy::ResumedByHuman,
        _ => RunWhy::Continuation,
    }
}

/// What a run changed, drawn.
fn changes_of(rng: &mut impl Rng, run: &RunId, head_before: Option<CommitId>, begun: i64) -> RunChanges {
    RunChanges {
        run: run.clone(),
        head_before,
        head_after: rng.random_bool(0.8).then(|| commit(rng)),
        commits: (1..=rng.random_range(0..4_u8))
            .map(|n| CommitRecord {
                id: commit(rng),
                subject: some_text(rng, 60),
                authored_at: at(begun + i64::from(n), 0),
            })
            .collect(),
        files: file_changes(rng),
        uncommitted: file_changes(rng),
    }
}

/// A directory name.
fn some_text_path(rng: &mut impl Rng) -> String {
    format!("task-{}", rng.random_range(0..1000_u32))
}

/// One step event of any kind.
pub(crate) fn event(rng: &mut impl Rng) -> ActivityEvent {
    match rng.random_range(0..8_u8) {
        0 => ActivityEvent::Text(text(rng, 120)),
        1 => ActivityEvent::Command(some_text(rng, 80)),
        2 => ActivityEvent::FileEdit {
            path: some_text(rng, 40),
        },
        3 => ActivityEvent::BusCall {
            tool: some_text(rng, 20),
            arguments: text(rng, 60),
        },
        4 => ActivityEvent::ToolCall {
            name: some_text(rng, 20),
            summary: text(rng, 60),
        },
        5 => ActivityEvent::Error(text(rng, 100)),
        6 => ActivityEvent::Info(text(rng, 100)),
        _ => ActivityEvent::Control {
            action: [
                ControlAction::Stop,
                ControlAction::Pause,
                ControlAction::Unpause,
                ControlAction::Resume,
                ControlAction::Say,
            ][rng.random_range(0..5)],
            by: if rng.random_bool(0.5) {
                Client::Cli
            } else {
                Client::Dashboard {
                    login: TailscaleLogin::from_str("you@example.com").unwrap(),
                }
            },
        },
    }
}

/// One way a run ends.
fn run_end(rng: &mut impl Rng, begun: i64) -> RunEnd {
    match rng.random_range(0..6_u8) {
        0 => RunEnd::Reported {
            outcome: [
                RunOutcome::Done,
                RunOutcome::Questions,
                RunOutcome::Failed,
                RunOutcome::Sleeping,
                RunOutcome::Excluded,
            ][rng.random_range(0..5)],
        },
        1 => RunEnd::NoStatus {
            exit_code: rng.random_bool(0.7).then(|| rng.random_range(-3..130)),
        },
        2 => RunEnd::QuotaExhausted {
            resets_at: rng.random_bool(0.5).then(|| at(begun + 3600, 0)),
        },
        3 => RunEnd::RateLimited,
        4 => RunEnd::Hung,
        _ => RunEnd::Stopped,
    }
}

/// The world of `seed` whose first run has all that a run can have: a launch, three steps, an end, and changes with a
/// commit and a renamed file, and whose team has a round that the run works for.
pub(crate) fn complete(seed: u64) -> World {
    let mut world = world(seed);
    if world.rounds.is_empty() {
        let round = RoundId::generate(&Still(1_795_000_100), &mut Xoshiro256PlusPlus::seed_from_u64(seed)).unwrap();
        world.rounds.push(RoundOpening {
            round,
            team: world.team.team.clone(),
            at: at(5, 0),
            subject: RoundSubject::Brief("the whole list".to_owned()),
        });
    }
    let round = world.rounds[0].round.clone();
    let run = &mut world.runs[0];
    run.start.rounds = vec![round];
    run.start.why = RunWhy::Item {
        item: ItemId::from_str("1.1").unwrap(),
    };
    run.launch = Some(Launch {
        run: run.start.run.clone(),
        at: run.start.at,
        program: "claude".to_owned(),
        args: vec!["-p".to_owned(), "do it".to_owned()],
        working_dir: run.start.worktree.clone(),
        head: None,
        env_names: vec![EnvName::from_str("PATH").unwrap()],
        prompt_in_file: false,
    });
    run.steps = (1..=3)
        .map(|n| Step {
            run: run.start.run.clone(),
            number: NonZeroU32::new(n).unwrap(),
            at: run.start.at,
            event: ActivityEvent::Text(format!("step {n}")),
        })
        .collect();
    run.end = Some(RunEndRecord {
        run: run.start.run.clone(),
        at: at(1000, 0),
        end: RunEnd::Reported {
            outcome: RunOutcome::Done,
        },
        final_message: Some("done".to_owned()),
    });
    run.changes = Some(RunChanges {
        run: run.start.run.clone(),
        head_before: None,
        head_after: None,
        commits: vec![CommitRecord {
            id: commit(&mut Xoshiro256PlusPlus::seed_from_u64(seed)),
            subject: "a commit".to_owned(),
            authored_at: at(10, 0),
        }],
        files: vec![FileChange {
            path: RepoPath::from_str("src/b.rs").unwrap(),
            change: Change::Renamed {
                from: RepoPath::from_str("src/a.rs").unwrap(),
            },
            lines: Lines::Text { added: 2, removed: 1 },
        }],
        uncommitted: Vec::new(),
    });
    world
}
