//! The pool workers of a team in a diagnostic bundle. A worker's task is no member of the team, so the history names
//! it: every run of it has a row with the team of the requester and the role `worker`.

use std::path::{Path, PathBuf};

use agent_core::history::{Record, RunWhy, SequencedRecord};
use agent_core::ids::{RunId, TaskId, TeamId};
use agent_core::kinds::Role;
use agent_core::state::{RunState, Timestamp};

use super::{Collection, escaped_path, run_detail_at};
use crate::run::context::CONTEXT_FILE;
use crate::store::Home;

/// How many of its newest runs the bundle keeps the output of, for each worker task.
const WORKER_RUNS_KEPT: usize = 3;
/// The file of a run that holds the CLI's standard output.
const OUTPUT_FILE: &str = "out.jsonl";
/// The file of a run that holds the CLI's standard error.
const ERROR_FILE: &str = "err.log";

/// One run of a worker task, as the history says it.
struct WorkerRun {
    /// The run.
    run: RunId,
    /// When it started.
    started: Timestamp,
    /// The run of the requester that asked for it, when the state knew it.
    asked_by: Option<RunId>,
    /// Whether it goes on or how it ended.
    state: RunState,
}

/// A worker task and its runs, oldest first.
struct WorkerTask {
    /// The task.
    task: TaskId,
    /// The agent of its latest run.
    agent: String,
    /// Its runs in the order the history wrote them; never empty.
    runs: Vec<WorkerRun>,
}

/// The worker tasks of `team` that `records` name, in the order of their first run.
fn worker_tasks(records: &[SequencedRecord], team: &TeamId) -> Vec<WorkerTask> {
    let mut tasks: Vec<WorkerTask> = Vec::new();
    for stored in records {
        let Record::Run(start) = &stored.record else {
            continue;
        };
        if start.role != Role::Worker || start.team != *team {
            continue;
        }
        let run = WorkerRun {
            run: start.run.clone(),
            started: start.at,
            asked_by: match &start.why {
                RunWhy::Request { asked_by } => asked_by.clone(),
                _ => None,
            },
            state: RunState::Running,
        };
        match tasks.iter_mut().find(|worker| worker.task == start.task) {
            Some(worker) => {
                worker.agent.clone_from(&start.agent.name);
                worker.runs.push(run);
            }
            None => tasks.push(WorkerTask {
                task: start.task.clone(),
                agent: start.agent.name.clone(),
                runs: vec![run],
            }),
        }
    }
    for stored in records {
        let Record::RunEnd(end) = &stored.record else {
            continue;
        };
        let ended = tasks
            .iter_mut()
            .flat_map(|worker| worker.runs.iter_mut())
            .find(|run| run.run == end.run);
        if let Some(run) = ended {
            run.state = RunState::Ended {
                at: end.at,
                end: end.end.clone(),
            };
        }
    }
    tasks
}

/// The `workers:` block of the summary: a line for each worker task.
fn workers_text(tasks: &[WorkerTask], zone: agent_core::local_time::Zone) -> String {
    let lines: Vec<String> = tasks
        .iter()
        .filter_map(|worker| {
            let last = worker.runs.last()?;
            let asked_by = last.asked_by.as_ref().map_or("unknown".to_owned(), ToString::to_string);
            Some(format!(
                "- {} (agent {}): asked by {asked_by}; last run {} started {}; {}",
                worker.task,
                worker.agent,
                last.run,
                agent_core::debug_summary::format_time(last.started, zone),
                run_detail_at(&last.state, zone),
            ))
        })
        .collect();
    format!("workers:\n{}\n", lines.join("\n"))
}

impl Collection {
    /// Adds the files of the worker tasks of `team` that `records` name, and lists them in the summary.
    pub(super) async fn workers(&mut self, home: &Home, records: &[SequencedRecord], team: &TeamId) {
        let tasks = worker_tasks(records, team);
        if tasks.is_empty() {
            return;
        }
        for worker in &tasks {
            self.worker_files(home, worker).await;
        }
        let text = workers_text(&tasks, self.zone);
        match self.summary.history_tail.as_mut() {
            Some(tail) => {
                tail.push('\n');
                tail.push_str(&text);
            }
            None => self.summary.history_tail = Some(text),
        }
    }

    /// Adds the context file of a worker task and the output of its newest runs; a missing task directory is a note.
    async fn worker_files(&mut self, home: &Home, worker: &WorkerTask) {
        let root = home.task_dir(&worker.task);
        let part = format!("worker task {}", worker.task);
        match tokio::fs::symlink_metadata(&root).await {
            Ok(metadata) if metadata.is_dir() => {}
            Ok(_) => {
                self.task_failed(part, "the task directory is not a directory");
                return;
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                self.task_failed(part, "the task directory is missing");
                return;
            }
            Err(error) => {
                self.task_failed(part, error.to_string());
                return;
            }
        }
        let target = PathBuf::from("tasks").join(worker.task.as_str());
        self.worker_file(&root.join(CONTEXT_FILE), target.join(CONTEXT_FILE), &worker.task)
            .await;
        for run in worker.runs.iter().rev().take(WORKER_RUNS_KEPT) {
            let source = home.run_dir(&worker.task, &run.run);
            let destination = target.join("runs").join(run.run.as_str());
            for name in [OUTPUT_FILE, ERROR_FILE] {
                self.worker_file(&source.join(name), destination.join(name), &worker.task)
                    .await;
            }
        }
    }

    /// Adds one file of a worker task with the bounds of a member's files; a file that does not exist is not a failure.
    async fn worker_file(&mut self, source: &Path, target: PathBuf, task: &TaskId) {
        match tokio::fs::symlink_metadata(source).await {
            Ok(metadata) if metadata.is_file() => self.task_file(source, target, task, &metadata).await,
            Ok(_) => self.task_failed(
                format!("task artifact {}", escaped_path(source)),
                "symlink or special file is not followed",
            ),
            // A run that failed before its CLI started wrote no output, and a worker that never reported its context
            // has no context file: both are normal.
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => self.task_failed(format!("task artifact {}", escaped_path(source)), error.to_string()),
        }
    }
}
