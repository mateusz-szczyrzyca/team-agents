//! The feed of a follow: the opening, and each page of what was recorded since, every line once.

use std::collections::BTreeMap;
use std::num::NonZeroU32;
use std::path::Path;

use agent_core::cli::ActivityEvent;
use agent_core::history::{AgentIdentity, MaxTextBytes, Record, RunStart, RunWhy, Step, TeamStart};
use agent_core::ids::{RoundId, RunId, TaskId, TeamId};
use agent_core::kinds::{CliKind, Role, Tier};
use agent_core::log::{LogEvent, LogRecord, LogTaskScope, LogTeamScope};
use agent_core::state::Timestamp;
use agent_core::story::StepView;
use agent_runtime::history::{HistoryDb, LogFilter, Reader, Store};

use super::Feed;

/// When `second` is, after 12:50:00 UTC.
fn at(second: u32) -> Timestamp {
    format!("2026-11-21T12:50:{second:02}Z")
        .parse()
        .expect("a literal time is valid")
}

/// The team of the history.
fn team() -> TeamId {
    "team-20261121-132909-4qem".parse().expect("a literal id is valid")
}

/// The round of the history.
fn round() -> RoundId {
    "round-20261121-132909-q1w2".parse().expect("a literal id is valid")
}

/// The task numbered `number`.
fn task(number: u32) -> TaskId {
    format!("task-20261121-132909-{number:04}")
        .parse()
        .expect("a literal id is valid")
}

/// The run numbered `number`.
fn run(number: u32) -> RunId {
    format!("run-20261121-132909-{number:04}")
        .parse()
        .expect("a literal id is valid")
}

/// The start of run `number`, of `agent`, on the task numbered `on`.
fn start(number: u32, agent: &str, on: u32, second: u32) -> RunStart {
    RunStart {
        run: run(number),
        team: team(),
        task: task(on),
        at: at(second),
        agent: AgentIdentity {
            name: agent.to_owned(),
            tier: Tier::Normal,
            cli: CliKind::Claude,
            model: None,
        },
        role: Role::Solo,
        rounds: vec![round()],
        why: RunWhy::FirstRun,
        prompt: String::new(),
        worktree: "/home/example/worktrees/example/task"
            .parse()
            .expect("a literal path is valid"),
        branch: "agents/example/task".parse().expect("a literal branch is valid"),
        session: None,
    }
}

/// Step `number` of run `of`, a command `text` recorded at `second`.
fn step(of: u32, number: u32, second: u32, text: &str) -> Record {
    Record::Step(Box::new(Step {
        run: run(of),
        number: NonZeroU32::new(number).expect("a step number starts at one"),
        at: at(second),
        event: ActivityEvent::Command(text.to_owned()),
    }))
}

/// A row of `event` of run `of` of the task numbered `on`, recorded at `second`.
fn row(event: LogEvent, of: u32, on: u32, second: u32) -> LogRecord {
    LogRecord {
        at: at(second),
        module: "test".to_owned(),
        event,
        team: Some(LogTeamScope::new(team(), [round()])),
        task: Some(LogTaskScope::new(task(on), Some(run(of)))),
        message: "a row".to_owned(),
        fields: [("agent".to_owned(), serde_json::json!("claude-1"))]
            .into_iter()
            .collect::<BTreeMap<_, _>>(),
        cut: agent_core::history::Cut::default(),
    }
}

/// An empty history with the team, and its writer.
fn database(path: &Path) -> HistoryDb {
    let mut db = HistoryDb::open(path, MaxTextBytes::default()).expect("a temporary history opens");
    db.write(&[Record::Team(Box::new(TeamStart {
        team: team(),
        at: at(0),
        repo: "example".to_owned(),
        base_branch: "main".parse().expect("a literal branch is valid"),
        merge: agent_core::state::MergeMode::NoMerge,
        task_list: String::new(),
        plan: Vec::new(),
    }))])
    .expect("the team is written");
    db
}

/// The filter of the task numbered `number`.
fn of_task(number: u32) -> LogFilter {
    LogFilter {
        task: Some(task(number)),
        ..LogFilter::default()
    }
}

/// What no round has to wait for: the line the feed asks for when a run ends.
fn waits(round: &RoundId) -> String {
    format!("round {round} waits for the queued run of its task to start")
}

/// Opens a feed on `filter` in `path` with the newest `limit` lines and the text of its opening.
fn open(path: &Path, filter: LogFilter, limit: u32) -> (Feed, String) {
    let reader = Reader::open(path).expect("the history reads");
    Feed::open(&reader, filter, StepView::Listed, limit).expect("the feed opens")
}

/// The text of the next page of `feed`.
fn poll(feed: &mut Feed, path: &Path) -> String {
    let reader = Reader::open(path).expect("the history reads");
    feed.poll(&reader, &mut waits).expect("the page is read")
}

#[test]
fn the_opening_is_the_newest_lines_of_rows_and_steps_together_in_the_order_they_happened() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let path = dir.path().join("history.db");
    let mut db = database(&path);
    db.write(&[
        Record::Run(Box::new(start(1, "claude-1", 1, 1))),
        step(1, 1, 2, "cargo build"),
    ])
    .expect("the run is written");
    db.write_logs(&[row(LogEvent::TaskStatusChanged, 1, 1, 3)])
        .expect("the row is written");
    db.write(&[step(1, 2, 4, "cargo test")]).expect("the step is written");

    let (_feed, text) = open(&path, of_task(1), 2);

    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len(), 3, "{text}");
    assert!(
        lines[0].starts_with("2026-11-21T12:50:03Z info task_status_changed "),
        "{text}"
    );
    assert!(
        lines[1].contains(" run run-20261121-132909-0001 started by claude-1"),
        "{text}"
    );
    assert_eq!(lines[2], "2026-11-21T12:50:04Z command: cargo test", "{text}");
}

#[test]
fn the_opening_with_no_lines_prints_nothing_and_what_comes_is_printed() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let path = dir.path().join("history.db");
    let mut db = database(&path);
    db.write(&[
        Record::Run(Box::new(start(1, "claude-1", 1, 1))),
        step(1, 1, 2, "cargo build"),
    ])
    .expect("the run is written");

    let (mut feed, text) = open(&path, of_task(1), 0);
    db.write(&[step(1, 2, 3, "cargo test")]).expect("the step is written");
    let page = poll(&mut feed, &path);

    assert_eq!(text, "");
    let lines: Vec<&str> = page.lines().collect();
    assert_eq!(lines.len(), 2, "{page}");
    assert!(
        lines[0].contains(" run run-20261121-132909-0001 started by claude-1"),
        "{page}"
    );
    assert_eq!(lines[1], "2026-11-21T12:50:03Z command: cargo test");
}

#[test]
fn a_page_prints_each_step_once_and_the_run_only_before_its_first_step() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let path = dir.path().join("history.db");
    let mut db = database(&path);
    db.write(&[
        Record::Run(Box::new(start(1, "claude-1", 1, 1))),
        step(1, 1, 2, "cargo build"),
    ])
    .expect("the run is written");
    let (mut feed, opening) = open(&path, of_task(1), 10);
    assert_eq!(opening.lines().count(), 2, "{opening}");

    assert_eq!(poll(&mut feed, &path), "", "nothing was recorded since the opening");
    db.write(&[step(1, 2, 3, "cargo test")]).expect("the step is written");
    assert_eq!(poll(&mut feed, &path), "2026-11-21T12:50:03Z command: cargo test\n");
    assert_eq!(poll(&mut feed, &path), "", "the step is not printed again");
}

#[test]
fn a_row_recorded_with_an_earlier_time_than_a_printed_step_is_printed_once_and_the_step_is_not_repeated() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let path = dir.path().join("history.db");
    let mut db = database(&path);
    db.write(&[
        Record::Run(Box::new(start(1, "claude-1", 1, 1))),
        step(1, 1, 2, "cargo build"),
    ])
    .expect("the run is written");
    let (mut feed, _opening) = open(&path, of_task(1), 10);

    db.write(&[step(1, 2, 8, "cargo test")]).expect("the step is written");
    assert_eq!(poll(&mut feed, &path), "2026-11-21T12:50:08Z command: cargo test\n");
    db.write_logs(&[row(LogEvent::TaskStatusChanged, 1, 1, 7)])
        .expect("the row is written");
    let page = poll(&mut feed, &path);

    assert_eq!(page.lines().count(), 1, "{page}");
    assert!(
        page.starts_with("2026-11-21T12:50:07Z info task_status_changed "),
        "{page}"
    );
    assert_eq!(poll(&mut feed, &path), "");
}

#[test]
fn the_next_run_of_the_task_is_followed_from_its_first_step() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let path = dir.path().join("history.db");
    let mut db = database(&path);
    db.write(&[
        Record::Run(Box::new(start(1, "claude-1", 1, 1))),
        step(1, 1, 2, "cargo build"),
    ])
    .expect("the run is written");
    let (mut feed, _opening) = open(&path, of_task(1), 10);

    db.write(&[
        Record::Run(Box::new(start(2, "claude-1", 1, 9))),
        step(2, 1, 10, "git status"),
    ])
    .expect("the next run is written");
    let page = poll(&mut feed, &path);

    let lines: Vec<&str> = page.lines().collect();
    assert_eq!(lines.len(), 2, "{page}");
    assert!(
        lines[0].contains(" run run-20261121-132909-0002 started by claude-1"),
        "{page}"
    );
    assert_eq!(lines[1], "2026-11-21T12:50:10Z command: git status");
}

#[test]
fn the_end_of_a_run_is_followed_by_what_its_round_waits_for() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let path = dir.path().join("history.db");
    let mut db = database(&path);
    db.write(&[
        Record::Run(Box::new(start(1, "claude-1", 1, 1))),
        step(1, 1, 2, "cargo build"),
    ])
    .expect("the run is written");
    let (mut feed, _opening) = open(&path, of_task(1), 10);

    let mut end = row(LogEvent::RunEnded, 1, 1, 5);
    end.fields
        .insert("end".to_owned(), serde_json::json!("Reported { outcome: Done }"));
    db.write_logs(&[end]).expect("the end is written");
    let page = poll(&mut feed, &path);

    let lines: Vec<&str> = page.lines().collect();
    assert_eq!(lines.len(), 2, "{page}");
    assert!(lines[0].ends_with(": ended reported DONE"), "{page}");
    assert_eq!(
        lines[1],
        "round round-20261121-132909-q1w2 waits for the queued run of its task to start"
    );
}

#[test]
fn a_round_followed_names_the_agent_of_each_step() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let path = dir.path().join("history.db");
    let mut db = database(&path);
    db.write(&[
        Record::Run(Box::new(start(1, "claude-1", 1, 1))),
        Record::Run(Box::new(start(2, "codex-1", 2, 1))),
        step(1, 1, 2, "cargo build"),
        step(2, 1, 3, "cargo test"),
    ])
    .expect("the runs are written");
    let filter = LogFilter {
        round: Some(round()),
        ..LogFilter::default()
    };

    let (_feed, text) = open(&path, filter, 10);

    assert!(
        text.contains("2026-11-21T12:50:02Z claude-1 command: cargo build"),
        "{text}"
    );
    assert!(
        text.contains("2026-11-21T12:50:03Z codex-1 command: cargo test"),
        "{text}"
    );
}

#[test]
fn steps_left_out_print_the_records_alone() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let path = dir.path().join("history.db");
    let mut db = database(&path);
    db.write(&[
        Record::Run(Box::new(start(1, "claude-1", 1, 1))),
        step(1, 1, 2, "cargo build"),
    ])
    .expect("the run is written");
    db.write_logs(&[row(LogEvent::TaskStatusChanged, 1, 1, 3)])
        .expect("the row is written");
    let reader = Reader::open(&path).expect("the history reads");
    let (mut feed, opening) = Feed::open(&reader, of_task(1), StepView::LeftOut, 10).expect("the feed opens");
    db.write(&[step(1, 2, 4, "cargo test")]).expect("the step is written");

    let page = feed.poll(&reader, &mut waits).expect("the page is read");

    assert_eq!(opening.lines().count(), 1, "{opening}");
    assert_eq!(page, "");
}

/// Diagnostics of the writer in a test: there are none to read.
#[derive(Debug)]
struct Silent;

impl agent_runtime::history::Reporter for Silent {
    fn report(&self, _problem: &agent_runtime::history::Problem) {}
}

#[test]
fn a_value_the_history_masks_is_masked_in_the_step_a_follow_prints() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let path = dir.path().join("history.db");
    let db = HistoryDb::open(&path, MaxTextBytes::default()).expect("a temporary history opens");
    let secrets =
        agent_core::config::Secrets::parse("github_token = \"private-é-token\"", 0o600).expect("the secrets parse");
    let started = agent_runtime::history::History::start_with_secrets(
        db,
        agent_runtime::history::Setup {
            capacity: agent_runtime::history::DEFAULT_QUEUE,
            reporter: std::sync::Arc::new(Silent),
        },
        secrets,
    )
    .expect("the history starts");
    started.history.record(Record::Team(Box::new(TeamStart {
        team: team(),
        at: at(0),
        repo: "example".to_owned(),
        base_branch: "main".parse().expect("a literal branch is valid"),
        merge: agent_core::state::MergeMode::NoMerge,
        task_list: String::new(),
        plan: Vec::new(),
    })));
    started
        .history
        .record(Record::Run(Box::new(start(1, "claude-1", 1, 1))));
    started.history.record(step(
        1,
        1,
        2,
        "curl -H 'Authorization: private-é-token' https://example.com",
    ));
    started.writer.close().expect("the writer closes");

    let (_feed, text) = open(&path, of_task(1), 10);

    assert!(
        text.contains("2026-11-21T12:50:02Z command: curl -H 'Authorization: [REDACTED]' https://example.com"),
        "{text}"
    );
    assert!(!text.contains("private"), "{text}");
}

/// The lines of `text`.
fn lines_of(text: &str) -> Vec<&str> {
    text.lines().collect()
}

/// A history of run 1 of task 1 (started at 01 s) with a step at 12 s and rows at 05 s and 15 s.
fn two_rows_around_a_step(path: &Path) -> HistoryDb {
    let mut db = database(path);
    db.write(&[
        Record::Run(Box::new(start(1, "claude-1", 1, 1))),
        step(1, 1, 12, "cargo test"),
    ])
    .expect("the run is written");
    db.write_logs(&[
        row(LogEvent::TaskStatusChanged, 1, 1, 5),
        row(LogEvent::TaskStatusChanged, 1, 1, 15),
    ])
    .expect("the rows are written");
    db
}

#[test]
fn a_step_stands_between_the_rows_recorded_before_and_after_it() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let path = dir.path().join("history.db");
    let _db = two_rows_around_a_step(&path);

    let (_feed, text) = open(&path, of_task(1), 10);

    let lines = lines_of(&text);
    assert_eq!(lines.len(), 4, "{text}");
    assert!(
        lines[0].starts_with("2026-11-21T12:50:05Z info task_status_changed "),
        "{text}"
    );
    assert!(
        lines[1].contains(" run run-20261121-132909-0001 started by claude-1"),
        "{text}"
    );
    assert_eq!(lines[2], "2026-11-21T12:50:12Z command: cargo test");
    assert!(
        lines[3].starts_with("2026-11-21T12:50:15Z info task_status_changed "),
        "{text}"
    );
}

#[test]
fn a_page_puts_a_step_between_the_rows_around_it_too() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let path = dir.path().join("history.db");
    let mut db = database(&path);
    db.write(&[Record::Run(Box::new(start(1, "claude-1", 1, 1)))])
        .expect("the run is written");
    let (mut feed, _opening) = open(&path, of_task(1), 10);
    db.write(&[step(1, 1, 12, "cargo test")]).expect("the step is written");
    db.write_logs(&[
        row(LogEvent::TaskStatusChanged, 1, 1, 5),
        row(LogEvent::TaskStatusChanged, 1, 1, 15),
    ])
    .expect("the rows are written");

    let page = poll(&mut feed, &path);

    let lines = lines_of(&page);
    assert_eq!(lines.len(), 4, "{page}");
    assert!(lines[0].starts_with("2026-11-21T12:50:05Z "), "{page}");
    assert_eq!(lines[2], "2026-11-21T12:50:12Z command: cargo test");
    assert!(lines[3].starts_with("2026-11-21T12:50:15Z "), "{page}");
}

#[test]
fn the_line_of_a_run_does_not_take_a_place_of_the_newest_lines() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let path = dir.path().join("history.db");
    let mut db = database(&path);
    db.write(&[
        Record::Run(Box::new(start(1, "claude-1", 1, 1))),
        step(1, 1, 8, "cargo test"),
    ])
    .expect("the run is written");
    db.write_logs(&[row(LogEvent::TaskStatusChanged, 1, 1, 5)])
        .expect("the row is written");

    let (_feed, text) = open(&path, of_task(1), 2);

    assert!(
        text.contains("2026-11-21T12:50:05Z info task_status_changed "),
        "{text}"
    );
    assert!(text.contains("2026-11-21T12:50:08Z command: cargo test"), "{text}");
}

#[test]
fn each_run_is_told_just_before_its_own_first_step() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let path = dir.path().join("history.db");
    let mut db = database(&path);
    db.write(&[
        Record::Run(Box::new(start(1, "claude-1", 1, 1))),
        step(1, 1, 2, "first"),
        Record::Run(Box::new(start(2, "claude-1", 1, 3))),
        step(2, 1, 4, "second"),
    ])
    .expect("the runs are written");

    let (_feed, text) = open(&path, of_task(1), 10);

    let lines = lines_of(&text);
    assert_eq!(lines.len(), 4, "{text}");
    assert!(lines[0].contains(" run run-20261121-132909-0001 "), "{text}");
    assert!(lines[1].ends_with("command: first"), "{text}");
    assert!(lines[2].contains(" run run-20261121-132909-0002 "), "{text}");
    assert!(lines[3].ends_with("command: second"), "{text}");
}

/// A filter of task 1 that also asks for `change`.
fn filtered(change: impl FnOnce(&mut LogFilter)) -> LogFilter {
    let mut filter = of_task(1);
    change(&mut filter);
    filter
}

#[test]
fn a_step_outside_the_times_of_the_filter_is_not_printed() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let path = dir.path().join("history.db");
    let _db = two_rows_around_a_step(&path);

    let (_feed, after) = open(&path, filtered(|filter| filter.since = Some(at(20))), 10);
    let (_feed, before) = open(&path, filtered(|filter| filter.until = Some(at(10))), 10);
    let (_feed, inside) = open(&path, filtered(|filter| filter.since = Some(at(10))), 10);

    assert!(!after.contains("command"), "{after}");
    assert!(!before.contains("command"), "{before}");
    assert!(inside.contains("2026-11-21T12:50:12Z command: cargo test"), "{inside}");
}

#[test]
fn a_step_that_does_not_contain_the_searched_phrase_is_not_printed() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let path = dir.path().join("history.db");
    let _db = two_rows_around_a_step(&path);

    let (_feed, missing) = open(&path, filtered(|filter| filter.grep = Some("absent".to_owned())), 10);
    let (_feed, found) = open(&path, filtered(|filter| filter.grep = Some("CARGO".to_owned())), 10);

    assert!(!missing.contains("command"), "{missing}");
    assert!(found.contains("2026-11-21T12:50:12Z command: cargo test"), "{found}");
}

#[test]
fn a_page_leaves_out_the_steps_a_filter_refuses_and_still_goes_past_them() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let path = dir.path().join("history.db");
    let mut db = database(&path);
    db.write(&[Record::Run(Box::new(start(1, "claude-1", 1, 1)))])
        .expect("the run is written");
    let (mut feed, _opening) = open(&path, filtered(|filter| filter.grep = Some("test".to_owned())), 10);

    db.write(&[step(1, 1, 3, "cargo build")]).expect("the step is written");
    let refused = poll(&mut feed, &path);
    db.write(&[step(1, 2, 4, "cargo test")]).expect("the step is written");
    let accepted = poll(&mut feed, &path);

    assert_eq!(refused, "");
    assert!(
        accepted.ends_with("2026-11-21T12:50:04Z command: cargo test\n"),
        "{accepted}"
    );
    assert!(!accepted.contains("build"), "{accepted}");
}

#[test]
fn a_filter_by_level_or_event_has_no_steps_to_show() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let path = dir.path().join("history.db");
    let _db = two_rows_around_a_step(&path);

    let (_feed, by_event) = open(
        &path,
        filtered(|filter| filter.event = Some(LogEvent::TaskStatusChanged)),
        10,
    );
    let (_feed, by_level) = open(
        &path,
        filtered(|filter| filter.level = Some(agent_core::log::LogLevel::Info)),
        10,
    );

    assert!(!by_event.contains("command"), "{by_event}");
    assert!(!by_level.contains("command"), "{by_level}");
    assert_eq!(by_event.lines().count(), 2, "{by_event}");
}
