//! `agent log` on the command line: how it is parsed, the story it prints from the history file alone, `--json` with the
//! records each once, what each flag changes, and the exit status of an ID the history does not know.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use agent_core::ids::{RoundId, TeamId};
use agent_runtime::history::Reader;
use rusqlite::Connection;

use super::*;

#[path = "../../tests/support/story_history.rs"]
mod story_history;

use story_history::{PROMPT, Written, write_history, write_old_history};

/// A home in a temporary directory.
fn home() -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().canonicalize().unwrap();
    (dir, home)
}

fn environment(home: &Path) -> Environment {
    Environment::new(Some(OsString::from(home)))
}

/// Runs `agent` with `args` in `environment` and returns its status, standard output and standard error.
fn agent(environment: &Environment, args: &[&str]) -> (ExitCode, String, String) {
    let mut out = Vec::new();
    let mut err = Vec::new();
    let code = run_in(
        environment,
        std::iter::once("agent").chain(args.iter().copied()),
        &mut out,
        &mut err,
    )
    .unwrap();
    (code, String::from_utf8(out).unwrap(), String::from_utf8(err).unwrap())
}

/// A home with a history in it.
fn with_history() -> (tempfile::TempDir, PathBuf, Written) {
    let (dir, home) = home();
    let written = write_history(&home.join("history.db"));
    (dir, home, written)
}

fn parsed(args: &[&str]) -> Result<LogArgs, clap::Error> {
    match parse_invocation(std::iter::once("agent").chain(args.iter().copied()))? {
        Invocation::Log(args) => Ok(args),
        other => panic!("`agent log` was expected: {other:?}"),
    }
}

#[test]
fn log_takes_a_round_or_a_team_and_the_flags() {
    let round: RoundId = "round-20261111-193001-q1w2".parse().unwrap();
    let team: TeamId = "team-20261111-193001-t5m8".parse().unwrap();

    let by_round = parsed(&["log", "--round", round.as_str(), "--prompts", "--no-steps", "--json"]).unwrap();
    let by_team = parsed(&["log", "--team", team.as_str()]).unwrap();

    assert_eq!(by_round.round, Some(round));
    assert!(by_round.prompts && by_round.no_steps && by_round.json);
    assert_eq!(by_team.team, Some(team));
    assert!(!by_team.prompts && !by_team.no_steps && !by_team.json);
}

#[test]
fn log_refuses_no_subject_two_subjects_and_a_name_that_is_not_an_id_with_status_2() {
    use clap::error::ErrorKind;
    let cases = [
        (&["log"][..], ErrorKind::MissingRequiredArgument),
        (
            &[
                "log",
                "--round",
                "round-20261111-193001-q1w2",
                "--team",
                "team-20261111-193001-t5m8",
            ],
            ErrorKind::ArgumentConflict,
        ),
        (&["log", "--round", "nonsense"], ErrorKind::ValueValidation),
        (
            &["log", "--team", "round-20261111-193001-q1w2"],
            ErrorKind::ValueValidation,
        ),
    ];

    for (args, kind) in cases {
        assert_eq!(parsed(args).unwrap_err().kind(), kind, "{args:?}");
        let (code, out, err) = agent(&Environment::new(None), args);
        assert_eq!((code, out.as_str()), (ExitCode::from(2), ""), "{args:?}");
        assert!(!err.is_empty(), "{args:?}");
    }
}

#[test]
fn the_help_lists_log() {
    assert!(command().render_help().to_string().contains("log"));
}

#[test]
fn a_round_is_told_from_its_item_to_its_merge_without_a_daemon() {
    let (_dir, home, written) = with_history();

    let (code, out, err) = agent(&environment(&home), &["log", "--round", written.rounds[0].as_str()]);

    assert_eq!((code, err.as_str()), (ExitCode::SUCCESS, ""));
    for told in [
        format!("Round {} of team {}\n", written.rounds[0], written.team),
        "Item 1.1: Item 1 — zażółć\n".to_owned(),
        "prompt: 7 lines".to_owned(),
        "      line 5\n      … 2 more lines\n".to_owned(),
        "steps (2):\n".to_owned(),
        "changes: head 1111111 → 2222222\n".to_owned(),
        "pull request #7 opened".to_owned(),
        "review pass 1 of #7 by inv-c: APPROVE\n".to_owned(),
        "pull request #7 merged as 4444444\n".to_owned(),
    ] {
        assert!(out.contains(&told), "{told:?} is not in\n{out}");
    }
    assert!(!out.contains("line 6"), "{out}");
    assert!(!out.contains("Item 2"), "the other round is told too\n{out}");
    assert!(!home.join("agent.sock").exists());
}

#[test]
fn the_prompts_are_told_whole_and_the_steps_are_left_out_when_asked() {
    let (_dir, home, written) = with_history();
    let round = written.rounds[0].as_str();

    let (_, whole, _) = agent(&environment(&home), &["log", "--round", round, "--prompts"]);
    let (_, short, _) = agent(&environment(&home), &["log", "--round", round, "--no-steps"]);

    assert!(whole.contains(&format!("{PROMPT}\n")), "{whole}");
    assert!(short.contains("steps: 2 (left out)\n"), "{short}");
    assert!(!short.contains("step 1 of the run"), "{short}");
}

#[test]
fn a_team_is_told_round_by_round_in_order() {
    let (_dir, home, written) = with_history();

    let (code, out, _) = agent(&environment(&home), &["log", "--team", written.team.as_str()]);

    assert_eq!(code, ExitCode::SUCCESS);
    let first = out.find(&format!("Round {} of team", written.rounds[0])).unwrap();
    let second = out.find(&format!("Round {} of team", written.rounds[1])).unwrap();
    assert!(first < second, "{out}");
    assert_eq!(out.matches("Team started").count(), 1, "{out}");
    assert!(out.contains("team ended\n"), "{out}");
}

/// The number of records the database has for `round`: those linked to it and the start of its team, counted in SQL.
fn records_of_round(home: &Path, round: &RoundId) -> u32 {
    let raw = Connection::open(home.join("history.db")).unwrap();
    raw.query_row(
        "SELECT count(*) FROM history_records WHERE sequence IN \
         (SELECT sequence FROM history_record_rounds WHERE round = ?1) \
         OR (kind = 'team' AND team = (SELECT team FROM rounds WHERE round = ?1))",
        [round.as_str()],
        |row| row.get::<_, u32>(0),
    )
    .unwrap()
}

#[test]
fn json_prints_every_record_of_the_round_once_and_the_flags_do_not_change_it() {
    let (_dir, home, written) = with_history();
    let round = written.rounds[0].as_str();

    let (code, out, err) = agent(&environment(&home), &["log", "--round", round, "--json"]);
    let (_, flagged, _) = agent(
        &environment(&home),
        &["log", "--round", round, "--json", "--no-steps", "--prompts"],
    );

    assert_eq!((code, err.as_str()), (ExitCode::SUCCESS, ""));
    assert_eq!(
        u32::try_from(out.lines().count()).unwrap(),
        records_of_round(&home, &written.rounds[0])
    );
    assert_eq!(flagged, out);
    let kinds: Vec<String> = out
        .lines()
        .map(|line| {
            serde_json::from_str::<serde_json::Value>(line).unwrap()["kind"]
                .as_str()
                .unwrap()
                .to_owned()
        })
        .collect();
    assert_eq!(kinds.iter().filter(|kind| *kind == "step").count(), 2, "{kinds:?}");
    assert!(kinds.contains(&"pull_merged".to_owned()), "{kinds:?}");
    let sequences: Vec<u64> = out
        .lines()
        .map(|line| {
            serde_json::from_str::<serde_json::Value>(line).unwrap()["sequence"]
                .as_u64()
                .unwrap()
        })
        .collect();
    let mut once = sequences.clone();
    once.sort_unstable();
    once.dedup();
    assert_eq!(once.len(), sequences.len());
}

#[test]
fn a_round_and_a_team_the_history_does_not_know_are_refused_with_status_3_and_named() {
    let (_dir, home, written) = with_history();
    let other_round = "round-20990101-000000-zzzz";
    let other_team = "team-20990101-000000-zzzz";

    let (code_round, out_round, err_round) = agent(&environment(&home), &["log", "--round", other_round]);
    let (code_team, out_team, err_team) = agent(&environment(&home), &["log", "--team", other_team, "--json"]);

    assert_eq!((code_round, out_round.as_str()), (ExitCode::from(3), ""));
    assert!(err_round.contains(other_round), "{err_round}");
    assert_eq!((code_team, out_team.as_str()), (ExitCode::from(3), ""));
    assert!(err_team.contains(other_team), "{err_team}");
    let (known, _, _) = agent(&environment(&home), &["log", "--round", written.rounds[1].as_str()]);
    assert_eq!(known, ExitCode::SUCCESS);
}

#[test]
fn a_home_without_a_history_and_a_history_of_a_newer_version_are_failures_with_status_1() {
    let (_dir, home) = home();

    let (missing, out, err) = agent(&environment(&home), &["log", "--round", "round-20261111-193001-q1w2"]);

    assert_eq!((missing, out.as_str()), (ExitCode::FAILURE, ""));
    assert!(err.contains("history.db"), "{err}");
    let raw = Connection::open(home.join("history.db")).unwrap();
    raw.pragma_update(None, "user_version", 99).unwrap();
    drop(raw);
    let (newer, out, err) = agent(&environment(&home), &["log", "--team", "team-20261111-193001-t5m8"]);
    assert_eq!((newer, out.as_str()), (ExitCode::FAILURE, ""));
    assert!(err.contains("version 99"), "{err}");
}

#[test]
fn a_history_a_daemon_of_item_8_4_left_is_told_as_it_is_and_is_not_changed() {
    let (_dir, home) = home();
    let written = write_old_history(&home.join("history.db"));
    let before = std::fs::read(home.join("history.db")).unwrap();

    let (code, out, err) = agent(&environment(&home), &["log", "--round", written.rounds[0].as_str()]);

    assert_eq!((code, err.as_str()), (ExitCode::SUCCESS, ""));
    assert!(out.contains("Item 1.1: Item 1 — zażółć\n"), "{out}");
    assert!(out.contains("run run-20261111-193001-r7q2:"), "{out}");
    assert!(out.contains("no pull request recorded for this round\n"), "{out}");
    assert_eq!(std::fs::read(home.join("history.db")).unwrap(), before);
    assert!(
        Reader::open(&home.join("history.db"))
            .unwrap()
            .team(&written.team)
            .unwrap()
            .is_some()
    );
}

/// A writer that refuses every write, like a closed pipe.
struct ClosedPipe;

impl std::io::Write for ClosedPipe {
    fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
        Err(std::io::Error::from(std::io::ErrorKind::BrokenPipe))
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Err(std::io::Error::from(std::io::ErrorKind::BrokenPipe))
    }
}

#[test]
fn a_closed_pipe_is_a_failure_that_names_the_story_not_a_panic() {
    let (_dir, home, written) = with_history();
    let mut err = Vec::new();

    let code = run_in(
        &environment(&home),
        ["agent", "log", "--round", written.rounds[0].as_str()],
        &mut ClosedPipe,
        &mut err,
    )
    .unwrap();

    assert_eq!(code, ExitCode::FAILURE);
    assert!(String::from_utf8(err).unwrap().contains("cannot write the story"));
}
