use std::collections::BTreeMap;
use std::num::NonZeroU64;
use std::str::FromStr;

use serde_json::json;

use super::*;
use crate::history::{MaxTextBytes, UnknownCutField};
use crate::ids::{RoundId, RunId, TaskId, TeamId};
use crate::state::Timestamp;

/// The daemon log documentation (`docs/log.md`).
const LOG_DOCS: &str = include_str!("../../../../docs/log.md");

fn team_id() -> TeamId {
    "team-20260413-090000-aaa1".parse().unwrap()
}

fn other_team_id() -> TeamId {
    "team-20260413-090500-bbb2".parse().unwrap()
}

fn task_id() -> TaskId {
    "task-20260413-090100-ccc3".parse().unwrap()
}

fn other_task_id() -> TaskId {
    "task-20260413-090200-ddd4".parse().unwrap()
}

fn round(n: u32) -> RoundId {
    RoundId::derive(&team_id(), n)
}

fn run_num(n: u32) -> RunId {
    format!("run-20260413-09010{n}-eee{n}").parse().unwrap()
}

fn ts(rfc3339: &str) -> Timestamp {
    rfc3339.parse().unwrap()
}

fn sample_warn_record() -> LogRecord {
    let mut fields = BTreeMap::new();
    fields.insert("line".to_owned(), json!("not json"));
    LogRecord {
        at: ts("2026-04-13T09:00:00Z"),
        module: "agent_runtime::run".to_owned(),
        event: LogEvent::CliOutputUnreadable,
        team: Some(LogTeamScope::new(team_id(), [round(1)])),
        task: Some(LogTaskScope::new(task_id(), Some(run_num(2)))),
        message: "model CLI line is not valid JSON".to_owned(),
        fields,
        cut: Cut::default(),
    }
}

#[test]
fn every_log_level_parses_renders_and_keeps_events_at_or_above_its_severity() {
    assert_eq!(LogLevel::default(), LogLevel::Info);
    assert_eq!(
        LogLevel::ALL,
        &[LogLevel::Error, LogLevel::Warn, LogLevel::Info, LogLevel::Debug]
    );

    for (level, name) in [
        (LogLevel::Error, "error"),
        (LogLevel::Warn, "warn"),
        (LogLevel::Info, "info"),
        (LogLevel::Debug, "debug"),
    ] {
        assert_eq!(level.as_str(), name);
        assert_eq!(level.to_string(), name);
        assert_eq!(LogLevel::from_str(name).unwrap(), level);
        assert_eq!(LogLevel::try_from(name.to_owned()).unwrap(), level);
        let encoded = serde_json::to_string(&level).unwrap();
        assert_eq!(encoded, format!("\"{name}\""));
        let decoded: LogLevel = serde_json::from_str(&encoded).unwrap();
        assert_eq!(decoded, level);
    }

    for bad in ["trace", "", "INFO", "warning"] {
        let err = LogLevel::from_str(bad).unwrap_err();
        assert_eq!(err.what, "log level");
        assert_eq!(err.value, bad);
    }

    for (threshold, expected_kept) in [
        (LogLevel::Error, &[LogLevel::Error][..]),
        (LogLevel::Warn, &[LogLevel::Error, LogLevel::Warn][..]),
        (LogLevel::Info, &[LogLevel::Error, LogLevel::Warn, LogLevel::Info][..]),
        (
            LogLevel::Debug,
            &[LogLevel::Error, LogLevel::Warn, LogLevel::Info, LogLevel::Debug][..],
        ),
    ] {
        for &candidate in LogLevel::ALL {
            assert_eq!(
                threshold.keeps(candidate),
                expected_kept.contains(&candidate),
                "threshold {threshold} vs event {candidate}"
            );
        }
    }
}

#[test]
fn every_log_event_has_its_fixed_snake_case_name_and_level() {
    let expected = [
        (LogEvent::CliOutputUnreadable, "cli_output_unreadable", LogLevel::Warn),
        (
            LogEvent::RunEndedWithoutStatus,
            "run_ended_without_status",
            LogLevel::Warn,
        ),
        (LogEvent::RunHung, "run_hung", LogLevel::Warn),
        (LogEvent::RunQuotaExhausted, "run_quota_exhausted", LogLevel::Warn),
        (LogEvent::RunRateLimited, "run_rate_limited", LogLevel::Warn),
        (LogEvent::BusDown, "bus_down", LogLevel::Warn),
        (LogEvent::GitCallFailed, "git_call_failed", LogLevel::Warn),
        (LogEvent::GithubCallFailed, "github_call_failed", LogLevel::Warn),
        (LogEvent::GateCannotStart, "gate_cannot_start", LogLevel::Warn),
        (LogEvent::GateTimedOut, "gate_timed_out", LogLevel::Warn),
        (LogEvent::GateDirtyTree, "gate_dirty_tree", LogLevel::Warn),
        (LogEvent::StateSaveFailed, "state_save_failed", LogLevel::Error),
        (LogEvent::QuestionsFileInvalid, "questions_file_invalid", LogLevel::Warn),
        (
            LogEvent::ConfigurationNameMissing,
            "configuration_name_missing",
            LogLevel::Error,
        ),
        (LogEvent::RestartCorrected, "restart_corrected", LogLevel::Warn),
        (LogEvent::SnapshotFailed, "snapshot_failed", LogLevel::Warn),
        (LogEvent::SnapshotLarge, "snapshot_large", LogLevel::Warn),
        (LogEvent::RequestRefused, "request_refused", LogLevel::Warn),
        (LogEvent::DaemonPanicked, "daemon_panicked", LogLevel::Error),
        (LogEvent::DaemonTaskFailed, "daemon_task_failed", LogLevel::Error),
        (LogEvent::TeamNotStarted, "team_not_started", LogLevel::Error),
        (LogEvent::NotificationFailed, "notification_failed", LogLevel::Error),
        (
            LogEvent::HistoryRecordIncomplete,
            "history_record_incomplete",
            LogLevel::Warn,
        ),
        (LogEvent::TeamChanged, "team_changed", LogLevel::Info),
        (LogEvent::RoundChanged, "round_changed", LogLevel::Info),
        (LogEvent::ItemPhaseChanged, "item_phase_changed", LogLevel::Info),
        (LogEvent::TaskStatusChanged, "task_status_changed", LogLevel::Info),
        (LogEvent::RunStarted, "run_started", LogLevel::Info),
        (LogEvent::RunEnded, "run_ended", LogLevel::Info),
        (LogEvent::DaemonStarted, "daemon_started", LogLevel::Info),
        (LogEvent::DaemonStopRequested, "daemon_stop_requested", LogLevel::Info),
        (LogEvent::DaemonStopped, "daemon_stopped", LogLevel::Info),
        (LogEvent::RequestDelegated, "request_delegated", LogLevel::Info),
        (LogEvent::QuestionsAnswered, "questions_answered", LogLevel::Info),
        (LogEvent::ControlUsed, "control_used", LogLevel::Info),
        (LogEvent::ChecksDecided, "checks_decided", LogLevel::Info),
        (LogEvent::ReviewsRequested, "reviews_requested", LogLevel::Info),
        (LogEvent::ReviewReportLate, "review_report_late", LogLevel::Info),
        (LogEvent::NotificationSent, "notification_sent", LogLevel::Info),
        (LogEvent::QuestionsFileOpened, "questions_file_opened", LogLevel::Info),
        (
            LogEvent::BuildDirectoryRemoved,
            "build_directory_removed",
            LogLevel::Info,
        ),
        (LogEvent::DecisionTaken, "decision_taken", LogLevel::Debug),
        (
            LogEvent::OutsideCallCompleted,
            "outside_call_completed",
            LogLevel::Debug,
        ),
        (LogEvent::PeerGone, "peer_gone", LogLevel::Debug),
        (LogEvent::GithubStepWaiting, "github_step_waiting", LogLevel::Warn),
        (LogEvent::GithubStepResumed, "github_step_resumed", LogLevel::Info),
    ];

    assert_eq!(LogEvent::ALL.len(), expected.len());
    for (&actual, &(variant, name, level)) in LogEvent::ALL.iter().zip(expected.iter()) {
        assert_eq!(actual, variant);
        assert_eq!(variant.as_str(), name);
        assert_eq!(variant.to_string(), name);
        assert_eq!(variant.level(), level, "{name}");
        assert_eq!(LogEvent::from_str(name).unwrap(), variant);
        assert_eq!(LogEvent::try_from(name.to_owned()).unwrap(), variant);
        let encoded = serde_json::to_string(&variant).unwrap();
        assert_eq!(encoded, format!("\"{name}\""));
        let decoded: LogEvent = serde_json::from_str(&encoded).unwrap();
        assert_eq!(decoded, variant);
    }

    for bad in ["unknown_event", "", "CLI_OUTPUT_UNREADABLE"] {
        let err = LogEvent::from_str(bad).unwrap_err();
        assert_eq!(err.what, "log event");
        assert_eq!(err.value, bad);
    }
}

#[test]
fn docs_log_md_lists_every_log_event_name_and_its_level() {
    let section = LOG_DOCS
        .split("## Events (`agent_core::log::LogEvent`)")
        .nth(1)
        .expect("docs/log.md must have the Events section");

    let mut documented = Vec::new();
    for line in section.lines() {
        let trimmed = line.trim();
        if !trimmed.starts_with("| `") {
            continue;
        }
        let mut cells = trimmed.split('|').map(str::trim).filter(|cell| !cell.is_empty());
        let event_cell = cells.next().expect("event cell");
        let level_cell = cells.next().expect("level cell");
        let meaning_cell = cells.next().expect("meaning cell");
        assert!(!meaning_cell.is_empty(), "every event in docs/log.md needs a meaning");
        let event_name = event_cell.trim_matches('`');
        let level_name = level_cell.trim_matches('`');
        documented.push((event_name, level_name));
    }

    let expected: Vec<(&str, &str)> = LogEvent::ALL
        .iter()
        .map(|&event| (event.as_str(), event.level().as_str()))
        .collect();
    assert_eq!(documented, expected);
}

#[test]
fn log_team_scope_sorts_and_deduplicates_rounds_and_validates_sorted_inputs() {
    let scope = LogTeamScope::new(team_id(), [round(3), round(1), round(2), round(1)]);
    assert_eq!(scope.team(), &team_id());
    assert_eq!(scope.rounds(), &[round(1), round(2), round(3)]);

    let empty = LogTeamScope::from_sorted(team_id(), Vec::new()).unwrap();
    assert_eq!(empty.rounds(), &[]);

    let sorted = LogTeamScope::from_sorted(team_id(), vec![round(1), round(4)]).unwrap();
    assert_eq!(sorted.rounds(), &[round(1), round(4)]);

    for bad in [vec![round(2), round(2)], vec![round(3), round(1)]] {
        let err = LogTeamScope::from_sorted(team_id(), bad).unwrap_err();
        assert_eq!(err, InvalidLogEntry::UnsortedRounds);
    }
}

#[test]
fn log_record_keep_truncates_message_on_char_boundary_and_marks_oversized_fields_json() {
    let mut fields = BTreeMap::new();
    fields.insert("detail".to_owned(), json!("zażółć gęślą jaźń"));
    let record = LogRecord {
        at: ts("2026-04-13T09:00:00Z"),
        module: "agent::daemon".to_owned(),
        event: LogEvent::GitCallFailed,
        team: None,
        task: None,
        message: "zażółć gęślą jaźń".to_owned(),
        fields,
        cut: Cut::default(),
    };

    // "zaż" is 4 bytes ('z'=1, 'a'=1, 'ż'=2); 'ó' starts at byte 4 and is 2 bytes, so a 5-byte limit keeps "zaż".
    let kept = record.clone().keep(MaxTextBytes::new(5).unwrap());
    assert_eq!(kept.message, "zaż");
    assert_eq!(kept.fields.get("truncated"), Some(&json!(true)));
    assert_eq!(kept.fields.len(), 1);
    assert_eq!(kept.cut.to_string(), "fields,message");

    // A generous limit keeps both message and fields intact and leaves cut empty.
    let intact = record.keep(MaxTextBytes::new(1024).unwrap());
    assert_eq!(intact.message, "zażółć gęślą jaźń");
    assert_eq!(intact.fields.get("detail"), Some(&json!("zażółć gęślą jaźń")));
    assert!(intact.cut.is_empty());
}

#[test]
fn same_warning_matches_only_identical_warnings_in_the_same_scope() {
    let base = sample_warn_record();
    let mut later_same = base.clone();
    later_same.at = ts("2026-04-13T09:01:00Z");
    later_same.module = "other::module".to_owned();
    assert!(base.same_warning(&later_same));

    let mut different_event = base.clone();
    different_event.event = LogEvent::RunHung;
    assert!(!base.same_warning(&different_event));

    let mut different_team = base.clone();
    different_team.team = Some(LogTeamScope::new(other_team_id(), [round(1)]));
    assert!(!base.same_warning(&different_team));

    let mut different_rounds = base.clone();
    different_rounds.team = Some(LogTeamScope::new(team_id(), [round(1), round(2)]));
    assert!(!base.same_warning(&different_rounds));

    let mut different_task = base.clone();
    different_task.task = Some(LogTaskScope::new(other_task_id(), Some(run_num(2))));
    assert!(!base.same_warning(&different_task));

    let mut different_run = base.clone();
    different_run.task = Some(LogTaskScope::new(task_id(), Some(run_num(3))));
    assert!(!base.same_warning(&different_run));

    let mut different_message = base.clone();
    different_message.message = "different warning message".to_owned();
    assert!(!base.same_warning(&different_message));

    let mut different_fields = base.clone();
    different_fields.fields.insert("line".to_owned(), json!("other"));
    assert!(!base.same_warning(&different_fields));

    for non_warn in [LogEvent::StateSaveFailed, LogEvent::RunStarted, LogEvent::DecisionTaken] {
        let mut record = base.clone();
        record.event = non_warn;
        assert!(!record.same_warning(&record));
    }
}

#[test]
fn log_entry_try_new_accepts_valid_parts_and_refuses_every_broken_invariant() {
    let mut fields = BTreeMap::new();
    fields.insert("line".to_owned(), json!("bad"));
    let valid_parts = LogEntryParts {
        id: 7,
        first_at: ts("2026-04-13T09:00:00Z"),
        at: ts("2026-04-13T09:00:10Z"),
        level: LogLevel::Warn,
        module: "agent_runtime::run".to_owned(),
        event: LogEvent::CliOutputUnreadable,
        team: Some(team_id()),
        rounds: vec![round(1), round(2)],
        task: Some(task_id()),
        run: Some(run_num(3)),
        message: "bad line".to_owned(),
        fields: fields.clone(),
        count: NonZeroU64::new(5).unwrap(),
        cut: "message".to_owned(),
    };

    let entry = LogEntry::try_new(valid_parts.clone()).unwrap();
    assert_eq!(entry.id(), 7);
    assert_eq!(entry.first_at(), ts("2026-04-13T09:00:00Z"));
    assert_eq!(entry.at(), ts("2026-04-13T09:00:10Z"));
    assert_eq!(entry.level(), LogLevel::Warn);
    assert_eq!(entry.module(), "agent_runtime::run");
    assert_eq!(entry.event(), LogEvent::CliOutputUnreadable);
    assert_eq!(entry.team(), Some(&team_id()));
    assert_eq!(entry.rounds(), &[round(1), round(2)]);
    assert_eq!(entry.task(), Some(&task_id()));
    assert_eq!(entry.run(), Some(&run_num(3)));
    assert_eq!(entry.message(), "bad line");
    assert_eq!(entry.fields(), &fields);
    assert_eq!(entry.count(), NonZeroU64::new(5).unwrap());
    assert!(entry.cut().has("message"));

    let mut bad_level = valid_parts.clone();
    bad_level.level = LogLevel::Info;
    assert_eq!(
        LogEntry::try_new(bad_level).unwrap_err(),
        InvalidLogEntry::LevelMismatch {
            event: LogEvent::CliOutputUnreadable,
            level: LogLevel::Info,
        }
    );

    let mut non_warn_repeated = valid_parts.clone();
    non_warn_repeated.event = LogEvent::RunStarted;
    non_warn_repeated.level = LogLevel::Info;
    non_warn_repeated.count = NonZeroU64::new(2).unwrap();
    assert_eq!(
        LogEntry::try_new(non_warn_repeated).unwrap_err(),
        InvalidLogEntry::NonWarnRepeated {
            event: LogEvent::RunStarted,
            count: NonZeroU64::new(2).unwrap(),
        }
    );

    let mut first_after_last = valid_parts.clone();
    first_after_last.first_at = ts("2026-04-13T09:00:20Z");
    first_after_last.at = ts("2026-04-13T09:00:10Z");
    assert_eq!(
        LogEntry::try_new(first_after_last).unwrap_err(),
        InvalidLogEntry::FirstAfterLast {
            first_at: ts("2026-04-13T09:00:20Z"),
            at: ts("2026-04-13T09:00:10Z"),
        }
    );

    let mut run_without_task = valid_parts.clone();
    run_without_task.task = None;
    assert_eq!(
        LogEntry::try_new(run_without_task).unwrap_err(),
        InvalidLogEntry::RunWithoutTask { run: run_num(3) }
    );

    let mut rounds_without_team = valid_parts.clone();
    rounds_without_team.team = None;
    assert_eq!(
        LogEntry::try_new(rounds_without_team).unwrap_err(),
        InvalidLogEntry::RoundsWithoutTeam
    );

    let mut unsorted_rounds = valid_parts.clone();
    unsorted_rounds.rounds = vec![round(2), round(1)];
    assert_eq!(
        LogEntry::try_new(unsorted_rounds).unwrap_err(),
        InvalidLogEntry::UnsortedRounds
    );

    let mut bad_cut = valid_parts;
    bad_cut.cut = "prompt".to_owned();
    assert_eq!(
        LogEntry::try_new(bad_cut).unwrap_err(),
        InvalidLogEntry::Cut(UnknownCutField {
            field: "prompt".to_owned(),
        })
    );
}
