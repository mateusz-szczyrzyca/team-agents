//! When a task was last active: the newest change of the output of its runs.

#[cfg(test)]
mod run_activity {
    use std::path::Path;
    use std::str::FromStr;
    use std::time::{Duration, SystemTime};

    use agent_core::config::AbsolutePath;
    use agent_core::ids::{RunId, TaskId};
    use agent_runtime::run::activity::last_activity;
    use agent_runtime::store::Home;

    fn home(dir: &Path) -> Home {
        Home::new(&AbsolutePath::from_str(dir.to_str().unwrap()).unwrap())
    }

    fn task() -> TaskId {
        TaskId::from_str("task-20261116-214907-zvim").unwrap()
    }

    /// Writes the output of run `run` of the task and makes it `age` seconds old.
    fn output(home: &Home, run: &str, age: u64) -> SystemTime {
        let dir = home.run_dir(&task(), &RunId::from_str(run).unwrap());
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("out.jsonl");
        std::fs::write(&file, "{}\n").unwrap();
        let changed = SystemTime::now().checked_sub(Duration::from_secs(age)).unwrap();
        std::fs::File::options()
            .write(true)
            .open(&file)
            .unwrap()
            .set_modified(changed)
            .unwrap();
        std::fs::metadata(&file).unwrap().modified().unwrap()
    }

    // Rule: the last activity of a task is the newest change of the output of any of its runs.
    #[test]
    fn the_last_activity_is_the_newest_change_of_the_output_of_the_runs() {
        let dir = tempfile::tempdir().unwrap();
        let home = home(dir.path());
        output(&home, "run-20261116-214908-aaaa", 900);
        let newest = output(&home, "run-20261116-220000-bbbb", 30);
        output(&home, "run-20261116-215000-cccc", 300);

        assert_eq!(last_activity(&home, &task()).unwrap(), Some(newest));
    }

    // Rule: a task that has not run, and a run that has written nothing, have no activity: that is not an error.
    #[test]
    fn a_task_without_output_has_no_activity() {
        let dir = tempfile::tempdir().unwrap();
        let home = home(dir.path());

        assert_eq!(last_activity(&home, &task()).unwrap(), None);

        std::fs::create_dir_all(home.run_dir(&task(), &RunId::from_str("run-20261116-214908-aaaa").unwrap())).unwrap();
        assert_eq!(last_activity(&home, &task()).unwrap(), None);
    }

    // Rule: runs that cannot be listed are an error that names what could not be read.
    #[test]
    fn runs_that_cannot_be_listed_are_an_error_that_names_the_directory() {
        let dir = tempfile::tempdir().unwrap();
        let home = home(dir.path());
        let runs = home.runs_dir(&task());
        std::fs::create_dir_all(runs.parent().unwrap()).unwrap();
        // A file where the directory of the runs would be.
        std::fs::write(&runs, "x").unwrap();

        let error = last_activity(&home, &task()).unwrap_err();

        assert_eq!(error.path, runs);
    }
}
