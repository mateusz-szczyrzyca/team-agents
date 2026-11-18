use std::path::PathBuf;
use time::UtcOffset;

use super::*;
use crate::config::{AbsolutePath, RepoName};
use crate::ids::{AgentName, BranchName, ItemId, RoundId, TaskId, TeamId};
use crate::kinds::Role;
use crate::state::{CommitId, GateOutcome, GateRecord, GateResult, GateTree, RoundLife, Timestamp};
use crate::status::{ItemStatus, TaskStatus};

fn ts(text: &str) -> Timestamp {
    text.parse().expect("valid test timestamp")
}

fn sample_team() -> TeamSummaryFacts {
    TeamSummaryFacts {
        id: "team-20261117-200000-aaaa".parse::<TeamId>().expect("valid team id"),
        repo: "team-agents".parse::<RepoName>().expect("valid repo name"),
        items: vec![ItemSummaryFacts {
            id: "1.1".parse::<ItemId>().expect("valid item id"),
            title: "Summary renderer".to_owned(),
            phase: ItemStatus::InProgress,
            assignee: Some("gemini-1".parse::<AgentName>().expect("valid agent name")),
        }],
        rounds: vec![RoundSummaryFacts {
            id: "round-20261117-200000-rrrr".parse::<RoundId>().expect("valid round id"),
            item: Some("1.1".parse::<ItemId>().expect("valid item id")),
            opened_at: ts("2026-11-17T19:00:00Z"),
            status: RoundLife::Open,
        }],
    }
}

fn sample_task() -> TaskSummaryFacts {
    TaskSummaryFacts {
        id: "task-20261117-200000-tttt".parse::<TaskId>().expect("valid task id"),
        agent: "gemini-1".parse::<AgentName>().expect("valid agent name"),
        role: Role::Member,
        status: TaskStatus::Running,
        run_count: 2,
        last_run_started: Some(ts("2026-11-17T19:30:00Z")),
        last_run_ended: None,
        last_activity: Some(ts("2026-11-17T19:45:00Z")),
        last_run_end_detail: Some("Run 2 running".to_owned()),
        questions: Some(QuestionsSummaryFacts {
            has_unanswered: false,
            content: "All answered".to_owned(),
        }),
    }
}

fn sample_git() -> GitWorktreeSummary {
    GitWorktreeSummary {
        task_id: "task-20261117-200000-tttt".parse::<TaskId>().expect("valid task id"),
        branch: "feature/test".parse::<BranchName>().expect("valid branch name"),
        tip: "0123456789abcdef0123456789abcdef01234567"
            .parse::<CommitId>()
            .expect("valid commit id"),
        origin_tip: Some(
            "0123456789abcdef0123456789abcdef01234567"
                .parse::<CommitId>()
                .expect("valid commit id"),
        ),
        commits_over_base: vec![GitCommitSummaryFacts {
            id: "0123456789abcdef0123456789abcdef01234567"
                .parse::<CommitId>()
                .expect("valid commit id"),
            author: "Dev <dev@example.com>".to_owned(),
            time: ts("2026-11-17T19:35:00Z"),
            subject: "First commit".to_owned(),
        }],
        uncommitted_files: vec![PathBuf::from("crates/agent/src/test.rs")],
        harness_refs: vec!["refs/team-agents/heads/test".to_owned()],
        lead_contains_tip: Some(true),
        newest_untracked: vec![UntrackedFileSummaryFacts {
            path: PathBuf::from("test.log"),
            size_bytes: 1024,
            modified: Some(ts("2026-11-17T19:40:00Z")),
        }],
    }
}

fn sample_input() -> SummaryInput {
    SummaryInput {
        collected_at: ts("2026-11-17T20:00:00Z"),
        anomalies: vec![],
        daemon: Some(DaemonSummary {
            pid: 12345,
            started_at: Some(ts("2026-11-17T18:00:00Z")),
            cmdline: vec!["agent".to_owned(), "daemon".to_owned()],
            env_var_names: vec!["PATH".to_owned(), "HOME".to_owned()],
            exe_path: "/usr/local/bin/agent".to_owned(),
            disk_newer_than_process: false,
            version: "0.1.0".to_owned(),
        }),
        team: sample_team(),
        tasks: vec![sample_task()],
        git: vec![sample_git()],
        gate: vec![TaskGateSummary {
            task_id: "task-20261117-200000-tttt".parse::<TaskId>().expect("valid task id"),
            record: Some(GateRecord {
                tree: GateTree::Clean,
                result: GateResult {
                    head: "0123456789abcdef0123456789abcdef01234567"
                        .parse::<CommitId>()
                        .expect("valid commit id"),
                    finished_at: ts("2026-11-17T19:50:00Z"),
                    outcome: GateOutcome::Passed,
                    log: "/tmp/gate.log".parse::<AbsolutePath>().expect("valid path"),
                },
            }),
            recent_output_tails: vec!["Gate passed with zero findings".to_owned()],
        }],
        history_tail: Some("Round opened".to_owned()),
        daemon_log_lines: Some(vec!["2026-11-17 19:00:00 [info] round started".to_owned()]),
        bus_messages: vec!["[#1] agentbus: team created".to_owned()],
        notifications: vec!["Notification: round active".to_owned()],
        machine: MachineSummary {
            load_avg: Some((0.5, 0.4, 0.3)),
            memory_total_bytes: Some(16_000_000_000),
            memory_available_bytes: Some(8_000_000_000),
            home_fs: Some(FsUsageSummary {
                space_percent: 45,
                inode_percent: 20,
            }),
            temp_fs: Some(FsUsageSummary {
                space_percent: 10,
                inode_percent: 5,
            }),
            daemon_descendant_pids: vec![12346, 12347],
            tasks_dir_bytes: Some(50_000_000),
            worktrees_dir_bytes: Some(120_000_000),
        },
        unreadable_parts: vec![],
    }
}

#[test]
fn sections_in_exact_order_and_present() {
    let input = sample_input();
    let offset = UtcOffset::from_hms(1, 0, 0).expect("valid offset");

    let summary = render_summary(&input, offset).expect("mandatory summary facts fit");

    let mut last_idx = 0;
    for section in SummarySection::ALL {
        let heading = section.heading();
        let remaining = summary.get(last_idx..).expect("valid char boundary");
        let found = remaining.find(heading).unwrap_or_else(|| {
            panic!("missing or out-of-order section heading: {heading}");
        });
        last_idx += found + heading.len();
    }
}

#[test]
fn anomalies_empty_shows_none_of_the_known_ones() {
    let mut input = sample_input();
    input.anomalies.clear();
    let offset = UtcOffset::UTC;

    let summary = render_summary(&input, offset).expect("mandatory summary facts fit");

    assert!(summary.contains("none of the known ones"));
}

#[test]
fn anomalies_with_entries_renders_messages_and_commands() {
    let mut input = sample_input();
    input.anomalies = vec![
        SummaryAnomaly {
            message: "a task is failed".to_owned(),
            command: Some("agent retry task-20261117-200000-tttt".to_owned()),
        },
        SummaryAnomaly {
            message: "filesystem is nearly full".to_owned(),
            command: None,
        },
    ];
    let offset = UtcOffset::UTC;

    let summary = render_summary(&input, offset).expect("mandatory summary facts fit");

    assert!(!summary.contains("none of the known ones"));
    assert!(summary.contains("a task is failed"));
    assert!(summary.contains("agent retry task-20261117-200000-tttt"));
    assert!(summary.contains("filesystem is nearly full"));
}

#[test]
fn task_table_computes_minutes_since_activity_and_saturates_future() {
    let mut input = sample_input();
    // Task 1: active 15 minutes before now (19:45 vs 20:00)
    input.tasks.first_mut().expect("first task").last_activity = Some(ts("2026-11-17T19:45:00Z"));

    // Task 2: active 5 minutes in the future (20:05 vs 20:00) -> should saturate at 0
    let mut task2 = input.tasks.first().expect("first task").clone();
    task2.id = "task-20261117-200000-2222".parse().expect("valid task id");
    task2.last_activity = Some(ts("2026-11-17T20:05:00Z"));
    input.tasks.push(task2);

    let offset = UtcOffset::UTC;

    let summary = render_summary(&input, offset).expect("mandatory summary facts fit");

    assert!(summary.contains("| 15 min |"));
    assert!(summary.contains("| 0 min |"));
}

#[test]
fn timestamps_formatted_in_utc_and_local_offset() {
    let input = sample_input();
    // Offset +02:00
    let offset_plus = UtcOffset::from_hms(2, 0, 0).expect("valid offset");
    let summary_plus = render_summary(&input, offset_plus).expect("mandatory summary facts fit");

    assert!(summary_plus.contains("20:00:00") && summary_plus.contains("22:00:00"));

    // Offset -05:00
    let offset_minus = UtcOffset::from_hms(-5, 0, 0).expect("valid offset");
    let summary_minus = render_summary(&input, offset_minus).expect("mandatory summary facts fit");

    assert!(summary_minus.contains("20:00:00") && summary_minus.contains("15:00:00"));
}

#[test]
fn daemon_unreachable_renders_daemon_does_not_answer() {
    let mut input = sample_input();
    input.daemon = None;
    let offset = UtcOffset::UTC;

    let summary = render_summary(&input, offset).expect("mandatory summary facts fit");

    assert!(summary.contains("The daemon does not answer (unreachable).\n"));
}

#[test]
fn large_content_truncated_under_100k_bytes_keeps_tail_and_preserves_headings() {
    let mut input = sample_input();
    // Add 120KB of daemon log content
    let huge_chunk = "Log line entry with multibyte unicode characters 🦀 and details\n".repeat(2000);
    input.daemon_log_lines = Some(vec![huge_chunk]);

    let offset = UtcOffset::UTC;

    let summary = render_summary(&input, offset).expect("mandatory summary facts fit");

    assert!(
        summary.len() < MAX_SUMMARY_BYTES,
        "summary len {} must be < {}",
        summary.len(),
        MAX_SUMMARY_BYTES
    );
    assert!(summary.contains(CUT_MARKER.trim()), "summary must contain cut marker");

    // All section headings must still be preserved
    for section in SummarySection::ALL {
        assert!(
            summary.contains(section.heading()),
            "heading {} must be preserved after truncation",
            section.heading()
        );
    }
}

#[test]
fn unreadable_parts_named_in_summary() {
    let mut input = sample_input();
    input.unreadable_parts = vec![UnreadablePartSummary {
        name: "daemon.log".to_owned(),
        reason: "permission denied".to_owned(),
    }];
    let offset = UtcOffset::UTC;

    let summary = render_summary(&input, offset).expect("mandatory summary facts fit");

    assert!(summary.contains("daemon.log"));
    assert!(summary.contains("permission denied"));
}

#[test]
fn git_commits_include_author() {
    let input = sample_input();
    let offset = UtcOffset::UTC;

    let summary = render_summary(&input, offset).expect("mandatory summary facts fit");

    assert!(
        summary.contains("Dev <dev@example.com>"),
        "summary must contain commit author"
    );
}

#[test]
fn format_time_includes_local_date_and_handles_midnight_crossing() {
    let mut input = sample_input();
    input.collected_at = ts("2026-11-17T23:30:00Z");
    let offset = UtcOffset::from_hms(2, 0, 0).expect("valid offset");

    let summary = render_summary(&input, offset).expect("mandatory summary facts fit");

    // UTC is 2026-11-17, local date after crossing midnight at +02:00 is 2026-11-18
    assert!(summary.contains("2026-11-17 23:30:00 UTC"));
    assert!(summary.contains("2026-11-18 01:30:00 +02:00"));
}

#[test]
fn huge_early_content_keeps_all_12_headings() {
    let mut input = sample_input();
    // Huge task details (80KB) and huge anomalies (30KB)
    input.tasks.first_mut().expect("task").last_run_end_detail =
        Some("Very long task output detail with unicode 🚀\n".repeat(1500));
    input.anomalies = (0..500)
        .map(|i| SummaryAnomaly {
            message: format!("Anomaly {i}: deeply detailed problem with system 🔧"),
            command: Some(format!("agent fix-rule-{i}")),
        })
        .collect();

    let offset = UtcOffset::UTC;

    let summary = render_summary(&input, offset).expect("mandatory summary facts fit");

    assert!(
        summary.len() < MAX_SUMMARY_BYTES,
        "summary len {} must be < {}",
        summary.len(),
        MAX_SUMMARY_BYTES
    );
    for section in SummarySection::ALL {
        assert!(
            summary.contains(section.heading()),
            "heading {} must be preserved despite huge early content",
            section.heading()
        );
    }
}

#[test]
fn daemon_log_lines_preserve_boundaries() {
    let mut input = sample_input();
    input.daemon_log_lines = Some(vec![
        "2026-11-17 19:00:00 [info] first line".to_owned(),
        "2026-11-17 19:00:01 [info] second line".to_owned(),
    ]);

    let offset = UtcOffset::UTC;

    let summary = render_summary(&input, offset).expect("mandatory summary facts fit");

    assert!(summary.contains("first line\n2026-11-17 19:00:01 [info] second line"));
}

#[cfg(unix)]
#[test]
fn git_paths_preserve_non_utf8_bytes_in_changed_and_untracked_names() {
    use std::os::unix::ffi::OsStringExt;

    let mut input = sample_input();
    let path = std::path::PathBuf::from(std::ffi::OsString::from_vec(b"log-\xff".to_vec()));
    let git = input.git.first_mut().expect("worktree");
    git.uncommitted_files = vec![path.clone()];
    git.newest_untracked.first_mut().expect("untracked file").path = path;

    let summary = render_summary(&input, UtcOffset::UTC).expect("mandatory summary facts fit");

    assert_eq!(summary.matches(r"log-\xFF").count(), 2);
    assert!(!summary.contains('\u{fffd}'));
}

#[test]
fn gate_records_include_full_commit_finished_time_and_log_path() {
    let input = sample_input();
    let offset = UtcOffset::UTC;

    let summary = render_summary(&input, offset).expect("mandatory summary facts fit");

    assert!(
        summary.contains("0123456789abcdef0123456789abcdef01234567"),
        "gate section must include full commit hash"
    );
    assert!(
        summary.contains("/tmp/gate.log"),
        "gate section must include gate log path"
    );
    assert!(
        summary.contains("2026-11-17 19:50:00 UTC"),
        "gate section must include finished timestamp"
    );
}

#[test]
fn format_time_handles_offset_seconds_and_extreme_timestamp_fallback() {
    let mut input = sample_input();
    input.collected_at = ts("2026-11-17T20:00:00Z");
    let offset_seconds = UtcOffset::from_hms(2, 30, 15).expect("valid offset with seconds");

    let summary = render_summary(&input, offset_seconds).expect("mandatory summary facts fit");
    assert!(
        summary.contains("+02:30:15"),
        "offset string must include seconds when non-zero"
    );

    // Extreme timestamps: year 0000 with negative offset underflows
    let early = ts("0000-01-01T00:00:00Z");
    let extreme_neg = UtcOffset::from_hms(-12, 0, 0).expect("valid negative offset");
    let formatted_early = format_time(early, extreme_neg);
    assert!(
        formatted_early.contains("out of range"),
        "underflowing local time must produce explicit out of range notice without panicking"
    );

    // Extreme timestamps: year 9999 with positive offset overflows
    let late = ts("9999-12-31T23:59:59Z");
    let extreme_pos = UtcOffset::from_hms(14, 0, 0).expect("valid positive offset");
    let formatted_late = format_time(late, extreme_pos);
    assert!(
        formatted_late.contains("out of range"),
        "overflowing local time must produce explicit out of range notice without panicking"
    );
}

#[test]
fn filesystem_values_above_100_percent_render_raw_observed() {
    let mut input = sample_input();
    input.machine.home_fs = Some(FsUsageSummary {
        space_percent: 105,
        inode_percent: 110,
    });
    let offset = UtcOffset::UTC;

    let summary = render_summary(&input, offset).expect("mandatory summary facts fit");

    assert!(
        summary.contains("105% space, 110% inodes"),
        "must render raw observed filesystem percentage above 100%"
    );
}

#[test]
fn newer_executable_and_closed_round_are_rendered_with_context() {
    let mut input = sample_input();
    input.daemon.as_mut().unwrap().disk_newer_than_process = true;
    input.team.rounds.first_mut().unwrap().status = RoundLife::Closed {
        at: ts("2026-11-17T19:55:00Z"),
        how: crate::state::RoundEnd::Approved,
    };
    let summary = render_summary(&input, UtcOffset::UTC).expect("mandatory summary facts fit");
    assert!(summary.contains("  (Warning: binary on disk is newer than running process)\n"));
    assert!(summary.contains("closed (approved) at 2026-11-17 19:55:00 UTC (2026-11-17 19:55:00 +00:00)"));
}

#[test]
fn gate_tree_outcomes_and_missing_record_are_rendered_exactly() {
    for (outcome, expected) in [
        (
            GateOutcome::Failed {
                exit_code: std::num::NonZeroI32::new(17).unwrap(),
            },
            "failed (exit 17)",
        ),
        (GateOutcome::TimedOut, "timed out"),
    ] {
        let mut input = sample_input();
        let record = input.gate.first_mut().unwrap().record.as_mut().unwrap();
        record.tree = GateTree::NotClean;
        record.result.outcome = outcome;
        let summary = render_summary(&input, UtcOffset::UTC).expect("mandatory summary facts fit");
        assert!(summary.contains(&format!(
            "- Record: Commit `0123456789abcdef0123456789abcdef01234567`, tree dirty, outcome: {expected}\n"
        )));
    }
    let mut input = sample_input();
    input.gate.first_mut().unwrap().record = None;
    let summary = render_summary(&input, UtcOffset::UTC).expect("mandatory summary facts fit");
    assert!(summary.contains("- Record: none\n- Recent gate outputs:\n"));
}

#[test]
fn every_distinct_anomaly_is_retained_once_even_above_old_section_budget() {
    let mut input = sample_input();
    input.anomalies = (0..500)
        .map(|i| SummaryAnomaly {
            message: format!("unique anomaly {i}: missing branch integration"),
            command: None,
        })
        .collect();
    input.anomalies.push(input.anomalies.first().unwrap().clone());
    let summary = render_summary(&input, UtcOffset::UTC).unwrap();
    for anomaly in &input.anomalies {
        assert_eq!(
            summary
                .lines()
                .filter(|line| *line == format!("- {}", anomaly.message))
                .count(),
            1
        );
    }
}

#[test]
fn short_document_keeps_text_above_old_per_section_limit_without_cutting() {
    let mut input = sample_input();
    let history = format!("history start\n{}history end", "complete line\n".repeat(1500));
    input.history_tail = Some(history.clone());
    let summary = render_summary(&input, UtcOffset::UTC).unwrap();
    assert!(summary.contains(&history));
    assert!(!summary.contains(CUT_MARKER));
}

#[test]
fn long_individual_tails_preserve_every_task_gate_record_and_fence() {
    let mut input = sample_input();
    let detail = format!("{}RUN-END", "long output 🦀\n".repeat(20_000));
    input.tasks.first_mut().unwrap().last_run_end_detail = Some(detail.clone());
    input.tasks.first_mut().unwrap().questions.as_mut().unwrap().content = format!("{detail}ANSWER-END");
    let mut second = sample_task();
    second.id = "task-20261117-200000-2222".parse().unwrap();
    input.tasks.push(second.clone());
    let first_gate = input.gate.first_mut().unwrap();
    first_gate.recent_output_tails = vec![
        "discard-fourth-oldest".to_owned(),
        format!("{detail}GATE-1-END"),
        format!("{detail}GATE-2-END"),
        format!("{detail}GATE-3-END"),
    ];
    let mut second_gate = first_gate.clone();
    second_gate.task_id = second.id;
    second_gate.recent_output_tails.clear();
    input.gate.push(second_gate);
    input.history_tail = Some(format!("{detail}HISTORY-END"));
    input.daemon_log_lines = Some(vec![format!("{detail}DAEMON-END")]);
    input.bus_messages = vec![format!("{detail}BUS-END")];
    input.notifications = vec![format!("{detail}NOTIFICATION-END")];
    let summary = render_summary(&input, UtcOffset::UTC).unwrap();
    assert!(summary.len() < MAX_SUMMARY_BYTES);
    assert!(summary.contains(CUT_MARKER));
    for task in &input.tasks {
        assert!(summary.contains(&format!("### Task `{}` (`{}`)\n", task.id, task.agent)));
        assert!(summary.contains(&format!("### Gate for Task `{}`\n", task.id)));
    }
    assert_eq!(summary.matches("- Record: Commit").count(), 2);
    assert_eq!(summary.lines().filter(|line| *line == "```text").count(), 3);
    assert_eq!(summary.lines().filter(|line| *line == "```").count(), 3);
    assert!(!summary.contains("discard-fourth-oldest"));
    for end in [
        "RUN-END",
        "ANSWER-END",
        "GATE-1-END",
        "GATE-2-END",
        "GATE-3-END",
        "HISTORY-END",
        "DAEMON-END",
        "BUS-END",
        "NOTIFICATION-END",
    ] {
        assert!(summary.contains(end), "missing {end}");
    }
}

#[test]
fn task_table_header_and_all_rows_survive_large_task_counts() {
    let mut input = sample_input();
    input.tasks = (0..100)
        .map(|i| {
            let mut task = sample_task();
            task.id = format!("task-20261117-200000-{i:04}").parse().unwrap();
            task
        })
        .collect();
    let summary = render_summary(&input, UtcOffset::UTC).unwrap();
    assert!(
        summary.contains("| Task | Agent | Role | Status | Runs | Started | Ended | Last Activity | Minutes Since |\n")
    );
    for task in &input.tasks {
        assert_eq!(summary.matches(&format!("| {} |", task.id)).count(), 1);
    }
}

#[test]
fn mandatory_facts_over_budget_return_an_error_instead_of_dropping_anomalies() {
    let mut input = sample_input();
    input.anomalies = vec![SummaryAnomaly {
        message: "x".repeat(MAX_SUMMARY_BYTES),
        command: None,
    }];
    assert!(
        matches!(render_summary(&input, UtcOffset::UTC), Err(SummaryError::RequiredFactsTooLarge { required_bytes }) if required_bytes >= MAX_SUMMARY_BYTES)
    );
}

#[test]
fn status_and_phase_use_cli_display_spellings() {
    let input = sample_input();
    let summary = render_summary(&input, UtcOffset::UTC).unwrap();
    assert!(summary.contains("| member | running |"));
    assert!(summary.contains("| in_progress | gemini-1 |"));
}

#[test]
fn gate_log_path_escapes_quotes_losslessly() {
    let mut input = sample_input();
    input.gate.first_mut().unwrap().record.as_mut().unwrap().result.log = "/tmp/log-\"quoted\"".parse().unwrap();
    let summary = render_summary(&input, UtcOffset::UTC).unwrap();
    assert!(summary.contains(r#"Log: `"/tmp/log-\"quoted\""`"#));
}

/// Builds a document whose mandatory facts and minimal tails occupy exactly the requested bytes.
fn input_at_minimum_summary_bytes(required_bytes: usize, short_tail: &str) -> SummaryInput {
    let mut input = sample_input();
    input.tasks.clear();
    input.gate.clear();
    input.history_tail = Some(String::new());
    input.daemon_log_lines = None;
    input.bus_messages = vec![short_tail.to_owned()];
    input.notifications.clear();
    input.anomalies = vec![SummaryAnomaly {
        message: String::new(),
        command: None,
    }];
    let baseline = render_summary(&input, UtcOffset::UTC).unwrap().len();
    input.anomalies.first_mut().unwrap().message = "a".repeat(required_bytes - baseline - CUT_MARKER.len());
    input.history_tail = Some("history".repeat(MAX_SUMMARY_BYTES));
    input
}

#[test]
fn summary_budget_one_byte_below_limit_preserves_short_tail_and_exact_cut_marker() {
    for short_tail in ["", "tiny", "🦀"] {
        let input = input_at_minimum_summary_bytes(MAX_SUMMARY_BYTES - 1, short_tail);
        let summary = render_summary(&input, UtcOffset::UTC).unwrap();
        assert_eq!(summary.len(), MAX_SUMMARY_BYTES - 1);
        assert_eq!(summary.matches(CUT_MARKER).count(), 1);
        let bus = summary.split(SummarySection::Bus.heading()).nth(1).unwrap();
        assert!(bus.starts_with(&format!("\n\n- {short_tail}\n")));
    }
}

#[test]
fn summary_budget_at_limit_returns_exact_required_bytes_with_short_and_long_tails() {
    for short_tail in ["", "tiny", "🦀"] {
        let input = input_at_minimum_summary_bytes(MAX_SUMMARY_BYTES, short_tail);
        assert_eq!(
            render_summary(&input, UtcOffset::UTC),
            Err(SummaryError::RequiredFactsTooLarge {
                required_bytes: MAX_SUMMARY_BYTES,
            })
        );
    }
}

#[test]
fn deferred_sections_say_not_collected_instead_of_reporting_empty_observations() {
    let input = sample_input();
    let sections = [
        SummarySection::DaemonAndVersions,
        SummarySection::DaemonLog,
        SummarySection::Bus,
        SummarySection::Notifications,
        SummarySection::Machine,
    ];
    let rendered = render_summary_with_uncollected(&input, UtcOffset::UTC, &sections).unwrap();
    for section in sections {
        let body = rendered
            .split(section.heading())
            .nth(1)
            .unwrap()
            .split("\n## ")
            .next()
            .unwrap();
        assert_eq!(body.trim(), "Not collected in this version.");
    }
    assert!(rendered.contains("Summary renderer"));
    assert!(rendered.contains("none of the known ones"));
}

#[test]
fn deferred_machine_section_keeps_collection_failures_visible() {
    let mut input = sample_input();
    input.unreadable_parts.push(UnreadablePartSummary {
        name: "worktree".into(),
        reason: "removed".into(),
    });
    let rendered = render_summary_with_uncollected(&input, UtcOffset::UTC, &[SummarySection::Machine]).unwrap();
    assert!(rendered.contains("Not collected in this version."));
    assert!(rendered.contains("### Unreadable Parts\n\n- `worktree`: removed"));
    assert!(!rendered.contains("Load average"));
}

#[test]
fn deferred_untracked_metadata_does_not_claim_zero_files() {
    let mut input = sample_input();
    input.git[0].newest_untracked.clear();
    let summary = render_summary_with_coverage(&input, UtcOffset::UTC, &[], UntrackedCollection::Deferred).unwrap();
    assert!(summary.contains("Untracked file metadata: Not collected in this version."));
    assert!(!summary.contains("Untracked files (newest 0)"));
    assert!(summary.contains("Commits over base"));
}

#[test]
fn redacts_optional_text_before_truncation_and_byte_budgeting() {
    let mut input = sample_input();
    input.history_tail = Some("xy".repeat(100_000));
    let mut expected = input.clone();
    expected.history_tail = Some("<cut>".repeat(100_000));
    assert_eq!(
        render_redacted_summary(&input, UtcOffset::UTC, &[], UntrackedCollection::Collected, |text| text
            .replace("xy", "<cut>")),
        render_summary(&expected, UtcOffset::UTC)
    );
}

#[test]
fn redacted_diagnostic_paths_are_plain_and_unread_history_is_named_in_its_section() {
    let mut input = sample_input();
    input.git[0].uncommitted_files = vec![PathBuf::from("café file.txt")];
    input.gate[0].record.as_mut().unwrap().result.log = "/tmp/gate.log".parse().unwrap();
    input.history_tail = None;
    input.unreadable_parts.push(UnreadablePartSummary {
        name: "team history".into(),
        reason: "database unavailable".into(),
    });
    let summary = render_redacted_summary(
        &input,
        UtcOffset::from_hms(2, 0, 0).unwrap(),
        &[],
        UntrackedCollection::Collected,
        str::to_owned,
    )
    .unwrap();
    assert!(summary.contains("Uncommitted files (1): café file.txt"), "{summary}");
    assert!(summary.contains("Log: `/tmp/gate.log`"), "{summary}");
    let history = summary
        .split("## Team History")
        .nth(1)
        .unwrap()
        .split("## Daemon Log")
        .next()
        .unwrap();
    assert!(
        history.contains("Could not read team history: database unavailable"),
        "{history}"
    );
    assert!(!history.contains("No history recorded"));
    assert!(summary.contains("+02:00"));
}
