//! What the record of the checks keeps of what the human allowed: for which issues, at which head, and what an answer
//! does to the count of the team's passes. A record an earlier daemon wrote has none of the newer keys and still loads.

#[cfg(test)]
mod grants {
    use agent_core::decide::checks::Subject;
    use agent_core::ids::ItemId;
    use agent_core::state::{Basis, ChecksRecord, CommitId, Covers};

    fn item() -> ItemId {
        "1.1".parse().unwrap()
    }

    fn path(text: &str) -> Subject {
        Subject::Path(text.parse().unwrap())
    }

    fn head(digit: char) -> CommitId {
        digit.to_string().repeat(40).parse().unwrap()
    }

    /// A record with `passes` team returns used.
    fn used(passes: u32) -> ChecksRecord {
        let mut record = ChecksRecord::default();
        record.set_passes(&[item()], passes);
        record
    }

    const LEGACY: &str = r#"{
  "items": { "1.1": { "passes": 1, "allowed": [{ "path": "tests/old.txt" }] } },
  "asked": {
    "number": 2,
    "at": "2026-11-20T10:00:00Z",
    "items": ["1.1"],
    "subjects": [{ "path": "ci/gate.sh" }],
    "relays": 0
  }
}"#;

    #[test]
    fn a_record_written_before_the_grants_keys_loads_and_its_allowance_holds_for_any_head() {
        let record: ChecksRecord = serde_json::from_str(LEGACY).unwrap();

        assert_eq!(record.allowed_of(&[item()]), [path("tests/old.txt")]);
        assert_eq!(record.human_only_of(&[item()]), []);
        assert!(record.allowed_at_of(&[item()]).is_empty());
        assert_eq!(record.basis, None);
        assert_eq!(record.passes_of(&[item()]), 1);
    }

    #[test]
    fn a_record_without_new_keys_is_written_without_them() {
        let record: ChecksRecord = serde_json::from_str(LEGACY).unwrap();

        let written = serde_json::to_value(&record).unwrap();

        assert_eq!(written["items"]["1.1"].get("human_only"), None);
        assert_eq!(written["items"]["1.1"].get("allowed_at"), None);
        assert_eq!(written.get("basis"), None);
    }

    #[test]
    fn the_answer_to_a_question_about_the_human_part_keeps_the_passes_and_covers_the_human_issues_only() {
        let mut record = used(1);
        record.basis = Some(Basis {
            head: head('a'),
            covers: Covers::HumanPart,
        });

        record.answer(&[item()], &[path("ci/gate.sh")]);

        assert_eq!(record.passes_of(&[item()]), 1);
        assert_eq!(record.allowed_of(&[item()]), [path("ci/gate.sh")]);
        assert_eq!(record.human_only_of(&[item()]), [path("ci/gate.sh")]);
        assert_eq!(record.allowed_at_of(&[item()]).len(), 1);
        assert_eq!((record.asked, record.basis), (None, None));
    }

    #[test]
    fn the_answer_to_a_question_about_everything_starts_the_passes_again_and_covers_all_issues() {
        let mut record = used(2);
        record.basis = Some(Basis {
            head: head('a'),
            covers: Covers::Everything,
        });

        record.answer(&[item()], &[path("src/a.txt")]);

        assert_eq!(record.passes_of(&[item()]), 0);
        assert_eq!(record.human_only_of(&[item()]), []);
        assert_eq!(record.allowed_of(&[item()]), [path("src/a.txt")]);
    }

    #[test]
    fn the_answer_to_a_question_an_earlier_daemon_wrote_starts_the_passes_again_and_binds_no_head() {
        let mut record = used(2);

        record.answer(&[item()], &[path("tests/old.txt")]);

        assert_eq!(record.passes_of(&[item()]), 0);
        assert_eq!(record.allowed_of(&[item()]), [path("tests/old.txt")]);
        assert!(record.allowed_at_of(&[item()]).is_empty());
    }

    #[test]
    fn an_allowance_for_everything_is_not_narrowed_by_a_later_answer_about_the_human_part() {
        let mut record = ChecksRecord::default();
        let first = Basis {
            head: head('a'),
            covers: Covers::Everything,
        };
        let later = Basis {
            head: head('b'),
            covers: Covers::HumanPart,
        };
        record.allow_on(&[item()], &[path("ci/gate.sh")], &first);

        record.allow_on(&[item()], &[path("ci/gate.sh")], &later);

        assert_eq!(record.human_only_of(&[item()]), []);
    }

    #[test]
    fn a_revoked_file_is_no_longer_allowed_and_has_no_head() {
        let mut record = ChecksRecord::default();
        let basis = Basis {
            head: head('a'),
            covers: Covers::HumanPart,
        };
        record.allow_on(&[item()], &[path("ci/gate.sh"), path("tests/x.txt")], &basis);

        record.revoke(&[item()], &[path("ci/gate.sh")]);

        assert_eq!(record.allowed_of(&[item()]), [path("tests/x.txt")]);
        assert_eq!(record.human_only_of(&[item()]), [path("tests/x.txt")]);
        assert_eq!(
            record
                .allowed_at_of(&[item()])
                .keys()
                .map(ToString::to_string)
                .collect::<Vec<_>>(),
            ["tests/x.txt"]
        );
    }

    #[test]
    fn the_grants_survive_a_round_trip_through_json() {
        let mut record = used(1);
        record.basis = Some(Basis {
            head: head('c'),
            covers: Covers::HumanPart,
        });
        record.answer(&[item()], &[path("ci/gate.sh")]);

        let again: ChecksRecord = serde_json::from_str(&serde_json::to_string(&record).unwrap()).unwrap();

        assert_eq!(again, record);
    }
}
