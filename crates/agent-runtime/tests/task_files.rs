//! The files the autopilot reads and keeps in a task's directory: the last gate record, which the gate service wrote,
//! and the reports of the reviewers, which the daemon writes for the fix pass.

#[cfg(test)]
mod task_files {
    use std::num::NonZeroU32;
    use std::path::PathBuf;
    use std::str::FromStr;

    use agent_core::config::AbsolutePath;
    use agent_core::ids::TaskId;
    use agent_core::state::{GateOutcome, GateRecord, GateResult, GateTree, PrNumber};
    use agent_runtime::store::{Home, StateStore, StoreError};

    fn task() -> TaskId {
        "task-20261111-153601-k3x9".parse().unwrap()
    }

    fn pr(number: u32) -> PrNumber {
        PrNumber::new(NonZeroU32::new(number).unwrap())
    }

    fn record() -> GateRecord {
        GateRecord {
            tree: GateTree::Clean,
            result: GateResult {
                head: "26b65de7e4076596768ca96b48a7f94799c93874".parse().unwrap(),
                finished_at: "2026-11-13T09:30:00Z".parse().unwrap(),
                outcome: GateOutcome::Passed,
                log: AbsolutePath::from_str("/x/gate-1.log").unwrap(),
            },
        }
    }

    async fn store() -> (tempfile::TempDir, Home, StateStore) {
        let dir = tempfile::tempdir().unwrap();
        let home = Home::new(&AbsolutePath::from_str(dir.path().to_str().unwrap()).unwrap());
        let store = StateStore::open(home.clone()).await.unwrap();
        (dir, home, store)
    }

    #[test]
    fn a_report_has_its_file_in_the_directory_of_the_reviewers_task_named_by_pull_request_and_pass() {
        let home = Home::new(&AbsolutePath::from_str("/srv/agents-home").unwrap());

        assert_eq!(
            home.review_report_file(&task(), pr(7), 2),
            PathBuf::from("/srv/agents-home/tasks/task-20261111-153601-k3x9/review-pr7-pass2.md")
        );
    }

    #[tokio::test]
    async fn a_task_that_has_not_been_gated_has_no_gate_record() {
        let (_dir, _home, store) = store().await;

        assert_eq!(store.load_gate_record(&task()).await.unwrap(), None);
    }

    #[tokio::test]
    async fn the_gate_record_that_was_saved_is_the_one_that_is_loaded_and_a_later_one_replaces_it() {
        let (_dir, _home, store) = store().await;
        let mut second = record();
        second.tree = GateTree::NotClean;

        store.save_gate_record(&task(), &record()).await.unwrap();
        let first_loaded = store.load_gate_record(&task()).await.unwrap();
        store.save_gate_record(&task(), &second).await.unwrap();
        let second_loaded = store.load_gate_record(&task()).await.unwrap();

        assert_eq!(first_loaded, Some(record()));
        assert_eq!(second_loaded, Some(second));
    }

    #[tokio::test]
    async fn a_gate_record_that_is_not_json_is_an_error_that_names_the_file() {
        let (_dir, home, store) = store().await;
        std::fs::create_dir_all(home.task_dir(&task())).unwrap();
        std::fs::write(home.gate_record_file(&task()), "not json").unwrap();

        let error = store.load_gate_record(&task()).await.unwrap_err();

        assert!(
            matches!(&error, StoreError::ParseGateRecord { path, .. } if *path == home.gate_record_file(&task())),
            "{error:?}"
        );
    }

    #[tokio::test]
    async fn a_gate_record_that_cannot_be_read_is_an_error_that_names_the_file() {
        let (_dir, home, store) = store().await;
        std::fs::create_dir_all(home.gate_record_file(&task())).unwrap();

        let error = store.load_gate_record(&task()).await.unwrap_err();

        assert!(
            matches!(&error, StoreError::ReadTaskFile { path, .. } if *path == home.gate_record_file(&task())),
            "{error:?}"
        );
    }

    #[tokio::test]
    async fn a_report_that_was_saved_is_loaded_and_a_later_one_for_the_same_pass_replaces_it() {
        let (_dir, home, store) = store().await;

        store
            .save_review_report(&task(), pr(7), 0, "VERDICT: REQUEST_CHANGES\nżółć\n")
            .await
            .unwrap();
        let first = store.load_review_report(&task(), pr(7), 0).await.unwrap();
        store
            .save_review_report(&task(), pr(7), 0, "VERDICT: APPROVE\n")
            .await
            .unwrap();
        let second = store.load_review_report(&task(), pr(7), 0).await.unwrap();

        assert_eq!(first.as_deref(), Some("VERDICT: REQUEST_CHANGES\nżółć\n"));
        assert_eq!(second.as_deref(), Some("VERDICT: APPROVE\n"));
        let names: Vec<String> = std::fs::read_dir(home.task_dir(&task()))
            .unwrap()
            .map(|entry| entry.unwrap().file_name().into_string().unwrap())
            .collect();
        assert_eq!(names, ["review-pr7-pass0.md"], "a temporary file was left behind");
    }

    #[tokio::test]
    async fn the_reports_of_another_pass_or_pull_request_are_not_this_one() {
        let (_dir, _home, store) = store().await;
        store.save_review_report(&task(), pr(7), 0, "first").await.unwrap();

        assert_eq!(store.load_review_report(&task(), pr(7), 1).await.unwrap(), None);
        assert_eq!(store.load_review_report(&task(), pr(8), 0).await.unwrap(), None);
    }

    #[tokio::test]
    async fn a_report_that_is_not_text_is_an_error_that_names_the_file() {
        let (_dir, home, store) = store().await;
        std::fs::create_dir_all(home.task_dir(&task())).unwrap();
        std::fs::write(home.review_report_file(&task(), pr(7), 0), [0xff, 0xfe]).unwrap();

        let error = store.load_review_report(&task(), pr(7), 0).await.unwrap_err();

        assert!(
            matches!(&error, StoreError::ReadTaskFile { path, .. } if *path == home.review_report_file(&task(), pr(7), 0)),
            "{error:?}"
        );
    }
}
