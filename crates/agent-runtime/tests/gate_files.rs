//! Where the gate of a task keeps its files, and the atomic save of its record.

#[cfg(test)]
mod gate_files {
    use std::num::NonZeroU32;
    use std::path::PathBuf;
    use std::str::FromStr;

    use agent_core::config::AbsolutePath;
    use agent_core::ids::TaskId;
    use agent_core::state::{GateOutcome, GateRecord, GateResult, GateTree};
    use agent_runtime::store::{Home, StateStore};

    const TASK: &str = "task-20261111-153601-k3x9";

    fn task() -> TaskId {
        TASK.parse().unwrap()
    }

    fn record(tree: GateTree, log: &str) -> GateRecord {
        GateRecord {
            tree,
            result: GateResult {
                head: "26b65de7e4076596768ca96b48a7f94799c93874".parse().unwrap(),
                finished_at: "2026-11-13T09:30:00Z".parse().unwrap(),
                outcome: GateOutcome::Passed,
                log: AbsolutePath::from_str(log).unwrap(),
            },
        }
    }

    #[test]
    fn a_gate_log_is_numbered_by_its_run_and_the_record_has_its_file_in_the_directory_of_the_task() {
        let home = Home::new(&AbsolutePath::from_str("/srv/agents-home").unwrap());

        assert_eq!(
            home.gate_log(&task(), NonZeroU32::new(3).unwrap()),
            PathBuf::from("/srv/agents-home/tasks/task-20261111-153601-k3x9/gate-3.log")
        );
        assert_eq!(
            home.gate_record_file(&task()),
            PathBuf::from("/srv/agents-home/tasks/task-20261111-153601-k3x9/gate.json")
        );
    }

    #[test]
    fn the_worktrees_of_every_repository_are_in_one_directory_of_the_home() {
        let home = Home::new(&AbsolutePath::from_str("/srv/agents-home").unwrap());
        let repo = agent_core::config::RepoName::from_str("team-agents").unwrap();

        assert_eq!(home.worktrees_dir(), PathBuf::from("/srv/agents-home/worktrees"));
        assert!(home.worktree_dir(&repo, &task()).starts_with(home.worktrees_dir()));
    }

    #[tokio::test]
    async fn a_saved_record_is_in_its_file_and_a_later_one_replaces_it_leaving_nothing_behind() {
        let dir = tempfile::tempdir().unwrap();
        let home = Home::new(&AbsolutePath::from_str(dir.path().to_str().unwrap()).unwrap());
        let store = StateStore::open(home.clone()).await.unwrap();
        let first = record(GateTree::Clean, "/x/gate-1.log");
        let second = record(GateTree::NotClean, "/x/gate-2.log");

        store.save_gate_record(&task(), &first).await.unwrap();
        let after_first: GateRecord =
            serde_json::from_slice(&std::fs::read(home.gate_record_file(&task())).unwrap()).unwrap();
        store.save_gate_record(&task(), &second).await.unwrap();
        let after_second: GateRecord =
            serde_json::from_slice(&std::fs::read(home.gate_record_file(&task())).unwrap()).unwrap();

        assert_eq!(after_first, first);
        assert_eq!(after_second, second);
        let names: Vec<String> = std::fs::read_dir(home.task_dir(&task()))
            .unwrap()
            .map(|entry| entry.unwrap().file_name().into_string().unwrap())
            .collect();
        assert_eq!(
            names,
            ["gate.json"],
            "a temporary file was left in the task's directory"
        );
    }
}
