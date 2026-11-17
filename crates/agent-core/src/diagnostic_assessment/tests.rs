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
        }),
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
                    part: "git".into(),
                    reason: "worktree removed".into(),
                }];
                f
            },
            vec![Anomaly::Unreadable(ReadFailure {
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
