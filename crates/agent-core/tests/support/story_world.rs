//! Histories for the tests of the story: a builder that hands out places in the order of writes, and one of every kind of
//! record, with the values a test reads back in the text.

use std::num::NonZeroU32;

use agent_core::cli::ActivityEvent;
use agent_core::decide::checks::{Change, FileChange, Lines};
use agent_core::history::{
    AgentIdentity, CheckOutcome, CommitRecord, Cut, EventRecord, HistoryEvent, HowChosen, ItemText, Launch, PlanEntry,
    Record, RoundOpening, RoundSubject, RunChanges, RunEndRecord, RunStart, RunWhy, SequencedRecord, Step, TeamStart,
};
use agent_core::ids::{AgentName, BranchName, ItemId, RoundId, RunId, TaskId, TeamId};
use agent_core::kinds::{CliKind, Role, Tier};
use agent_core::repo_path::RepoPath;
use agent_core::state::{CommitId, MergeMode, PrNumber, RunEnd, Timestamp};
use agent_core::status::RunOutcome;

/// A commit of forty copies of `digit`.
pub(crate) fn head(digit: char) -> CommitId {
    digit.to_string().repeat(40).parse().unwrap()
}

/// The pull request `n`.
pub(crate) fn pr(n: u32) -> PrNumber {
    PrNumber::new(NonZeroU32::new(n).unwrap())
}

/// The non-zero number `n`.
pub(crate) fn number(n: u32) -> NonZeroU32 {
    NonZeroU32::new(n).unwrap()
}

/// `seconds` after 19:30:00 UTC on 2026-11-11.
pub(crate) fn at(seconds: u32) -> Timestamp {
    let start: Timestamp = "2026-11-11T19:30:00Z".parse().unwrap();
    Timestamp::from_utc(start.as_utc() + time::Duration::seconds(i64::from(seconds))).unwrap()
}

/// The team `n`.
pub(crate) fn team(n: u32) -> TeamId {
    format!("team-20261111-1930{n:02}-t5m8").parse().unwrap()
}

/// The round `n`.
pub(crate) fn round(n: u32) -> RoundId {
    format!("round-20261111-1930{n:02}-q1w2").parse().unwrap()
}

/// The run `n`.
pub(crate) fn run(n: u32) -> RunId {
    format!("run-20261111-1930{n:02}-r7q2").parse().unwrap()
}

/// The task `n`.
pub(crate) fn task(n: u32) -> TaskId {
    format!("task-20261111-1930{n:02}-k3x9").parse().unwrap()
}

/// Records with the place each was written in, handed out in order from 1.
#[derive(Debug, Default)]
pub(crate) struct Written {
    /// The records, in the order they were written.
    pub(crate) records: Vec<SequencedRecord>,
}

impl Written {
    /// Writes `record` of `team` at `seconds`, for `rounds`, with `cut` fields cut.
    pub(crate) fn add(&mut self, team: &TeamId, seconds: u32, rounds: &[RoundId], record: Record, cut: Cut) {
        let sequence = u64::try_from(self.records.len()).unwrap() + 1;
        self.records.push(SequencedRecord {
            sequence,
            at: at(seconds),
            team: team.clone(),
            rounds: rounds.to_vec(),
            record,
            cut,
        });
    }

    /// Writes `event` of `team` at `seconds` for `rounds`.
    pub(crate) fn event(&mut self, team: &TeamId, seconds: u32, rounds: &[RoundId], event: HistoryEvent) {
        let record = Record::Event(Box::new(EventRecord {
            at: at(seconds),
            team: team.clone(),
            rounds: rounds.to_vec(),
            event,
        }));
        self.add(team, seconds, rounds, record, Cut::default());
    }

    /// Writes the start of `team` with a plan of a coordinator drawn at random and a member.
    pub(crate) fn team(&mut self, team: &TeamId, seconds: u32) {
        let agent = |name: &str| -> AgentName { name.parse().unwrap() };
        let start = TeamStart {
            team: team.clone(),
            at: at(seconds),
            repo: "demo".to_owned(),
            base_branch: "main".parse().unwrap(),
            merge: MergeMode::Autopilot,
            task_list: "Build.\n\n1. One\n[ ] 1.1 First\n    Do it.\n".to_owned(),
            plan: vec![
                PlanEntry {
                    agent: agent("inv-a"),
                    position: number(1),
                    role: Role::Coordinator,
                    how: HowChosen::Drawn {
                        among: vec![agent("inv-a"), agent("inv-b")],
                    },
                },
                PlanEntry {
                    agent: agent("dev-1"),
                    position: number(2),
                    role: Role::Member,
                    how: HowChosen::Named,
                },
            ],
        };
        self.add(team, seconds, &[], Record::Team(Box::new(start)), Cut::default());
    }

    /// Writes the opening of `round` of `team` for the item 1.1.
    pub(crate) fn round(&mut self, team: &TeamId, round: &RoundId, seconds: u32) {
        let opening = RoundOpening {
            round: round.clone(),
            team: team.clone(),
            at: at(seconds),
            subject: RoundSubject::Item(ItemText {
                item: "1.1".parse::<ItemId>().unwrap(),
                title: "The first item — zażółć".to_owned(),
                description: "Do the thing.\nThen the other thing.".to_owned(),
                expected_result: Some("The thing is done.".to_owned()),
                checks: Some("Run it.".to_owned()),
            }),
        };
        self.add(
            team,
            seconds,
            std::slice::from_ref(round),
            Record::Round(Box::new(opening)),
            Cut::default(),
        );
    }

    /// Writes a whole run of `team` for `rounds`: its start at `seconds`, a launch, `steps` steps, an end `took` seconds
    /// later and changes; `prompt` is its prompt.
    pub(crate) fn run(&mut self, team: &TeamId, rounds: &[RoundId], n: u32, seconds: u32, prompt: &str, steps: u32) {
        let id = run(n);
        let start = RunStart {
            run: id.clone(),
            team: team.clone(),
            task: task(n),
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
            prompt: prompt.to_owned(),
            worktree: "/work/tree".parse().unwrap(),
            branch: BranchName::try_from("agents/dev-1/1".to_owned()).unwrap(),
            session: None,
        };
        self.add(team, seconds, rounds, Record::Run(Box::new(start)), Cut::default());
        let launch = Launch {
            run: id.clone(),
            at: at(seconds + 1),
            program: "claude".to_owned(),
            args: vec!["-p".to_owned(), prompt.to_owned(), "--verbose".to_owned()],
            working_dir: "/work/tree".parse().unwrap(),
            head: Some(head('1')),
            env_names: Vec::new(),
            prompt_in_file: false,
        };
        self.add(
            team,
            seconds + 1,
            rounds,
            Record::Launch(Box::new(launch)),
            Cut::default(),
        );
        for index in 1..=steps {
            let step = Step {
                run: id.clone(),
                number: number(index),
                at: at(seconds + 1 + index),
                event: ActivityEvent::Text(format!("step {index} of the run")),
            };
            self.add(
                team,
                seconds + 1 + index,
                rounds,
                Record::Step(Box::new(step)),
                Cut::default(),
            );
        }
        let ended = seconds + 10 + steps;
        let end = RunEndRecord {
            run: id.clone(),
            at: at(ended),
            end: RunEnd::Reported {
                outcome: RunOutcome::Done,
            },
            final_message: Some("All done.\nSTATUS: DONE".to_owned()),
        };
        self.add(team, ended, rounds, Record::RunEnd(Box::new(end)), Cut::default());
        let changes = RunChanges {
            run: id,
            head_before: Some(head('1')),
            head_after: Some(head('2')),
            commits: vec![CommitRecord {
                id: head('2'),
                subject: "Add the thing".to_owned(),
                authored_at: at(ended),
            }],
            files: vec![
                FileChange {
                    path: "src/new.rs".parse::<RepoPath>().unwrap(),
                    change: Change::Added,
                    lines: Lines::Text { added: 10, removed: 0 },
                },
                FileChange {
                    path: "src/moved.rs".parse::<RepoPath>().unwrap(),
                    change: Change::Renamed {
                        from: "src/old.rs".parse().unwrap(),
                    },
                    lines: Lines::Text { added: 2, removed: 1 },
                },
                FileChange {
                    path: "logo.png".parse::<RepoPath>().unwrap(),
                    change: Change::Modified,
                    lines: Lines::Binary,
                },
            ],
            uncommitted: vec![FileChange {
                path: "notes.txt".parse::<RepoPath>().unwrap(),
                change: Change::Deleted,
                lines: Lines::Text { added: 0, removed: 3 },
            }],
        };
        self.add(team, ended, rounds, Record::Changes(Box::new(changes)), Cut::default());
    }

    /// A pull request's life for `rounds` from `seconds` on: opened, pushed, ready, a first review pass that asks for
    /// changes, a fix pass, and a second pass that approves, then the merge and the end of the round and the team.
    pub(crate) fn pull_request(&mut self, team: &TeamId, rounds: &[RoundId], seconds: u32) {
        let reviewer = |name: &str| name.parse().unwrap();
        let events = [
            HistoryEvent::PullOpened {
                number: pr(7),
                branch: BranchName::try_from("agents/dev-1/1".to_owned()).unwrap(),
                head: head('2'),
                title: "Add the thing".to_owned(),
                body: "The body of the pull request.\nSecond line — 日本語.".to_owned(),
            },
            HistoryEvent::PullReady { pr: pr(7) },
            HistoryEvent::ReviewAsked {
                pr: pr(7),
                pass: 0,
                head: head('2'),
                reviewer: reviewer("inv-c"),
                task: task(20),
            },
            HistoryEvent::ReviewSubmitted {
                pr: pr(7),
                pass: 0,
                reviewer: reviewer("inv-c"),
                task: task(20),
                verdict: agent_core::status::ReviewVerdict::RequestChanges,
                report: "Blocker: no test.\nVERDICT: REQUEST_CHANGES".to_owned(),
            },
            HistoryEvent::FixPassStarted {
                pr: pr(7),
                pass: number(1),
                of: 5,
                findings: "Blocker: no test.".to_owned(),
            },
            HistoryEvent::PullPushed {
                pr: pr(7),
                head: head('3'),
            },
            HistoryEvent::ReviewAsked {
                pr: pr(7),
                pass: 1,
                head: head('3'),
                reviewer: reviewer("inv-c"),
                task: task(21),
            },
            HistoryEvent::ReviewSubmitted {
                pr: pr(7),
                pass: 1,
                reviewer: reviewer("inv-c"),
                task: task(21),
                verdict: agent_core::status::ReviewVerdict::Approve,
                report: "No findings.\nVERDICT: APPROVE".to_owned(),
            },
            HistoryEvent::PullApproved { pr: pr(7) },
            HistoryEvent::PullMerged {
                pr: pr(7),
                commit: Some(head('4')),
            },
            HistoryEvent::RoundClosed {
                round: rounds[0].clone(),
                end: agent_core::state::RoundEnd::Merged,
            },
            HistoryEvent::TeamEnded {},
        ];
        for (index, event) in events.into_iter().enumerate() {
            let when = seconds + u32::try_from(index).unwrap();
            let linked = if matches!(event, HistoryEvent::TeamEnded {}) {
                &[][..]
            } else {
                rounds
            };
            self.event(team, when, linked, event);
        }
    }

    /// The checks before the first review found an issue and sent the round back.
    pub(crate) fn checked(&mut self, team: &TeamId, rounds: &[RoundId], seconds: u32) {
        self.event(
            team,
            seconds,
            rounds,
            HistoryEvent::Checked {
                pr: Some(pr(7)),
                head: head('2'),
                issues: vec!["the round changes production code and adds no test".to_owned()],
                outcome: CheckOutcome::SentBack { pass: number(1), of: 2 },
            },
        );
    }
}
