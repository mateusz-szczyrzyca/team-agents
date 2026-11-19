//! The schema before task permissions remains readable without granting permissions.
use super::*;

#[test]
fn schema_nine_migrates_without_allowances_and_writes_schema_ten() {
    let old = include_str!("../../tests/fixtures/state-before-task-allowances.json");
    let state = State::from_json(old).unwrap();
    assert_eq!(SCHEMA_VERSION, 10);
    for team in state.teams() {
        assert_eq!(team.allowances(), []);
    }
    assert_eq!(
        state.to_json().unwrap(),
        old.replacen("\"schema_version\": 9", "\"schema_version\": 10", 1)
    );
    assert_eq!(State::from_json(&state.to_json().unwrap()).unwrap(), state);
}

/// The schema-9 fixture with `line` put before the brief of its first team.
fn schema_nine_with_brief_line(line: &str) -> String {
    let old = include_str!("../../tests/fixtures/state-before-task-allowances.json");
    let marker = "\"task_list\": \"Build the thing.";
    assert!(old.contains(marker));
    old.replacen(marker, &format!("\"task_list\": \"{line}\\nBuild the thing."), 1)
}

#[test]
fn a_schema_nine_brief_that_starts_with_an_invalid_allow_line_is_read_as_the_brief_it_was() {
    let state = State::from_json(&schema_nine_with_brief_line("Allow: /absolute")).unwrap();
    let team = state.teams().first().unwrap();
    assert_eq!(team.allowances(), []);
    assert!(
        team.task_list()
            .list()
            .brief()
            .contains("Allow: /absolute\nBuild the thing.")
    );
    assert_eq!(State::from_json(&state.to_json().unwrap()).unwrap(), state);
}

#[test]
fn a_schema_nine_brief_that_starts_with_a_valid_allow_line_grants_nothing() {
    let state = State::from_json(&schema_nine_with_brief_line("Allow: docs/*.txt")).unwrap();
    let team = state.teams().first().unwrap();
    assert_eq!(team.allowances(), []);
    assert!(team.task_list().list().brief().contains("Allow: docs/*.txt"));
    let again = State::from_json(&state.to_json().unwrap()).unwrap();
    assert_eq!(again, state);
    assert_eq!(again.teams().first().unwrap().allowances(), []);
}

#[test]
fn a_schema_one_brief_that_has_an_allow_line_is_read_as_the_brief_it_was() {
    let old = include_str!("../../tests/fixtures/state-v1.json");
    let marker = "Coordinator: inv-opus\\n\\nBuild the thing.";
    assert!(old.contains(marker));
    let text = old.replacen(
        marker,
        "Coordinator: inv-opus\\nAllow: src/*.rs\\n\\nBuild the thing.",
        1,
    );
    let state = State::from_json(&text).unwrap();
    let team = state.teams().first().unwrap();
    assert_eq!(team.allowances(), []);
    assert!(team.task_list().list().brief().contains("Allow: src/*.rs"));
    assert_eq!(State::from_json(&state.to_json().unwrap()).unwrap(), state);
}

#[test]
fn a_schema_ten_task_list_keeps_its_allow_lines_as_permissions() {
    let state = State::from_json(&schema_nine_with_brief_line("Allow: docs/*.txt").replacen(
        "\"schema_version\": 9",
        "\"schema_version\": 10",
        1,
    ))
    .unwrap();
    let team = state.teams().first().unwrap();
    assert_eq!(team.allowances().len(), 1);
    assert_eq!(team.allowances().first().unwrap().as_str(), "docs/*.txt");
}

#[test]
fn a_legacy_brief_with_header_allow_lines_is_kept_byte_for_byte_through_the_new_schema() {
    for (line, grants) in [("Allow: /absolute", 0), ("Allow: docs/*.txt", 0)] {
        let state = State::from_json(&schema_nine_with_brief_line(line)).unwrap();
        let team = state.teams().first().unwrap();
        assert_eq!(
            team.task_list().text().lines().take(2).collect::<Vec<_>>(),
            [line, "Build the thing."]
        );
        assert_eq!(team.task_list().list().brief().lines().next(), Some(line));
        assert_eq!(team.allowances().len(), grants);
        let again = State::from_json(&state.to_json().unwrap()).unwrap();
        assert_eq!(again, state);
        let team = again.teams().first().unwrap();
        assert_eq!(team.task_list().text().lines().next(), Some(line));
        assert_eq!(team.allowances().len(), grants);
    }
}

#[test]
fn a_schema_one_brief_with_an_allow_line_is_kept_byte_for_byte() {
    let old = include_str!("../../tests/fixtures/state-v1.json");
    let marker = "Coordinator: inv-opus\\n\\nBuild the thing.";
    let text = old.replacen(
        marker,
        "Coordinator: inv-opus\\nAllow: src/*.rs\\n\\nBuild the thing.",
        1,
    );
    let state = State::from_json(&text).unwrap();
    let team = state.teams().first().unwrap();
    assert!(
        team.task_list()
            .text()
            .starts_with("Coordinator: inv-opus\nAllow: src/*.rs\n\nBuild the thing.")
    );
    assert_eq!(team.allowances(), []);
    assert_eq!(State::from_json(&state.to_json().unwrap()).unwrap(), state);
}

#[test]
fn only_a_task_list_with_brief_allow_lines_is_written_as_an_object() {
    let state = State::from_json(&schema_nine_with_brief_line("Allow: docs/*.txt")).unwrap();
    let json: serde_json::Value = serde_json::from_str(&state.to_json().unwrap()).unwrap();
    let teams = json["teams"].as_array().unwrap();
    assert_eq!(teams[0]["task_list"]["allow_lines"], serde_json::json!("brief"));
    assert!(
        teams[0]["task_list"]["text"]
            .as_str()
            .unwrap()
            .starts_with("Allow: docs/*.txt\n")
    );
    assert!(teams[1]["task_list"].is_string());
}
