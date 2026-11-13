//! The record of the checks before a review, as the daemon keeps it in the lead's task directory.

#[cfg(test)]
mod checks_record {
    use std::num::NonZeroU32;

    use agent_core::decide::checks::Subject;
    use agent_core::ids::ItemId;
    use agent_core::repo_path::RepoPath;
    use agent_core::state::{Asked, ChecksRecord, ItemChecks, Sent, Timestamp};

    fn item(text: &str) -> ItemId {
        text.parse().unwrap()
    }

    fn file(text: &str) -> Subject {
        Subject::Path(text.parse::<RepoPath>().unwrap())
    }

    fn at(text: &str) -> Timestamp {
        text.parse().unwrap()
    }

    // Rule: the passes of a round that moves together are the most any of its items has, and an item the checks have not
    // seen has none.
    #[test]
    fn the_passes_of_a_round_are_the_most_of_any_of_its_items() {
        let mut record = ChecksRecord::default();
        record.set_passes(&[item("1.1")], 2);
        record.set_passes(&[item("1.2")], 1);

        assert_eq!(record.passes_of(&[item("1.1"), item("1.2")]), 2);
        assert_eq!(record.passes_of(&[item("1.2")]), 1);
        assert_eq!(record.passes_of(&[item("2.1")]), 0);
        assert_eq!(record.passes_of(&[]), 0);
    }

    #[test]
    fn passes_are_set_for_every_item_of_the_round_and_replace_what_was_there() {
        let mut record = ChecksRecord::default();
        record.set_passes(&[item("1.1"), item("1.2")], 1);
        record.set_passes(&[item("1.1")], 2);

        assert_eq!(record.passes_of(&[item("1.1")]), 2);
        assert_eq!(record.passes_of(&[item("1.2")]), 1);
    }

    // Rule: what the human allowed is recorded for every item of the round and kept in the order of the subjects, once.
    #[test]
    fn what_the_human_allowed_is_recorded_for_every_item_without_repeats() {
        let mut record = ChecksRecord::default();
        record.allow(&[item("1.1"), item("1.2")], &[file("tests/b.rs"), file("tests/a.rs")]);
        record.allow(&[item("1.1")], &[file("tests/a.rs"), Subject::NoTests]);

        assert_eq!(
            record.allowed_of(&[item("1.1")]),
            [file("tests/a.rs"), file("tests/b.rs"), Subject::NoTests]
        );
        assert_eq!(
            record.allowed_of(&[item("1.2")]),
            [file("tests/a.rs"), file("tests/b.rs")]
        );
        assert_eq!(
            record.allowed_of(&[item("1.1"), item("1.2")]),
            [file("tests/a.rs"), file("tests/b.rs"), Subject::NoTests]
        );
        assert_eq!(record.allowed_of(&[item("3.1")]), []);
    }

    // Rule: an answer of the human starts the count again and keeps what was allowed.
    #[test]
    fn restarting_the_passes_keeps_what_was_allowed() {
        let mut record = ChecksRecord::default();
        record.set_passes(&[item("1.1"), item("1.2")], 2);
        record.allow(&[item("1.1")], &[file("tests/a.rs")]);

        record.restart_passes(&[item("1.1")]);

        assert_eq!(record.passes_of(&[item("1.1")]), 0);
        assert_eq!(record.passes_of(&[item("1.2")]), 2);
        assert_eq!(record.allowed_of(&[item("1.1")]), [file("tests/a.rs")]);
    }

    // Rule: the record is a JSON file the daemon writes and reads back; an empty one is `{}`.
    #[test]
    fn an_empty_record_is_an_empty_object_and_reads_back() {
        let json = serde_json::to_string(&ChecksRecord::default()).unwrap();

        assert_eq!(json, "{}");
        assert_eq!(
            serde_json::from_str::<ChecksRecord>(&json).unwrap(),
            ChecksRecord::default()
        );
    }

    #[test]
    fn a_record_with_everything_in_it_reads_back_as_it_was_written() {
        let mut record = ChecksRecord::default();
        record.set_passes(&[item("1.1")], 1);
        record.allow(&[item("1.1")], &[file("tests/a.rs"), Subject::NoTests]);
        record.asked = Some(Asked {
            number: NonZeroU32::new(2).unwrap(),
            at: at("2026-11-13T10:00:00Z"),
            items: vec![item("1.1")],
            subjects: vec![file("tests/a.rs")],
            relays: 1,
        });
        record.sent = Some(Sent {
            at: at("2026-11-13T09:00:00Z"),
            pass: 1,
            of: 2,
            text: "Fix the test.".to_owned(),
        });

        let json = serde_json::to_string(&record).unwrap();

        assert_eq!(serde_json::from_str::<ChecksRecord>(&json).unwrap(), record);
    }

    #[test]
    fn the_record_is_written_with_its_fields_in_this_shape() {
        let mut record = ChecksRecord::default();
        record.allow(&[item("1.1")], &[file("tests/a.rs"), Subject::NoTests]);
        record.sent = Some(Sent {
            at: at("2026-11-13T09:00:00Z"),
            pass: 1,
            of: 2,
            text: "x".to_owned(),
        });

        let json = serde_json::to_value(&record).unwrap();

        assert_eq!(
            json,
            serde_json::json!({
                "items": {"1.1": {"passes": 0, "allowed": [{"path": "tests/a.rs"}, "no_tests"]}},
                "sent": {"at": "2026-11-13T09:00:00Z", "pass": 1, "of": 2, "text": "x"}
            })
        );
    }

    #[test]
    fn a_record_with_a_field_it_does_not_know_is_refused() {
        for text in [
            r#"{"extra": 1}"#,
            r#"{"items": {"1.1": {"passes": 0, "allowed": [], "extra": 1}}}"#,
            r#"{"sent": {"at": "2026-11-13T09:00:00Z", "pass": 1, "of": 2, "text": "x", "extra": 1}}"#,
        ] {
            assert!(
                serde_json::from_str::<ChecksRecord>(text).unwrap_err().is_data(),
                "{text}"
            );
        }
    }

    #[test]
    fn a_record_with_an_item_that_is_no_item_id_is_refused() {
        let text = r#"{"items": {"one": {"passes": 0, "allowed": []}}}"#;

        assert!(serde_json::from_str::<ChecksRecord>(text).unwrap_err().is_data());
    }

    #[test]
    fn what_the_checks_hold_of_an_item_is_plain_data() {
        let held = ItemChecks::default();

        assert_eq!(held.passes, 0);
        assert!(held.allowed.is_empty());
    }
}
