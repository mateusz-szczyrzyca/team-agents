//! Task activity comes from validated steps in history, across every run of the task.
use super::support::{complete, database};
use agent_runtime::history::{Reader, Store};

#[test]
fn task_activity_includes_each_run_in_order_and_excludes_other_tasks() {
    for seed in [87001, 87002, 87003] {
        let (_dir, path, mut db) = database();
        let world = complete(seed);
        db.write(&world.records()).unwrap();
        let reader = Reader::open(&path).unwrap();
        for run in &world.runs {
            let expected: Vec<_> = world
                .runs
                .iter()
                .filter(|other| other.start.task == run.start.task)
                .flat_map(|other| other.steps.clone())
                .collect();
            let actual: Vec<_> = reader
                .steps_of_task(&run.start.task)
                .unwrap()
                .into_iter()
                .map(|step| step.value)
                .collect();
            assert_eq!(actual, expected, "seed={seed}");
        }
        assert_eq!(
            reader
                .steps_of_task(&"task-20261115-100000-zzzz".parse().unwrap())
                .unwrap(),
            Vec::new()
        );
    }
}
