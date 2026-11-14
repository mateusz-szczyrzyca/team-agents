//! Fields belonging to another variant must not disappear silently during typed decoding.

use std::num::NonZeroU32;
use std::str::FromStr;

use agent_core::history::{HowChosen, Record, RunWhy};
use agent_core::ids::ItemId;
use agent_core::kinds::Role;
use agent_core::state::PrNumber;
use agent_runtime::history::{ReadError, Reader, Store};
use rusqlite::{Connection, ErrorCode};

use super::support::{complete, database};

/// Proves the database enforces the constraint, then bypasses it to exercise the reader independently.
fn corrupt(raw: &Connection, sql: &str, value: &str) {
    let error = raw.execute(sql, [value]).unwrap_err();
    assert_eq!(
        error.sqlite_error_code(),
        Some(ErrorCode::ConstraintViolation),
        "{sql}: {error}"
    );
    raw.execute_batch("PRAGMA ignore_check_constraints = ON").unwrap();
    raw.execute(sql, [value]).unwrap();
}

/// Asserts the reader identifies the rejected column rather than returning a partial interpretation.
fn rejected(error: &ReadError, table: &str, column: &str) {
    assert!(
        matches!(error, ReadError::Value { table: found_table, column: found_column, .. }
            if *found_table == table && *found_column == column),
        "{table}.{column}: {error:?}"
    );
}

#[test]
fn every_run_reason_rejects_fields_of_other_variants_in_sql_and_when_read() {
    let reasons = [
        RunWhy::FirstRun,
        RunWhy::Item {
            item: ItemId::from_str("1.1").unwrap(),
        },
        RunWhy::WakeUp,
        RunWhy::Request { asked_by: None },
        RunWhy::FixPass {
            number: NonZeroU32::new(1).unwrap(),
        },
        RunWhy::Review {
            pr: PrNumber::new(NonZeroU32::new(1).unwrap()),
            pass: 0,
        },
        RunWhy::AskHuman,
        RunWhy::VerdictMissing,
        RunWhy::Checks,
        RunWhy::Answers,
        RunWhy::ResumedByHuman,
        RunWhy::Continuation,
    ];
    for reason in reasons {
        let (_dir, path, mut db) = database();
        let mut start = complete(1).runs.remove(0).start;
        start.why = reason;
        db.write(&[Record::Run(Box::new(start.clone()))]).unwrap();
        let reader = Reader::open(&path).unwrap();
        assert_eq!(reader.run(&start.run).unwrap().unwrap().start.value, start);
        let raw = Connection::open(&path).unwrap();
        let fields = [
            ("why_item", "1.1", matches!(start.why, RunWhy::Item { .. })),
            (
                "why_asked_by",
                start.run.as_str(),
                matches!(start.why, RunWhy::Request { .. }),
            ),
            (
                "why_pass",
                "1",
                matches!(start.why, RunWhy::FixPass { .. } | RunWhy::Review { .. }),
            ),
            ("why_pr", "1", matches!(start.why, RunWhy::Review { .. })),
        ];
        for (column, value, allowed) in fields {
            if allowed {
                continue;
            }
            corrupt(&raw, &format!("UPDATE runs SET {column} = ?1"), value);
            rejected(&reader.run(&start.run).unwrap_err(), "runs", column);
            raw.execute_batch(&format!(
                "UPDATE runs SET {column} = NULL; PRAGMA ignore_check_constraints = OFF"
            ))
            .unwrap();
        }
    }
}

#[test]
fn each_nonrandom_selection_rejects_draw_candidates_in_sql_and_when_read() {
    for how in [
        HowChosen::Named,
        HowChosen::NamedByFlag,
        HowChosen::NamedByLine,
        HowChosen::ConfiguredOrder,
    ] {
        let (_dir, path, mut db) = database();
        let mut team = complete(1).team;
        team.plan.truncate(1);
        team.plan[0].how = how;
        db.write(&[Record::Team(Box::new(team.clone()))]).unwrap();
        let reader = Reader::open(&path).unwrap();
        assert_eq!(reader.team(&team.team).unwrap().unwrap().value, team);
        corrupt(
            &Connection::open(&path).unwrap(),
            "UPDATE plan SET drawn_among = ?1",
            "[]",
        );
        rejected(&reader.team(&team.team).unwrap_err(), "plan", "drawn_among");
    }
}

#[test]
fn reviewer_is_a_run_role_but_not_a_team_plan_role_in_sql_or_when_read() {
    let (_dir, path, mut db) = database();
    let mut team = complete(1).team;
    team.plan.truncate(1);
    team.plan[0].role = Role::Member;
    db.write(&[Record::Team(Box::new(team.clone()))]).unwrap();
    let reader = Reader::open(&path).unwrap();
    assert_eq!(reader.team(&team.team).unwrap().unwrap().value, team);
    corrupt(
        &Connection::open(&path).unwrap(),
        "UPDATE plan SET role = ?1",
        "reviewer",
    );
    rejected(&reader.team(&team.team).unwrap_err(), "plan", "role");
}
