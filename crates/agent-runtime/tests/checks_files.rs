//! Where the lead of a team keeps the record of its checks before a review, and the atomic save of it.

#[cfg(test)]
mod checks_files {
    use std::num::NonZeroU32;
    use std::path::PathBuf;
    use std::str::FromStr;

    use agent_core::config::AbsolutePath;
    use agent_core::decide::checks::Subject;
    use agent_core::ids::TaskId;
    use agent_core::state::{Asked, ChecksRecord};
    use agent_runtime::store::{Home, StateStore, StoreError, load_checks_record};

    fn task() -> TaskId {
        "task-20261111-153601-k3x9".parse().unwrap()
    }

    fn record(passes: u32) -> ChecksRecord {
        let mut record = ChecksRecord::default();
        record.set_passes(&["1.1".parse().unwrap()], passes);
        record.allow(&["1.1".parse().unwrap()], &[Subject::NoTests]);
        record.asked = Some(Asked {
            number: NonZeroU32::new(1).unwrap(),
            at: "2026-11-13T09:30:00Z".parse().unwrap(),
            items: vec!["1.1".parse().unwrap()],
            subjects: vec![Subject::Path("tests/a.rs".parse().unwrap())],
            relays: 0,
        });
        record
    }

    async fn store() -> (tempfile::TempDir, Home, StateStore) {
        let dir = tempfile::tempdir().unwrap();
        let home = Home::new(&AbsolutePath::from_str(dir.path().to_str().unwrap()).unwrap());
        let store = StateStore::open(home.clone()).await.unwrap();
        (dir, home, store)
    }

    #[test]
    fn the_record_has_its_file_in_the_directory_of_the_leads_task() {
        let home = Home::new(&AbsolutePath::from_str("/srv/agents-home").unwrap());

        assert_eq!(
            home.checks_file(&task()),
            PathBuf::from("/srv/agents-home/tasks/task-20261111-153601-k3x9/checks.json")
        );
    }

    #[tokio::test]
    async fn a_task_the_checks_have_not_seen_has_no_record() {
        let (_dir, home, store) = store().await;

        assert_eq!(store.load_checks_record(&task()).await.unwrap(), None);
        assert_eq!(load_checks_record(&home, &task()).await.unwrap(), None);
    }

    #[tokio::test]
    async fn the_record_that_was_saved_is_the_one_that_is_loaded_and_a_later_one_replaces_it() {
        let (_dir, home, store) = store().await;

        store.save_checks_record(&task(), &record(1)).await.unwrap();
        let first = store.load_checks_record(&task()).await.unwrap();
        store.save_checks_record(&task(), &record(2)).await.unwrap();
        let second = load_checks_record(&home, &task()).await.unwrap();

        assert_eq!(first, Some(record(1)));
        assert_eq!(second, Some(record(2)));
        let names: Vec<String> = std::fs::read_dir(home.task_dir(&task()))
            .unwrap()
            .map(|entry| entry.unwrap().file_name().into_string().unwrap())
            .collect();
        assert_eq!(names, ["checks.json"], "a temporary file was left behind");
    }

    #[tokio::test]
    async fn a_record_that_is_not_json_is_an_error_that_names_the_file() {
        let (_dir, home, store) = store().await;
        std::fs::create_dir_all(home.task_dir(&task())).unwrap();
        std::fs::write(home.checks_file(&task()), "not json").unwrap();

        let error = store.load_checks_record(&task()).await.unwrap_err();

        assert!(
            matches!(&error, StoreError::ParseChecksRecord { path, .. } if *path == home.checks_file(&task())),
            "{error:?}"
        );
    }

    #[tokio::test]
    async fn a_record_that_cannot_be_read_is_an_error_that_names_the_file() {
        let (_dir, home, store) = store().await;
        std::fs::create_dir_all(home.checks_file(&task())).unwrap();

        let error = store.load_checks_record(&task()).await.unwrap_err();

        assert!(
            matches!(&error, StoreError::ReadTaskFile { path, .. } if *path == home.checks_file(&task())),
            "{error:?}"
        );
    }

    #[tokio::test]
    async fn the_record_does_not_touch_the_gate_record_of_the_task() {
        let (_dir, home, store) = store().await;
        std::fs::create_dir_all(home.task_dir(&task())).unwrap();
        std::fs::write(home.gate_record_file(&task()), "the gate's own").unwrap();

        store.save_checks_record(&task(), &record(1)).await.unwrap();

        assert_eq!(
            std::fs::read_to_string(home.gate_record_file(&task())).unwrap(),
            "the gate's own"
        );
    }
}
