//! Assessment contracts exercised entirely through collected facts.

use super::*;
use crate::state::{Completion, GateOutcome, GateResult, GateTree};

fn task_id() -> TaskId {
    "task-20261117-204920-lead".parse().unwrap()
}

fn head() -> CommitId {
    "a".repeat(40).parse().unwrap()
}

fn gate() -> GateRecord {
    GateRecord {
        tree: GateTree::Clean,
        result: GateResult {
            head: head(),
            finished_at: "2026-11-17T12:00:00Z".parse().unwrap(),
            outcome: GateOutcome::Passed,
            log: "/tmp/gate.log".parse().unwrap(),
        },
    }
}

fn healthy() -> Assessment {
    Assessment {
        now: "2026-11-17T12:00:00Z".parse().unwrap(),
        stale_after: Duration::from_secs(600),
        daemon: Some(Daemon::Present {
            executable_newer: Some(false),
            version: None,
        }),
        command_version: None,
        lead: task_id(),
        tasks: vec![Task {
            id: task_id(),
            status: TaskStatus::Running,
            last_activity: None,
        }],
        items: vec![ItemPhase::Done {
            completion: Completion::Listed,
        }],
        lead_git: Some(LeadGit {
            tip: head(),
            dirty: false,
            gate: Some(gate()),
        }),
        branches: vec![],
        filesystems: vec![],
        unanswered: vec![],
        unreadable: vec![],
    }
}

fn progress() -> ItemPhase {
    ItemPhase::InProgress {
        assignee: "member".parse().unwrap(),
        branch: "agents/member/task".parse().unwrap(),
    }
}

fn branch() -> Branch {
    Branch {
        task: "task-20261117-204920-memb".parse().unwrap(),
        name: "agents/member/task".parse().unwrap(),
        tip: head(),
        origin: Some(Origin::Tip(head())),
        contained_in_lead: Some(true),
    }
}

#[test]
#[expect(
    clippy::too_many_lines,
    reason = "one table keeps every anomaly case and its expected context together"
)]
fn every_known_anomaly_is_reported_exactly_with_its_context() {
    let cases: Vec<(Assessment, Vec<Anomaly>)> = vec![
        (
            {
                let mut f = healthy();
                f.daemon = Some(Daemon::Absent);
                f
            },
            vec![Anomaly::NoDaemon],
        ),
        (
            {
                let mut f = healthy();
                f.daemon = Some(Daemon::Present {
                    executable_newer: Some(true),
                    version: None,
                });
                f
            },
            vec![Anomaly::NewerExecutable],
        ),
        (
            {
                let mut f = healthy();
                f.tasks[0].status = TaskStatus::Failed;
                f
            },
            vec![Anomaly::FailedTask(task_id())],
        ),
        (
            {
                let mut f = healthy();
                f.tasks[0].status = TaskStatus::Paused;
                f
            },
            vec![Anomaly::PausedTask(task_id())],
        ),
        (
            {
                let mut f = healthy();
                f.tasks[0].last_activity = Some("2026-11-17T11:49:59Z".parse().unwrap());
                f
            },
            vec![Anomaly::StaleTask(task_id())],
        ),
        (
            {
                let mut f = healthy();
                f.tasks.clear();
                f.items = vec![ItemPhase::Pending];
                f
            },
            vec![Anomaly::NobodyWorking],
        ),
        (
            {
                let mut f = healthy();
                f.unanswered = vec!["questions-1.md".into()];
                f
            },
            vec![Anomaly::UnansweredQuestions("questions-1.md".into())],
        ),
        (
            {
                let mut f = healthy();
                f.lead_git.as_mut().unwrap().dirty = true;
                f
            },
            vec![Anomaly::DirtyLead],
        ),
        (
            {
                let mut f = healthy();
                f.lead_git.as_mut().unwrap().gate.as_mut().unwrap().tree = GateTree::NotClean;
                f
            },
            vec![Anomaly::DirtyGate, Anomaly::NoGatePassAtTip],
        ),
        (
            {
                let mut f = healthy();
                f.lead_git.as_mut().unwrap().gate.as_mut().unwrap().result.head = "b".repeat(40).parse().unwrap();
                f
            },
            vec![Anomaly::GateAtOtherCommit, Anomaly::NoGatePassAtTip],
        ),
        (
            {
                let mut f = healthy();
                f.lead_git.as_mut().unwrap().gate.as_mut().unwrap().result.outcome = GateOutcome::TimedOut;
                f
            },
            vec![Anomaly::NoGatePassAtTip],
        ),
        (
            {
                let mut f = healthy();
                f.lead_git.as_mut().unwrap().gate = None;
                f
            },
            vec![Anomaly::NoGatePassAtTip],
        ),
        (
            {
                let mut f = healthy();
                f.items = vec![progress()];
                f.tasks[0].status = TaskStatus::Waiting;
                f.tasks.push(Task {
                    id: branch().task,
                    status: TaskStatus::Queued,
                    last_activity: None,
                });
                f
            },
            vec![Anomaly::WaitingWithoutPr],
        ),
        (
            {
                let mut f = healthy();
                f.items = vec![progress()];
                let mut b = branch();
                b.contained_in_lead = Some(false);
                f.branches = vec![b];
                f
            },
            vec![Anomaly::UnmergedBranch(branch().name)],
        ),
        (
            {
                let mut f = healthy();
                let mut b = branch();
                b.origin = Some(Origin::Missing);
                f.branches = vec![b];
                f
            },
            vec![Anomaly::OriginDiffers(branch().name)],
        ),
        (
            {
                let mut f = healthy();
                let mut b = branch();
                b.origin = Some(Origin::Tip("b".repeat(40).parse().unwrap()));
                f.branches = vec![b];
                f
            },
            vec![Anomaly::OriginDiffers(branch().name)],
        ),
        (
            {
                let mut f = healthy();
                f.filesystems = vec![Filesystem {
                    path: "/home/żółć".into(),
                    space: Some(Usage {
                        used: 90,
                        total: NonZeroU64::new(100).unwrap(),
                    }),
                    inodes: None,
                }];
                f
            },
            vec![Anomaly::FullFilesystem("/home/żółć".into())],
        ),
        (
            {
                let mut f = healthy();
                f.filesystems = vec![Filesystem {
                    path: "/tmp".into(),
                    space: None,
                    inodes: Some(Usage {
                        used: 90,
                        total: NonZeroU64::new(100).unwrap(),
                    }),
                }];
                f
            },
            vec![Anomaly::FullFilesystem("/tmp".into())],
        ),
        (
            {
                let mut f = healthy();
                f.unreadable = vec![ReadFailure {
                    section: crate::debug_summary::SummarySection::Machine,
                    part: "git".into(),
                    reason: "worktree removed".into(),
                }];
                f
            },
            vec![Anomaly::Unreadable(ReadFailure {
                section: crate::debug_summary::SummarySection::Machine,
                part: "git".into(),
                reason: "worktree removed".into(),
            })],
        ),
    ];
    for (facts, expected) in cases {
        assert_eq!(assess(&facts), expected, "{facts:?}");
    }
}

#[test]
fn healthy_and_unknown_facts_do_not_imply_anomalies() {
    let mut facts = healthy();
    assert_eq!(assess(&facts), vec![]);
    facts.daemon = None;
    facts.lead_git = None;
    facts.branches = vec![Branch {
        origin: None,
        contained_in_lead: None,
        ..branch()
    }];
    facts.filesystems = vec![Filesystem {
        path: "/tmp".into(),
        space: None,
        inodes: None,
    }];
    assert_eq!(assess(&facts), vec![]);
}

#[test]
fn stale_output_uses_strict_injected_threshold_only_for_running_tasks() {
    for (status, activity, seconds, expected) in [
        (TaskStatus::Running, Some("2026-11-17T11:50:00Z"), 600, vec![]),
        (TaskStatus::Running, Some("2026-11-17T11:50:00.000000001Z"), 600, vec![]),
        (
            TaskStatus::Running,
            Some("2026-11-17T11:49:59.999999999Z"),
            600,
            vec![Anomaly::StaleTask(task_id())],
        ),
        (TaskStatus::Running, Some("2026-11-17T12:00:01Z"), 0, vec![]),
        (TaskStatus::Running, None, 0, vec![]),
        (
            TaskStatus::Running,
            Some("2026-11-17T11:59:59Z"),
            0,
            vec![Anomaly::StaleTask(task_id())],
        ),
        (
            TaskStatus::Running,
            Some("2026-11-17T11:58:59Z"),
            60,
            vec![Anomaly::StaleTask(task_id())],
        ),
        (TaskStatus::Queued, Some("2026-11-17T00:00:00Z"), 600, vec![]),
        (TaskStatus::Waiting, Some("2026-11-17T00:00:00Z"), 600, vec![]),
        (TaskStatus::Questions, Some("2026-11-17T00:00:00Z"), 600, vec![]),
        (TaskStatus::Stopped, Some("2026-11-17T00:00:00Z"), 600, vec![]),
        (TaskStatus::Done, Some("2026-11-17T00:00:00Z"), 600, vec![]),
    ] {
        let mut facts = healthy();
        facts.tasks[0].status = status;
        facts.tasks[0].last_activity = activity.map(|text| text.parse().unwrap());
        facts.stale_after = Duration::from_secs(seconds);
        assert_eq!(assess(&facts), expected);
    }
}

#[test]
fn filesystem_thresholds_are_exact_overflow_safe_and_deduplicated() {
    for (used, total, full) in [
        (0, 1, false),
        (89, 100, false),
        (899, 1000, false),
        (9, 10, true),
        (1, 1, true),
        (u64::MAX, u64::MAX, true),
        (u64::MAX, 1, true),
    ] {
        let mut facts = healthy();
        let usage = Some(Usage {
            used,
            total: NonZeroU64::new(total).unwrap(),
        });
        facts.filesystems = vec![
            Filesystem {
                path: "/tmp".into(),
                space: usage,
                inodes: usage
            };
            2
        ];
        assert_eq!(
            assess(&facts),
            if full {
                vec![Anomaly::FullFilesystem("/tmp".into())]
            } else {
                vec![]
            }
        );
    }
}

#[test]
fn unanswered_questions_read_failures_and_branch_anomalies_are_not_duplicated() {
    let mut facts = healthy();
    let failure = ReadFailure {
        section: crate::debug_summary::SummarySection::Machine,
        part: "output".into(),
        reason: "cannot read".into(),
    };
    facts.unanswered = vec!["questions.md".into(); 2];
    facts.unreadable = vec![failure.clone(); 2];
    let b = Branch {
        origin: Some(Origin::Missing),
        ..branch()
    };
    facts.branches = vec![b; 2];
    assert_eq!(
        assess(&facts),
        vec![
            Anomaly::UnansweredQuestions("questions.md".into()),
            Anomaly::OriginDiffers(branch().name),
            Anomaly::Unreadable(failure)
        ]
    );
}

#[test]
fn branch_containment_requires_an_unfinished_assignment_and_a_non_lead_task() {
    let mut facts = healthy();
    facts.branches = vec![Branch {
        contained_in_lead: Some(false),
        ..branch()
    }];
    assert_eq!(assess(&facts), vec![]);
    facts.items = vec![progress()];
    facts.branches[0].contained_in_lead = None;
    assert_eq!(assess(&facts), vec![]);
    facts.branches[0].contained_in_lead = Some(false);
    facts.branches[0].task = task_id();
    assert_eq!(assess(&facts), vec![]);
}

#[test]
fn waiting_without_pr_requires_the_lead_a_passing_tip_and_an_in_progress_item() {
    let mut facts = healthy();
    facts.items = vec![progress()];
    facts.tasks.push(Task {
        id: branch().task,
        status: TaskStatus::Waiting,
        last_activity: None,
    });
    assert_eq!(assess(&facts), vec![]);
    facts.tasks[0].status = TaskStatus::Waiting;
    assert_eq!(assess(&facts), vec![Anomaly::NobodyWorking, Anomaly::WaitingWithoutPr]);
    facts.lead_git.as_mut().unwrap().gate = None;
    assert_eq!(assess(&facts), vec![Anomaly::NobodyWorking, Anomaly::NoGatePassAtTip]);
    facts.items = vec![ItemPhase::Pending];
    facts.lead_git.as_mut().unwrap().gate = Some(gate());
    assert_eq!(assess(&facts), vec![Anomaly::NobodyWorking]);
}

#[test]
fn each_anomaly_has_a_sentence_and_task_findings_offer_a_log_command() {
    let cases = [
        (Anomaly::NoDaemon, "No daemon holds this home.", None),
        (
            Anomaly::NewerExecutable,
            "The daemon executable on disk is newer than the running process.",
            None,
        ),
        (
            Anomaly::FailedTask(task_id()),
            "Task task-20261117-204920-lead is failed.",
            Some("agent log --task task-20261117-204920-lead"),
        ),
        (
            Anomaly::PausedTask(task_id()),
            "Task task-20261117-204920-lead is paused.",
            Some("agent log --task task-20261117-204920-lead"),
        ),
        (
            Anomaly::StaleTask(task_id()),
            "Running task task-20261117-204920-lead has output older than the inactivity threshold.",
            Some("agent log --task task-20261117-204920-lead"),
        ),
        (
            Anomaly::NobodyWorking,
            "No team task is running or queued while an item remains unfinished.",
            None,
        ),
        (
            Anomaly::UnansweredQuestions("questions.md".into()),
            "Questions file \"questions.md\" has no answer yet.",
            None,
        ),
        (Anomaly::DirtyLead, "The lead worktree has uncommitted changes.", None),
        (
            Anomaly::DirtyGate,
            "The lead's last gate ran on a tree that was not clean.",
            None,
        ),
        (
            Anomaly::GateAtOtherCommit,
            "The lead's gate record belongs to a different commit than its tip.",
            None,
        ),
        (
            Anomaly::NoGatePassAtTip,
            "The lead has no clean gate pass recorded at its tip.",
            None,
        ),
        (
            Anomaly::WaitingWithoutPr,
            "The lead waits with a gate passed at its tip and an item in progress, but no pull request is recorded.",
            None,
        ),
        (
            Anomaly::UnmergedBranch(branch().name),
            "The lead branch does not contain assigned unfinished branch agents/member/task.",
            None,
        ),
        (
            Anomaly::OriginDiffers(branch().name),
            "Branch agents/member/task has a local tip that differs from its recorded origin ref.",
            None,
        ),
        (
            Anomaly::FullFilesystem("/tmp".into()),
            "Filesystem \"/tmp\" is at least 90% full by space or inodes.",
            None,
        ),
        (
            Anomaly::Unreadable(ReadFailure {
                section: crate::debug_summary::SummarySection::Machine,
                part: "prompt".into(),
                reason: "file gone".into(),
            }),
            "Could not collect prompt: file gone.",
            None,
        ),
    ];
    for (anomaly, sentence, command) in cases {
        assert_eq!(anomaly.sentence(), sentence);
        assert_eq!(anomaly.suggested_command().as_deref(), command);
    }
}

#[test]
fn an_old_merged_item_does_not_hide_a_current_waiting_round_without_a_pr() {
    use crate::state::{PrNumber, PullRequest};
    let mut facts = healthy();
    facts.tasks[0].status = TaskStatus::Waiting;
    facts.items = vec![
        progress(),
        ItemPhase::Done {
            completion: Completion::Merged {
                pr: PullRequest {
                    number: PrNumber::new(std::num::NonZeroU32::new(1).unwrap()),
                    branch: branch().name,
                    opened_at: facts.now,
                },
                at: facts.now,
            },
        },
    ];
    assert_eq!(assess(&facts), vec![Anomaly::NobodyWorking, Anomaly::WaitingWithoutPr]);
}

mod retained_assessment {
    use super::*;

    #[test]
    fn returns_typed_assessment_with_anomalies() {
        let mut facts = healthy();
        facts.daemon = Some(Daemon::Absent);

        assert_eq!(assess(&facts), vec![Anomaly::NoDaemon]);
    }

    #[test]
    fn detects_daemon_offline() {
        let mut facts = healthy();
        facts.daemon = Some(Daemon::Absent);

        assert_eq!(assess(&facts), vec![Anomaly::NoDaemon]);
    }

    #[test]
    fn detects_no_anomalies_when_all_healthy() {
        assert_eq!(assess(&healthy()), vec![]);
    }

    #[test]
    fn never_infers_anomaly_from_missing_facts() {
        let mut facts = healthy();
        facts.daemon = None;
        facts.lead_git = None;

        assert_eq!(assess(&facts), vec![]);
    }

    #[test]
    fn deduplicates_anomalies_per_task_category() {
        let mut facts = healthy();
        let failed = Task {
            id: task_id(),
            status: TaskStatus::Failed,
            last_activity: None,
        };
        facts.tasks = vec![failed.clone(), failed];

        assert_eq!(assess(&facts), vec![Anomaly::FailedTask(task_id())]);
    }
}

mod retained_anomalies {
    use super::*;

    #[test]
    fn no_anomalies_when_all_healthy() {
        assert_eq!(assess(&healthy()), vec![]);
    }

    #[test]
    fn daemon_offline_detected() {
        let mut facts = healthy();
        facts.daemon = Some(Daemon::Absent);

        assert_eq!(assess(&facts), vec![Anomaly::NoDaemon]);
    }

    #[test]
    fn daemon_stale_detected() {
        let mut facts = healthy();
        facts.daemon = Some(Daemon::Present {
            executable_newer: Some(true),
            version: None,
        });

        assert_eq!(assess(&facts), vec![Anomaly::NewerExecutable]);
    }

    #[test]
    fn task_failed_detected() {
        let mut facts = healthy();
        facts.tasks = vec![Task {
            id: task_id(),
            status: TaskStatus::Failed,
            last_activity: None,
        }];

        assert_eq!(assess(&facts), vec![Anomaly::FailedTask(task_id())]);
    }

    #[test]
    fn open_questions_detected() {
        let mut facts = healthy();
        let path = PathBuf::from("tasks/member/questions-1.md");
        facts.unanswered = vec![path.clone()];

        assert_eq!(assess(&facts), vec![Anomaly::UnansweredQuestions(path)]);
    }

    #[test]
    fn lead_dirty_detected() {
        let mut facts = healthy();
        facts.lead_git.as_mut().unwrap().dirty = true;

        assert_eq!(assess(&facts), vec![Anomaly::DirtyLead]);
    }

    #[test]
    fn filesystem_full_detected() {
        let mut facts = healthy();
        let path = PathBuf::from("/home");
        facts.filesystems = vec![Filesystem {
            path: path.clone(),
            space: Some(Usage {
                used: 95,
                total: NonZeroU64::new(100).unwrap(),
            }),
            inodes: None,
        }];

        assert_eq!(assess(&facts), vec![Anomaly::FullFilesystem(path)]);
    }
}

fn review_phases() -> Vec<ItemPhase> {
    let pr = crate::state::PullRequest {
        number: crate::state::PrNumber::new(std::num::NonZeroU32::new(12).unwrap()),
        branch: branch().name,
        opened_at: healthy().now,
    };
    vec![
        ItemPhase::InReview {
            pr: pr.clone(),
            reviews: vec![],
        },
        ItemPhase::Fixing {
            assignee: "member".parse().unwrap(),
            branch: branch().name,
            pr: pr.clone(),
        },
        ItemPhase::Approved { pr, reviews: vec![] },
    ]
}

#[test]
fn review_fixing_and_approved_items_are_unfinished_when_nobody_works() {
    for phase in review_phases() {
        let mut facts = healthy();
        facts.tasks.clear();
        facts.items = vec![phase];
        assert_eq!(assess(&facts), vec![Anomaly::NobodyWorking]);
    }
}

#[test]
fn recorded_pull_requests_prevent_waiting_without_pr_even_with_another_item_in_progress() {
    for phase in review_phases() {
        let mut facts = healthy();
        facts.tasks.first_mut().unwrap().status = TaskStatus::Waiting;
        facts.items = vec![progress(), phase];
        assert_eq!(assess(&facts), vec![Anomaly::NobodyWorking]);
    }
}

#[test]
fn only_fixing_has_an_assigned_branch_among_review_phases() {
    for phase in review_phases() {
        let expected = match phase {
            ItemPhase::Fixing { .. } => vec![Anomaly::UnmergedBranch(branch().name)],
            ItemPhase::InReview { .. } | ItemPhase::Approved { .. } => vec![],
            ItemPhase::Pending | ItemPhase::InProgress { .. } | ItemPhase::Done { .. } => panic!("unexpected fixture"),
        };
        let mut facts = healthy();
        facts.items = vec![phase];
        let mut member = branch();
        member.contained_in_lead = Some(false);
        facts.branches = vec![member];
        assert_eq!(assess(&facts), expected);
    }
}

#[test]
fn assess_finds_version_mismatch_anomaly_when_daemon_and_command_versions_differ() {
    let mut facts = healthy();
    facts.command_version = Some("agent 0.2.0".to_owned());
    facts.daemon = Some(Daemon::Present {
        executable_newer: Some(false),
        version: Some("agent 0.1.0".to_owned()),
    });
    let anomalies = assess(&facts);
    assert_eq!(
        anomalies,
        vec![Anomaly::VersionMismatch {
            command: "agent 0.2.0".to_owned(),
            daemon: "agent 0.1.0".to_owned(),
        }]
    );
    assert_eq!(
        anomalies[0].sentence(),
        "The daemon was started from another build (agent 0.1.0) than the command (agent 0.2.0); restart the daemon or run the command from the daemon's build."
    );
}

#[test]
fn assess_no_version_mismatch_when_daemon_absent_or_versions_match() {
    let mut facts = healthy();
    facts.command_version = Some("agent 0.2.0".to_owned());
    facts.daemon = Some(Daemon::Absent);
    let anomalies = assess(&facts);
    assert!(!anomalies.iter().any(|a| matches!(a, Anomaly::VersionMismatch { .. })));

    let mut facts2 = healthy();
    facts2.command_version = Some("agent 0.2.0".to_owned());
    facts2.daemon = Some(Daemon::Present {
        executable_newer: Some(false),
        version: Some("agent 0.2.0".to_owned()),
    });
    let anomalies2 = assess(&facts2);
    assert!(!anomalies2.iter().any(|a| matches!(a, Anomaly::VersionMismatch { .. })));
}
