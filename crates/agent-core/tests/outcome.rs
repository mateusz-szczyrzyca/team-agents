//! Run outcomes of the four model CLIs: `tests/fixtures/cli-outputs/<cli>.json` holds, per CLI, the final message,
//! error output, exit status and stream result of runs that ended in each way, with what `classify_outcome` is to make
//! of them. The texts are reconstructed (see each file's `provenance`), not captured from live runs.

#[cfg(test)]
mod outcome {
    use std::collections::BTreeSet;

    use agent_core::cli::{ErrorTail, RunOutput, StreamOutcome};
    use agent_core::config::Config;
    use agent_core::decide::outcome::{
        Classified, ExitStatus, LimitContext, Pause, RunFinish, StoppedBy, classify_outcome,
    };
    use agent_core::kinds::CliKind;
    use agent_core::state::{RunEnd, Timestamp};
    use serde::Deserialize;
    use time::UtcOffset;

    /// The clock of every fixture: the time the run ended.
    const NOW: &str = "2026-11-12T10:00:00Z";

    /// How the stream of a case ended.
    #[derive(Debug, Clone, Copy, Deserialize)]
    #[serde(rename_all = "snake_case")]
    enum Stream {
        Succeeded,
        Failed,
        Incomplete,
    }

    /// Who ended the run of a case.
    #[derive(Debug, Clone, Copy, Default, Deserialize)]
    #[serde(rename_all = "snake_case")]
    enum Stopper {
        #[default]
        Nobody,
        Watchdog,
        Human,
    }

    /// The pause a case expects.
    #[derive(Debug, Deserialize)]
    #[serde(deny_unknown_fields)]
    struct ExpectedPause {
        until: Timestamp,
        reason: String,
        evidence: String,
    }

    /// What a case expects of the classification.
    #[derive(Debug, Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Expect {
        end: RunEnd,
        pause: Option<ExpectedPause>,
    }

    /// One run of a CLI.
    #[derive(Debug, Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Case {
        name: String,
        #[serde(default)]
        final_message: Option<String>,
        #[serde(default)]
        stderr: String,
        /// The exit code; none when a signal ended the process.
        exit: Option<i32>,
        stream: Stream,
        #[serde(default)]
        stopped_by: Stopper,
        expect: Expect,
    }

    /// A fixture file.
    #[derive(Debug, Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Fixture {
        cli: CliKind,
        provenance: String,
        cases: Vec<Case>,
    }

    fn fixtures() -> Vec<Fixture> {
        [
            include_str!("fixtures/cli-outputs/claude.json"),
            include_str!("fixtures/cli-outputs/codex.json"),
            include_str!("fixtures/cli-outputs/gemini.json"),
            include_str!("fixtures/cli-outputs/opencode.json"),
        ]
        .iter()
        .map(|text| serde_json::from_str(text).unwrap())
        .collect()
    }

    /// What `classify_outcome` makes of `case`, with the default limits and a machine one hour ahead of UTC.
    fn classify(case: &Case) -> Classified {
        let config = Config::default();
        let output = RunOutput {
            final_message: case.final_message.clone(),
            session_id: None,
            stream: match case.stream {
                Stream::Succeeded => StreamOutcome::Succeeded,
                Stream::Failed => StreamOutcome::Failed,
                Stream::Incomplete => StreamOutcome::Incomplete,
            },
            error_tail: ErrorTail::from_text(&case.stderr, config.limits().tail_lines),
        };
        let finish = RunFinish {
            exit: case.exit.map_or(ExitStatus::Signal, ExitStatus::Code),
            stopped_by: match case.stopped_by {
                Stopper::Nobody => StoppedBy::Nobody,
                Stopper::Watchdog => StoppedBy::Watchdog,
                Stopper::Human => StoppedBy::Human,
            },
        };
        let context = LimitContext {
            limits: config.limits(),
            now: NOW.parse().unwrap(),
            local_offset: UtcOffset::from_hms(1, 0, 0).unwrap(),
        };
        classify_outcome(&output, finish, &context).unwrap()
    }

    /// The pause as a case writes it.
    fn written(pause: &Pause) -> (String, String, String) {
        (
            pause.until.to_string(),
            pause.reason.to_string(),
            pause.evidence.clone(),
        )
    }

    fn expected(pause: &ExpectedPause) -> (String, String, String) {
        (pause.until.to_string(), pause.reason.clone(), pause.evidence.clone())
    }

    #[test]
    fn every_case_of_every_cli_is_classified_as_recorded() {
        for fixture in fixtures() {
            for case in &fixture.cases {
                let got = classify(case);
                let label = format!("{} / {}", fixture.cli, case.name);
                assert_eq!(got.end, case.expect.end, "{label}: end");
                assert_eq!(
                    got.pause.as_ref().map(written),
                    case.expect.pause.as_ref().map(expected),
                    "{label}: pause"
                );
            }
        }
    }

    /// The text of the kind of an end: `reported DONE`, `quota_exhausted`, `hung`, ...
    fn kind_of(end: &RunEnd) -> String {
        match end {
            RunEnd::Reported { outcome } => format!("reported {outcome}"),
            RunEnd::NoStatus { .. } => "no_status".to_owned(),
            RunEnd::QuotaExhausted { .. } => "quota_exhausted".to_owned(),
            RunEnd::RateLimited => "rate_limited".to_owned(),
            RunEnd::Hung => "hung".to_owned(),
            RunEnd::Stopped => "stopped".to_owned(),
        }
    }

    #[test]
    fn every_cli_has_a_case_of_each_outcome_kind() {
        let wanted: BTreeSet<&str> = [
            "reported DONE",
            "reported QUESTIONS",
            "reported FAILED",
            "quota_exhausted",
            "rate_limited",
            "hung",
            "stopped",
            "no_status",
        ]
        .into();
        for fixture in fixtures() {
            let found: BTreeSet<String> = fixture.cases.iter().map(|c| kind_of(&c.expect.end)).collect();
            for kind in &wanted {
                assert!(found.contains(*kind), "{}: no case of {kind}", fixture.cli);
            }
        }
    }

    #[test]
    fn there_is_a_fixture_for_every_cli_and_it_says_where_its_texts_come_from() {
        let fixtures = fixtures();
        let clis: Vec<CliKind> = fixtures.iter().map(|f| f.cli).collect();
        assert_eq!(clis, CliKind::ALL);
        for fixture in &fixtures {
            assert!(!fixture.provenance.trim().is_empty(), "{}: no provenance", fixture.cli);
            let names: BTreeSet<&str> = fixture.cases.iter().map(|c| c.name.as_str()).collect();
            assert_eq!(names.len(), fixture.cases.len(), "{}: a case name twice", fixture.cli);
        }
    }

    #[test]
    fn every_cli_has_a_quota_case_with_a_reset_time() {
        for fixture in fixtures() {
            let resets: Vec<bool> = fixture
                .cases
                .iter()
                .filter_map(|c| match c.expect.end {
                    RunEnd::QuotaExhausted { resets_at } => Some(resets_at.is_some()),
                    RunEnd::Reported { .. }
                    | RunEnd::NoStatus { .. }
                    | RunEnd::RateLimited
                    | RunEnd::Hung
                    | RunEnd::Stopped => None,
                })
                .collect();
            assert!(resets.contains(&true), "{}: no quota with a reset time", fixture.cli);
        }
    }
}
