//! What the record of the checks keeps of a finding about a commit: what the human allowed and what they refused, for
//! the item until it is closed. A record an earlier daemon wrote has no refusals and still loads and goes on.

use agent_core::decide::checks::Subject;
use agent_core::ids::ItemId;
use agent_core::state::{ChecksRecord, CommitId};

const BEFORE: &str = include_str!("fixtures/checks-before-commit-subjects.json");

fn item() -> ItemId {
    "1.1".parse().unwrap()
}

fn other_item() -> ItemId {
    "1.2".parse().unwrap()
}

fn commit(digit: char, path: &str) -> Subject {
    Subject::Commit {
        commit: digit.to_string().repeat(40).parse::<CommitId>().unwrap(),
        path: path.parse().unwrap(),
    }
}

#[test]
fn a_record_written_before_commit_subjects_loads_with_its_question_and_its_allowances() {
    let record: ChecksRecord = serde_json::from_str(BEFORE).unwrap();

    assert_eq!(record.refused_of(&[item()]), []);
    assert_eq!(
        record.allowed_of(&[item()]),
        [
            Subject::Path("ci/gate.sh".parse().unwrap()),
            Subject::Path("tests/old.txt".parse().unwrap()),
            Subject::NoTests,
        ]
    );
    assert_eq!(record.passes_of(&[item()]), 1);
    assert_eq!(record.asked.as_ref().map(|asked| asked.relays), Some(1));
    assert!(record.sent.is_some());
    let written = serde_json::to_value(&record).unwrap();
    assert_eq!(written, serde_json::from_str::<serde_json::Value>(BEFORE).unwrap());
}

#[test]
fn an_answer_keeps_what_was_allowed_and_what_was_refused_for_every_item_of_the_round() {
    let mut record = ChecksRecord::default();
    let items = [item(), other_item()];

    record.answered(&items, &[commit('a', "tests/a.rs")], &[commit('b', "tests/a.rs")]);

    for each in items {
        assert_eq!(record.allowed_of(&[each]), [commit('a', "tests/a.rs")]);
        assert_eq!(record.refused_of(&[each]), [commit('b', "tests/a.rs")]);
    }
}

#[test]
fn a_later_answer_adds_to_what_was_kept_and_does_not_replace_it() {
    let mut record = ChecksRecord::default();
    record.answered(&[item()], &[commit('a', "tests/a.rs")], &[commit('b', "tests/a.rs")]);

    record.answered(&[item()], &[commit('c', "tests/a.rs")], &[commit('d', "tests/b.rs")]);

    assert_eq!(
        record.allowed_of(&[item()]),
        [commit('a', "tests/a.rs"), commit('c', "tests/a.rs")]
    );
    assert_eq!(
        record.refused_of(&[item()]),
        [commit('b', "tests/a.rs"), commit('d', "tests/b.rs")]
    );
}

#[test]
fn what_the_human_allows_after_refusing_it_is_no_longer_refused() {
    let mut record = ChecksRecord::default();
    record.answered(&[item()], &[], &[commit('a', "tests/a.rs")]);
    assert_eq!(record.refused_of(&[item()]), [commit('a', "tests/a.rs")]);

    record.answered(&[item()], &[commit('a', "tests/a.rs")], &[]);

    assert_eq!(record.allowed_of(&[item()]), [commit('a', "tests/a.rs")]);
    assert_eq!(record.refused_of(&[item()]), []);
}

#[test]
fn refusals_and_allowed_commits_survive_a_write_and_a_read_of_the_record() {
    let mut record: ChecksRecord = serde_json::from_str(BEFORE).unwrap();
    record.answered(&[item()], &[commit('a', "tests/a.rs")], &[commit('b', "tests/a.rs")]);

    let text = serde_json::to_string(&record).unwrap();
    let read: ChecksRecord = serde_json::from_str(&text).unwrap();

    assert_eq!(read, record);
    assert_eq!(read.refused_of(&[item()]), [commit('b', "tests/a.rs")]);
    assert!(read.allowed_of(&[item()]).contains(&commit('a', "tests/a.rs")));
}

#[test]
fn taking_back_an_allowance_does_not_take_back_a_refusal() {
    let mut record = ChecksRecord::default();
    record.answered(&[item()], &[commit('a', "tests/a.rs")], &[commit('b', "tests/a.rs")]);

    record.revoke(&[item()], &[commit('a', "tests/a.rs"), commit('b', "tests/a.rs")]);

    assert_eq!(record.allowed_of(&[item()]), []);
    assert_eq!(record.refused_of(&[item()]), [commit('b', "tests/a.rs")]);
}
