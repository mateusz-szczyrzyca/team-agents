//! `docs/history.md` against the database: every table, column and index of the schema is documented, and the queries of
//! the page run against a round and give what the reading API gives.

use std::collections::BTreeSet;
use std::fmt::Write as _;
use std::path::Path;

use agent_core::ids::{RoundId, RunId, TeamId};
use agent_runtime::history::{Reader, Store};
use rusqlite::{Connection, ToSql};

use super::support::{complete, database};

/// The page.
fn page() -> String {
    std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/history.md")).unwrap()
}

/// The sections of the page: the heading and the text under it, up to the next heading of level two or three.
fn sections(page: &str) -> Vec<(String, String)> {
    let mut found: Vec<(String, String)> = Vec::new();
    for line in page.lines() {
        if line.starts_with("## ") || line.starts_with("### ") {
            found.push((line.to_owned(), String::new()));
        } else if let Some((_, text)) = found.last_mut() {
            text.push_str(line);
            text.push('\n');
        }
    }
    found
}

/// The queries of the page: the blocks marked `sql`, each starting with a comment that names it.
fn queries(page: &str) -> Vec<(String, String)> {
    let mut found = Vec::new();
    let mut block: Option<String> = None;
    for line in page.lines() {
        match (&mut block, line) {
            (None, "```sql") => block = Some(String::new()),
            (Some(text), "```") => {
                let text = std::mem::take(text);
                let name = text.lines().next().unwrap().trim_start_matches("-- ").to_owned();
                found.push((name, text));
                block = None;
            }
            (Some(text), line) => {
                text.push_str(line);
                text.push('\n');
            }
            (None, _) => {}
        }
    }
    found
}

#[test]
fn every_table_column_and_index_of_the_schema_is_documented() {
    let (_dir, path, db) = database();
    drop(db);
    let raw = Connection::open(&path).unwrap();
    let page = page();
    let sections = sections(&page);
    let tables: Vec<String> = raw
        .prepare("SELECT name FROM sqlite_master WHERE type = 'table' ORDER BY name")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    for table in &tables {
        let (heading, text) = sections
            .iter()
            .find(|(heading, _)| heading.starts_with("### ") && heading.contains(&format!("`{table}`")))
            .unwrap_or_else(|| panic!("the page has no section for the table {table}"));
        let columns: Vec<String> = raw
            .prepare(&format!("SELECT name FROM pragma_table_info('{table}')"))
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        for column in columns {
            assert!(
                text.contains(&format!("`{column}`")),
                "the column {column} of {table} is not in the section {heading:?}"
            );
        }
    }
    let indexes: Vec<String> = raw
        .prepare("SELECT name FROM sqlite_master WHERE type = 'index' AND sql IS NOT NULL ORDER BY name")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_ne!(indexes.len(), 0);
    for index in indexes {
        assert!(
            page.contains(&format!("`{index}`")),
            "the index {index} is not on the page"
        );
    }
    for pragma in [
        "journal_mode",
        "synchronous",
        "foreign_keys",
        "busy_timeout",
        "user_version",
    ] {
        assert!(
            page.contains(&format!("`{pragma}`")),
            "the pragma {pragma} is not on the page"
        );
    }
}

/// The values of the named parameters of the page.
struct Names {
    round: RoundId,
    run: RunId,
    team: TeamId,
}

/// The rows a query gives, each as the texts of its columns, with the parameters it names bound.
fn run_query(raw: &Connection, sql: &str, names: &Names) -> Vec<Vec<String>> {
    let mut statement = raw.prepare(sql).unwrap_or_else(|error| panic!("{sql}\n{error}"));
    let mut bound: Vec<(&str, &dyn ToSql)> = Vec::new();
    let values = [names.round.as_str(), names.run.as_str(), names.team.as_str()];
    for (name, value) in [(":round", &values[0]), (":run", &values[1]), (":team", &values[2])] {
        if statement.parameter_index(name).unwrap().is_some() {
            bound.push((name, value as &dyn ToSql));
        }
    }
    let columns = statement.column_count();
    let rows = statement
        .query_map(bound.as_slice(), |row| {
            (0..columns)
                .map(|column| {
                    row.get::<_, rusqlite::types::Value>(column).map(|value| match value {
                        rusqlite::types::Value::Null => "NULL".to_owned(),
                        rusqlite::types::Value::Integer(number) => number.to_string(),
                        rusqlite::types::Value::Real(number) => number.to_string(),
                        rusqlite::types::Value::Text(text) => text,
                        rusqlite::types::Value::Blob(bytes) => format!("{} bytes", bytes.len()),
                    })
                })
                .collect::<Result<Vec<_>, _>>()
        })
        .unwrap();
    rows.collect::<Result<_, _>>().unwrap()
}

#[test]
fn the_queries_of_the_page_list_the_runs_of_a_round_print_a_prompt_whole_and_give_the_steps_and_files_of_a_run() {
    let (_dir, path, mut db) = database();
    let world = complete(21);
    db.write(&world.records()).unwrap();
    let names = Names {
        round: world.rounds[0].round.clone(),
        run: world.runs[0].start.run.clone(),
        team: world.team.team.clone(),
    };
    let raw = Connection::open_with_flags(&path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
    let reader = Reader::open(&path).unwrap();
    let page = page();
    let queries = queries(&page);

    let answers: Vec<(String, Vec<Vec<String>>)> = queries
        .iter()
        .map(|(name, sql)| (name.clone(), run_query(&raw, sql, &names)))
        .collect();

    let answer = |name: &str| {
        &answers
            .iter()
            .find(|(known, _)| known == name)
            .unwrap_or_else(|| panic!("no query {name}"))
            .1
    };
    let runs = reader.runs_of_round(&names.round).unwrap();
    let listed: Vec<(String, String)> = answer("runs of a round")
        .iter()
        .map(|row| (row[2].clone(), row[3].clone()))
        .collect();
    assert_eq!(
        listed,
        runs.iter()
            .map(|row| (row.start.value.agent.name.clone(), row.start.value.role.to_string()))
            .collect::<Vec<_>>()
    );
    assert_ne!(listed.len(), 0);
    let run = reader.run(&names.run).unwrap().unwrap();
    assert_eq!(
        answer("the prompt of a run"),
        &[vec![run.start.value.prompt.clone(), String::new()]]
    );
    assert_eq!(
        answer("the steps of a run").len(),
        reader.steps(&names.run).unwrap().len()
    );
    let changes = reader.changes(&names.run).unwrap().unwrap().value;
    assert_eq!(
        answer("the files a run changed").len(),
        changes.files.len() + changes.uncommitted.len()
    );
    assert_eq!(answer("the commits of a run").len(), changes.commits.len());
    assert_eq!(answer("the plan of a team").len(), world.team.plan.len());
    assert_eq!(answer("the launch of a run").len(), 1);
    assert_eq!(answer("the item of a round").len(), 1);
    assert_eq!(answer("how the runs of a team ended").len(), world.runs.len());
    assert_eq!(answer("the prompts of a round").len(), runs.len());
    let names_in_page: BTreeSet<&str> = queries.iter().map(|(name, _)| name.as_str()).collect();
    assert_eq!(
        names_in_page.len(),
        queries.len(),
        "two queries of the page have one name"
    );
}

#[test]
fn what_the_queries_of_the_page_print_for_a_round_is_pinned() {
    let (_dir, path, mut db) = database();
    let world = complete(21);
    db.write(&world.records()).unwrap();
    let names = Names {
        round: world.rounds[0].round.clone(),
        run: world.runs[0].start.run.clone(),
        team: world.team.team.clone(),
    };
    let raw = Connection::open_with_flags(&path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();

    let mut printed = String::new();
    for (name, sql) in queries(&page()) {
        writeln!(printed, "-- {name}").unwrap();
        for row in run_query(&raw, &sql, &names) {
            printed.push_str(&row.join(" | "));
            printed.push('\n');
        }
        printed.push('\n');
    }

    let golden = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/history-queries.golden.txt");
    assert_eq!(printed, std::fs::read_to_string(golden).unwrap());
}
