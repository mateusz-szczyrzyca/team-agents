use super::*;

/// The time of the tests: the table is read at this moment.
const NOW: &str = "2026-11-25T12:00:00Z";

fn at(text: &str) -> Timestamp {
    text.parse().expect("a time of the tests")
}

fn agent(text: &str) -> AgentName {
    text.parse().expect("an agent name of the tests")
}

/// A row `days_ago` days before `NOW`.
fn row(kind: ReminderKind, name: &str, model: Option<&str>, days_ago: i64, minutes: Option<u64>) -> ReminderRow {
    let when = Timestamp::from_utc(at(NOW).as_utc() - time::Duration::days(days_ago)).expect("a time in range");
    ReminderRow {
        at: when,
        kind,
        agent: agent(name),
        model: model.map(str::to_owned),
        minutes,
    }
}

fn followed(name: &str, minutes: u64) -> ReminderRow {
    row(ReminderKind::Followed, name, Some("sonnet-5-5"), 1, Some(minutes))
}

fn median_of(minutes: &[u64]) -> Option<u64> {
    let rows: Vec<ReminderRow> = minutes.iter().map(|m| followed("claude-1", *m)).collect();
    let table = reminder_table(&rows, at(NOW), 7);
    table.first().and_then(|line| line.median_minutes)
}

#[test]
fn the_median_of_an_odd_count_is_the_middle_value() {
    assert_eq!(median_of(&[20, 5, 9]), Some(9));
}

#[test]
fn the_median_of_an_even_count_is_the_lower_of_the_two_middle_values() {
    assert_eq!(median_of(&[10, 4]), Some(4));
}

#[test]
fn there_is_no_median_when_no_reminder_was_followed() {
    let rows = [row(ReminderKind::Commit, "claude-1", Some("sonnet-5-5"), 1, None)];
    let table = reminder_table(&rows, at(NOW), 7);
    assert_eq!(table.len(), 1);
    assert_eq!(table[0].median_minutes, None);
}

#[test]
fn a_row_older_than_the_days_is_left_out_and_a_row_of_the_last_day_is_kept() {
    let rows = [
        row(ReminderKind::Commit, "claude-1", Some("sonnet-5-5"), 8, None),
        row(ReminderKind::Commit, "claude-1", Some("sonnet-5-5"), 7, None),
        row(ReminderKind::Commit, "claude-1", Some("sonnet-5-5"), 1, None),
    ];
    let table = reminder_table(&rows, at(NOW), 7);
    assert_eq!(table.len(), 1);
    assert_eq!(table[0].commit, 2);
}

#[test]
fn no_row_is_in_zero_days_but_the_rows_of_now() {
    let rows = [row(ReminderKind::Commit, "claude-1", None, 1, None)];
    assert_eq!(reminder_table(&rows, at(NOW), 0), Vec::new());
}

#[test]
fn every_row_is_in_a_number_of_days_that_reaches_before_the_calendar() {
    let rows = [row(ReminderKind::Cut, "claude-1", None, 3000, None)];
    assert_eq!(reminder_table(&rows, at(NOW), u32::MAX).len(), 1);
}

#[test]
fn two_agents_of_one_model_are_two_lines_sorted_by_agent() {
    let rows = [
        row(ReminderKind::Commit, "codex-1", Some("sonnet-5-5"), 1, None),
        row(ReminderKind::Commit, "claude-1", Some("sonnet-5-5"), 1, None),
    ];
    let table = reminder_table(&rows, at(NOW), 7);
    let names: Vec<&str> = table.iter().map(|line| line.agent.as_str()).collect();
    assert_eq!(names, ["claude-1", "codex-1"]);
}

#[test]
fn one_agent_with_two_models_is_two_lines_sorted_by_model() {
    let rows = [
        row(ReminderKind::Commit, "claude-1", Some("sonnet-5-5"), 1, None),
        row(ReminderKind::Context, "claude-1", Some("opus-5-5"), 1, None),
        row(ReminderKind::Commit, "claude-1", None, 1, None),
    ];
    let table = reminder_table(&rows, at(NOW), 7);
    let models: Vec<Option<&str>> = table.iter().map(|line| line.model.as_deref()).collect();
    assert_eq!(models, [None, Some("opus-5-5"), Some("sonnet-5-5")]);
    assert_eq!(table[1].context, 1);
    assert_eq!(table[2].commit, 1);
}

#[test]
fn every_kind_of_row_is_counted_in_its_own_column() {
    let rows = [
        row(ReminderKind::Commit, "claude-1", Some("m"), 1, None),
        row(ReminderKind::Commit, "claude-1", Some("m"), 2, None),
        row(ReminderKind::Commit, "claude-1", Some("m"), 2, None),
        row(ReminderKind::Context, "claude-1", Some("m"), 1, None),
        row(ReminderKind::Followed, "claude-1", Some("m"), 1, Some(12)),
        row(ReminderKind::Followed, "claude-1", Some("m"), 1, Some(6)),
        row(ReminderKind::Cut, "claude-1", Some("m"), 1, None),
    ];
    assert_eq!(
        reminder_table(&rows, at(NOW), 7),
        [ReminderLine {
            agent: agent("claude-1"),
            model: Some("m".to_owned()),
            commit: 3,
            context: 1,
            followed: 2,
            median_minutes: Some(6),
            cuts: 1,
        }]
    );
}

#[test]
fn a_table_of_no_rows_has_no_lines() {
    assert_eq!(reminder_table(&[], at(NOW), 7), Vec::new());
}
