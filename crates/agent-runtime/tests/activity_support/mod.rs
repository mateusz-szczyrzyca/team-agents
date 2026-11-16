//! A real history writer for tests migrating away from the activity file.
use agent_core::history::{AgentIdentity, MaxTextBytes, Record, RunStart, RunWhy, TeamStart};
use agent_core::ids::{RunId, TaskId};
use agent_core::kinds::{CliKind, Role, Tier};
use agent_core::state::MergeMode;
use agent_runtime::history::{History, HistoryDb, Setup, Started, Store, TracingReporter};
use agent_runtime::store::Home;
use std::sync::Arc;

pub(crate) fn start(home: &Home, task: TaskId, run: RunId) -> Started {
    let at = "2026-11-12T10:00:00Z".parse().unwrap();
    let team: agent_core::ids::TeamId = "team-20261112-100000-abcd".parse().unwrap();
    let mut db = HistoryDb::open(&home.history_file(), MaxTextBytes::default()).unwrap();
    db.write(&[
        Record::Team(Box::new(TeamStart {
            team: team.clone(),
            at,
            repo: "example".to_owned(),
            base_branch: "main".parse().unwrap(),
            merge: MergeMode::NoMerge,
            task_list: "Record activity.".to_owned(),
            plan: Vec::new(),
        })),
        Record::Run(Box::new(RunStart {
            run,
            team,
            task,
            at,
            agent: AgentIdentity {
                name: "claude-1".to_owned(),
                cli: CliKind::Claude,
                tier: Tier::Normal,
                model: None,
            },
            role: Role::Solo,
            rounds: Vec::new(),
            why: RunWhy::FirstRun,
            prompt: "Record activity.".to_owned(),
            worktree: home.absolute_root().clone(),
            branch: "main".parse().unwrap(),
            session: None,
        })),
    ])
    .unwrap();
    History::start(
        db,
        Setup {
            reporter: Arc::new(TracingReporter),
            capacity: 100.try_into().unwrap(),
        },
    )
    .unwrap()
}
