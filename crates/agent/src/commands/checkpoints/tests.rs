//! `agent checkpoints` on a history written through the writer the daemon uses, and the prompts of the repository.

use std::ffi::OsString;
use std::num::NonZeroU32;
use std::path::Path;
use std::process::ExitCode;

use agent_core::history::{
    AgentIdentity, CheckpointCause, CheckpointContent, CheckpointJob, EventRecord, HistoryEvent, HowChosen, ItemText,
    MaxTextBytes, PlanEntry, Record, RoundOpening, RoundSubject, RunStart, RunWhy, TeamStart,
};
use agent_core::ids::{RoundId, TaskId, TeamId};
use agent_core::kinds::{CliKind, Role, Tier};
use agent_core::state::{CommitId, MergeMode, PhaseKind, PrNumber, Timestamp};
use agent_runtime::history::{HistoryDb, Store};

use crate::{Environment, run_in};

fn at(seconds: i64) -> Timestamp {
    format!("2026-11-27T04:{:02}:{:02}Z", seconds / 60, seconds % 60)
        .parse()
        .unwrap()
}

fn team() -> TeamId {
    "team-20261127-040000-t5m8".parse().unwrap()
}

fn round() -> RoundId {
    "round-20261127-040001-q1w2".parse().unwrap()
}

fn task() -> TaskId {
    "task-20261127-040002-k3x9".parse().unwrap()
}

fn checkpoint(seconds: i64, cause: CheckpointCause, pull: bool, content: Option<CheckpointContent>) -> Record {
    let head: CommitId = format!("{}{}", "0123abc", "0".repeat(33)).parse().unwrap();
    Record::Event(Box::new(EventRecord {
        at: at(seconds),
        team: team(),
        rounds: vec![round()],
        event: HistoryEvent::Checkpoint {
            round: round(),
            item: Some("1.1".parse().unwrap()),
            task: task(),
            job: CheckpointJob::Item,
            tier: Tier::Normal,
            cause,
            pull_request: pull.then(|| PrNumber::new(NonZeroU32::new(7).unwrap())),
            head: Some(head),
            draft: pull,
            phase: PhaseKind::InProgress,
            review_pass: None,
            fix_pass: None,
            snapshot: Some("refs/agents/snapshots/round-20261127-040001-q1w2/task-20261127-040002-k3x9/c1".to_owned()),
            content,
        },
    }))
}

fn content() -> CheckpointContent {
    CheckpointContent {
        meant: "Task: Build.\nItem 1.1: Item\nR1. works".to_owned(),
        done: "Totals: 2 commits, 1 steps\nR1 met — steps 1.1 (declared); commits abc1234".to_owned(),
        left: "Totals: 3 steps, 1 findings\nnext: wait for the merge".to_owned(),
    }
}

/// A home with a history of one round of one task with three checkpoints: the first has no pull request and no content,
/// the others have a draft and a content.
fn home() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let mut db = HistoryDb::open(&dir.path().join("history.db"), MaxTextBytes::default()).unwrap();
    let records = vec![
        Record::Team(Box::new(TeamStart {
            team: team(),
            at: at(0),
            repo: "demo".to_owned(),
            base_branch: "main".parse().unwrap(),
            merge: MergeMode::Autopilot,
            task_list: "Build.\n".to_owned(),
            plan: vec![PlanEntry {
                agent: "dev-1".parse().unwrap(),
                position: NonZeroU32::new(1).unwrap(),
                role: Role::Solo,
                how: HowChosen::Named,
            }],
        })),
        Record::Round(Box::new(RoundOpening {
            round: round(),
            team: team(),
            at: at(1),
            subject: RoundSubject::Item(ItemText {
                item: "1.1".parse().unwrap(),
                title: "Item".to_owned(),
                description: "Do it.".to_owned(),
                expected_result: None,
                checks: None,
            }),
        })),
        Record::Run(Box::new(RunStart {
            run: "run-20261127-040002-r7q2".parse().unwrap(),
            team: team(),
            task: task(),
            at: at(2),
            agent: AgentIdentity {
                name: "dev-1".to_owned(),
                tier: Tier::Normal,
                cli: CliKind::Claude,
                model: None,
            },
            role: Role::Solo,
            rounds: vec![round()],
            why: RunWhy::Item {
                item: "1.1".parse().unwrap(),
            },
            prompt: "Do it.".to_owned(),
            worktree: "/work/tree".parse().unwrap(),
            branch: "agents/dev-1/1".parse().unwrap(),
            session: None,
        })),
        checkpoint(10, CheckpointCause::RoundOpened, false, None),
        checkpoint(20, CheckpointCause::Commit, true, Some(content())),
        checkpoint(30, CheckpointCause::PullReady, true, Some(content())),
    ];
    db.write(&records).unwrap();
    dir
}

/// Runs `agent checkpoints` with `args` against `home` and its repository prompts; returns the status, stdout and stderr.
fn run(home: &Path, args: &[&str]) -> (ExitCode, String, String) {
    let prompts = home.join("prompts");
    if !prompts.exists() {
        std::os::unix::fs::symlink(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../prompts"), &prompts).unwrap();
    }
    let environment = Environment::new(Some(OsString::from(home)));
    let (mut out, mut err) = (Vec::new(), Vec::new());
    let status = run_in(
        &environment,
        ["agent", "checkpoints"].into_iter().chain(args.iter().copied()),
        &mut out,
        &mut err,
    )
    .unwrap();
    (status, String::from_utf8(out).unwrap(), String::from_utf8(err).unwrap())
}

const SNAPSHOT: &str = "refs/agents/snapshots/round-20261127-040001-q1w2/task-20261127-040002-k3x9/c1";

// Rule: one line a checkpoint, oldest first, with the pull request, head, pass, what is done and left, and the snapshot.
#[test]
fn the_listing_of_a_round_has_a_line_for_each_checkpoint_oldest_first() {
    let dir = home();

    let (status, out, err) = run(dir.path(), &["round-20261127-040001-q1w2"]);

    assert_eq!((status, err.as_str()), (ExitCode::SUCCESS, ""));
    assert_eq!(
        out,
        format!(
            "2026-11-27T04:00:10Z  round_opened  PR none  head 0123abc  pass 0  done n/a  left n/a  snapshot {SNAPSHOT}\n\
             2026-11-27T04:00:20Z  commit  PR #7 draft  head 0123abc  pass 0  done 2 commits, 1 steps  left 3 steps, 1 findings  snapshot {SNAPSHOT}\n\
             2026-11-27T04:00:30Z  pull_ready  PR #7 draft  head 0123abc  pass 0  done 2 commits, 1 steps  left 3 steps, 1 findings  snapshot {SNAPSHOT}\n"
        )
    );
}

// Rule: a task is told as a round is, over its rounds.
#[test]
fn the_listing_of_a_task_is_the_listing_of_its_checkpoints() {
    let dir = home();

    let (_, by_round, _) = run(dir.path(), &["round-20261127-040001-q1w2"]);
    let (status, by_task, _) = run(dir.path(), &["task-20261127-040002-k3x9"]);

    assert_eq!(status, ExitCode::SUCCESS);
    assert_eq!(by_task.lines().count(), 3);
    assert_eq!(by_task, by_round);
}

// Rule: `--show N` prints checkpoint N, the oldest being 1, with the text of `checkpoint-state.md`; without N the last.
#[test]
fn show_prints_the_numbered_checkpoint_and_without_a_number_the_last_one() {
    let dir = home();
    let want = |cause: &str, time: &str| {
        format!(
            "## Checkpoint: {cause} at {time}, head 0123abc000000000000000000000000000000000\n\n\
             ### What is to be done\nTask: Build.\nItem 1.1: Item\nR1. works\n\n\
             ### What is done\nTotals: 2 commits, 1 steps\nR1 met — steps 1.1 (declared); commits abc1234\n\n\
             ### What is left\nTotals: 3 steps, 1 findings\nnext: wait for the merge\n"
        )
    };

    let (status, second, err) = run(dir.path(), &["round-20261127-040001-q1w2", "--show", "2"]);
    let (_, last, _) = run(dir.path(), &["round-20261127-040001-q1w2", "--show"]);

    assert_eq!((status, err.as_str()), (ExitCode::SUCCESS, ""));
    assert_eq!(second, want("commit", "2026-11-27T04:00:20Z"));
    assert_eq!(last, want("pull_ready", "2026-11-27T04:00:30Z"));
}

// Rule: a checkpoint number that is not there is refused with what there is.
#[test]
fn show_of_a_number_beyond_the_checkpoints_is_refused() {
    let dir = home();

    let (status, out, err) = run(dir.path(), &["round-20261127-040001-q1w2", "--show", "4"]);

    assert_eq!((status, out.as_str()), (ExitCode::FAILURE, ""));
    assert!(err.contains("there is no checkpoint 4: there are 3"), "{err}");
}

// Rule: `--json` prints the checkpoint records, one object a line.
#[test]
fn json_prints_the_checkpoint_records_one_object_a_line() {
    let dir = home();

    let (status, out, _) = run(dir.path(), &["round-20261127-040001-q1w2", "--json"]);

    assert_eq!(status, ExitCode::SUCCESS);
    let values: Vec<serde_json::Value> = out.lines().map(|line| serde_json::from_str(line).unwrap()).collect();
    let causes: Vec<&str> = values
        .iter()
        .map(|value| value["record"]["cause"].as_str().unwrap())
        .collect();
    assert_eq!(causes, ["round_opened", "commit", "pull_ready"]);
    assert!(
        values.iter().all(|value| value["record"]["event"] == "checkpoint"),
        "{values:?}"
    );
}

// Rule: a round with no checkpoint says so and succeeds; one the history does not know exits as `agent log` does.
#[test]
fn a_round_with_no_checkpoint_says_so_and_an_unknown_target_exits_three() {
    let dir = home();
    let mut db = HistoryDb::open(&dir.path().join("history.db"), MaxTextBytes::default()).unwrap();
    let quiet: RoundId = "round-20261127-040100-q1w2".parse().unwrap();
    db.write(&[Record::Round(Box::new(RoundOpening {
        round: quiet,
        team: team(),
        at: at(61),
        subject: RoundSubject::Item(ItemText {
            item: "1.2".parse().unwrap(),
            title: "Second".to_owned(),
            description: "Do it.".to_owned(),
            expected_result: None,
            checks: None,
        }),
    }))])
    .unwrap();
    drop(db);

    let (status, out, _) = run(dir.path(), &["round-20261127-040100-q1w2"]);
    assert_eq!(
        (status, out.as_str()),
        (ExitCode::SUCCESS, "No checkpoints for round-20261127-040100-q1w2.\n")
    );
    for unknown in ["round-20261127-045959-zzzz", "task-20261127-045959-zzzz"] {
        let (status, out, err) = run(dir.path(), &[unknown]);
        assert_eq!((status, out.as_str()), (ExitCode::from(3), ""), "{unknown}");
        assert!(err.contains("the history has no"), "{err}");
    }
}

// Rule: a word that is no round or task is a mistake of the command line.
#[test]
fn a_word_that_is_no_round_or_task_is_a_usage_error() {
    let dir = home();

    let (status, out, err) = run(dir.path(), &["banana"]);

    assert_eq!((status, out.as_str()), (ExitCode::from(2), ""));
    assert!(err.contains("banana"), "{err}");
}

// Rule: a pull request that is not a draft is listed as ready.
#[test]
fn a_pull_request_that_is_not_a_draft_is_listed_as_ready() {
    let dir = home();
    let mut db = HistoryDb::open(&dir.path().join("history.db"), MaxTextBytes::default()).unwrap();
    let Record::Event(mut ready) = checkpoint(40, CheckpointCause::PullApproved, true, Some(content())) else {
        panic!("a checkpoint is an event")
    };
    let HistoryEvent::Checkpoint { draft, .. } = &mut ready.event else {
        panic!("a checkpoint is a checkpoint")
    };
    *draft = false;
    db.write(&[Record::Event(ready)]).unwrap();
    drop(db);

    let (status, out, _) = run(dir.path(), &["round-20261127-040001-q1w2"]);

    assert_eq!(status, ExitCode::SUCCESS);
    assert_eq!(
        out.lines().last().unwrap(),
        format!(
            "2026-11-27T04:00:40Z  pull_approved  PR #7 ready  head 0123abc  pass 0  done 2 commits, 1 steps  left 3 steps, 1 findings  snapshot {SNAPSHOT}"
        )
    );
}

// Rule: `--show 1` prints the oldest checkpoint; one that kept no account of the work says so in each part.
#[test]
fn show_one_prints_the_oldest_checkpoint_and_says_when_it_kept_no_account() {
    let dir = home();

    let (status, out, err) = run(dir.path(), &["round-20261127-040001-q1w2", "--show", "1"]);

    assert_eq!((status, err.as_str()), (ExitCode::SUCCESS, ""));
    assert_eq!(
        out,
        "## Checkpoint: round_opened at 2026-11-27T04:00:10Z, head 0123abc000000000000000000000000000000000\n\n\
         ### What is to be done\n(this checkpoint kept no account of the work)\n\n\
         ### What is done\n(this checkpoint kept no account of the work)\n\n\
         ### What is left\n(this checkpoint kept no account of the work)\n"
    );
}

// Rule: with `--json` and no checkpoint, stdout stays empty (no JSON line) and the message goes to stderr, exit 0.
#[test]
fn json_with_no_checkpoint_prints_no_line_and_says_so_on_stderr() {
    let dir = home();
    let mut db = HistoryDb::open(&dir.path().join("history.db"), MaxTextBytes::default()).unwrap();
    let quiet: RoundId = "round-20261127-040100-q1w2".parse().unwrap();
    db.write(&[Record::Round(Box::new(RoundOpening {
        round: quiet,
        team: team(),
        at: at(61),
        subject: RoundSubject::Item(ItemText {
            item: "1.2".parse().unwrap(),
            title: "Second".to_owned(),
            description: "Do it.".to_owned(),
            expected_result: None,
            checks: None,
        }),
    }))])
    .unwrap();
    drop(db);

    let (status, out, err) = run(dir.path(), &["round-20261127-040100-q1w2", "--json"]);

    assert_eq!(
        (status, out.as_str(), err.as_str()),
        (
            ExitCode::SUCCESS,
            "",
            "No checkpoints for round-20261127-040100-q1w2.\n"
        )
    );
}
