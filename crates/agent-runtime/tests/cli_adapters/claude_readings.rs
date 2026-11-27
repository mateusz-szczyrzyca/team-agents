//! What the Claude Code adapter reads of the `rate_limit_event` records of a run's stream: the newest reading of each
//! window of the account, and nothing at all, no reading and no error, for a record that does not have the shape
//! Claude Code 2.1 writes.

use std::num::NonZeroU32;

use agent_core::cli::{CliAdapter, OutputParser};
use agent_core::limit_reading::{LimitReading, LimitWindow};
use agent_runtime::cli::claude::ClaudeAdapter;
use serde_json::{Value, json};

/// A result whose error the test only reports.
type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

/// `2026-11-12T12:00:00Z`, as Claude Code writes a reset time: seconds since the epoch.
const NOON: i64 = 1_794_484_800;

/// `2026-11-12T16:40:00Z`.
const EVENING: i64 = 1_794_501_600;

/// A `rate_limit_event` line whose `rate_limit_info` is `info`.
fn record(info: &Value) -> String {
    json!({"type": "rate_limit_event", "rate_limit_info": info, "session_id": "s-1"}).to_string()
}

/// A record of Claude Code 2.1 with the windows `unified`.
fn record_with(unified: &Value) -> String {
    record(&json!({"status": "allowed", "rateLimitType": "five_hour", "unifiedWindows": unified}))
}

/// A window as Claude Code 2.1 writes it.
fn window(utilization: f64, resets_at: i64) -> Value {
    json!({"utilization": utilization, "resetsAt": resets_at})
}

/// The reading of `window`, from seconds since the epoch.
fn reading(window: LimitWindow, used: f64, resets_at: i64) -> TestResult<LimitReading> {
    Ok(LimitReading::from_unix(window, used, resets_at)?)
}

/// Feeds `lines` to a new parser of Claude Code, which must take every one of them without an error, and returns
/// the parser.
fn parsed(lines: &[String]) -> TestResult<Box<dyn OutputParser>> {
    let mut parser = ClaudeAdapter.parser();
    for line in lines {
        parser.push_line(line)?;
    }
    Ok(parser)
}

#[test]
fn reading_of_claude_2_1_record() -> TestResult {
    let line = record_with(&json!({"five_hour": window(0.42, NOON), "seven_day": window(0.07, EVENING)}));

    let parser = parsed(&[line])?;

    assert_eq!(
        parser.readings(),
        vec![
            reading(LimitWindow::FiveHour, 0.42, NOON)?,
            reading(LimitWindow::SevenDay, 0.07, EVENING)?
        ]
    );
    Ok(())
}

#[test]
fn a_window_alone_is_a_reading_of_that_window() -> TestResult {
    let parser = parsed(&[record_with(&json!({"seven_day": window(1.0, EVENING)}))])?;

    assert_eq!(parser.readings(), vec![reading(LimitWindow::SevenDay, 1.0, EVENING)?]);
    Ok(())
}

#[test]
fn record_without_windows_is_no_reading() -> TestResult {
    let lines = [
        record(&json!({"status": "allowed", "rateLimitType": "five_hour"})),
        record(&json!({"status": "allowed", "unifiedWindows": {}})),
        record(&json!({"status": "allowed", "unifiedWindows": null})),
        json!({"type": "rate_limit_event", "session_id": "s-1"}).to_string(),
    ];

    let parser = parsed(&lines)?;

    assert_eq!(parser.readings(), Vec::new());
    Ok(())
}

#[test]
fn malformed_windows_is_no_reading_and_no_error() -> TestResult {
    let cases = [
        json!({"five_hour": {"utilization": "0.5", "resetsAt": NOON}}),
        json!({"five_hour": {"utilization": 0.5}}),
        json!({"five_hour": {"utilization": 1.7, "resetsAt": NOON}}),
        json!({"five_hour": {"utilization": -0.2, "resetsAt": NOON}}),
        json!({"five_hour": {"utilization": 0.5, "resetsAt": "soon"}}),
        json!({"five_hour": {"utilization": 0.5, "resetsAt": i64::MAX}}),
        json!({"five_hour": null, "seven_day": 7}),
        json!({"five_hour": [0.5, NOON]}),
        json!("five_hour"),
        json!([window(0.5, NOON)]),
        json!(3),
    ];

    for unified in cases {
        let parser = parsed(&[record_with(&unified)])?;

        assert_eq!(parser.readings(), Vec::new(), "{unified}");
    }
    Ok(())
}

#[test]
fn a_malformed_window_does_not_cost_the_record_its_good_window() -> TestResult {
    let line = record_with(&json!({"five_hour": {"utilization": "?"}, "seven_day": window(0.3, EVENING)}));

    let parser = parsed(&[line])?;

    assert_eq!(parser.readings(), vec![reading(LimitWindow::SevenDay, 0.3, EVENING)?]);
    Ok(())
}

#[test]
fn output_without_records_has_no_readings() -> TestResult {
    let lines = [
        json!({"type": "system", "subtype": "init", "session_id": "s-1", "model": "claude-sonnet-5-5"}).to_string(),
        json!({"type": "result", "subtype": "success", "is_error": false, "result": "STATUS: DONE"}).to_string(),
    ];

    assert_eq!(parsed(&lines)?.readings(), Vec::new());
    assert_eq!(parsed(&[])?.readings(), Vec::new());
    Ok(())
}

#[test]
fn two_records_last_wins() -> TestResult {
    let lines = [
        record_with(&json!({"five_hour": window(0.10, NOON), "seven_day": window(0.50, EVENING)})),
        record_with(&json!({"five_hour": window(0.30, NOON), "seven_day": window(0.55, EVENING)})),
    ];

    let parser = parsed(&lines)?;

    assert_eq!(
        parser.readings(),
        vec![
            reading(LimitWindow::FiveHour, 0.30, NOON)?,
            reading(LimitWindow::SevenDay, 0.55, EVENING)?
        ]
    );
    Ok(())
}

#[test]
fn the_last_record_wins_per_window_so_a_record_of_one_window_keeps_the_other() -> TestResult {
    let lines = [
        record_with(&json!({"five_hour": window(0.10, NOON), "seven_day": window(0.50, EVENING)})),
        record_with(&json!({"five_hour": window(0.90, EVENING)})),
        record_with(&json!({"five_hour": {"utilization": 9.0, "resetsAt": NOON}})),
    ];

    let parser = parsed(&lines)?;

    assert_eq!(
        parser.readings(),
        vec![
            reading(LimitWindow::FiveHour, 0.90, EVENING)?,
            reading(LimitWindow::SevenDay, 0.50, EVENING)?
        ]
    );
    Ok(())
}

#[test]
fn rejected_record_still_feeds_the_error_tail() -> TestResult {
    let line = record(&json!({
        "status": "rejected",
        "resetsAt": NOON,
        "rateLimitType": "five_hour",
        "unifiedWindows": {"five_hour": window(1.0, NOON)},
    }));
    let allowed = record_with(&json!({"seven_day": window(0.2, EVENING)}));

    let parser = parsed(&[allowed.clone(), line.clone()])?;
    let readings = parser.readings();
    let output = parser.finish("", NonZeroU32::new(30).ok_or("30 is not zero")?);

    assert_eq!(
        readings,
        vec![
            reading(LimitWindow::FiveHour, 1.0, NOON)?,
            reading(LimitWindow::SevenDay, 0.2, EVENING)?
        ]
    );
    assert_eq!(output.error_tail.lines(), [line]);
    Ok(())
}
