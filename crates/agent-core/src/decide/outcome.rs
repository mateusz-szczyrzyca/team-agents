//! Run outcomes: how a finished model run is told apart, and the pause each outcome implies.
//!
//! [`classify_outcome`] reads what the runtime saw of a run — the CLI's final message, its exit status, the end of its
//! error output and how the human or the watchdog ended it — and returns the [`RunEnd`] to record and the [`Pause`] the
//! task takes, if any. The old `bin/agent` rules (`limit_kind`, `quota_reset_seconds`) are the reference.
//!
//! The decision runs in this order:
//!
//! 1. The human stopped the run: [`RunEnd::Stopped`].
//! 2. The final message has a recognised `STATUS:` line ([`parse_status_line`]) and the run succeeded — or the
//!    watchdog ended it, which only a silent run needs —: [`RunEnd::Reported`].
//! 3. The watchdog ended the run: [`RunEnd::Hung`].
//! 4. The run failed — a failed exit status, or a stream that did not end with a successful result — and its error
//!    tail shows a provider limit: [`RunEnd::QuotaExhausted`] for an exhausted quota, usage limit or credits,
//!    [`RunEnd::RateLimited`] for a rate limit (HTTP 429) or an overloaded model (HTTP 503 and 529: a pause as short
//!    as a rate limit's). A line that is no provider's — the machine ran out of processes, memory or disk, a Unix
//!    socket failed — is never a limit. The final message is read as an error only when it has no `STATUS:` line: the
//!    agent's own report is never taken for a provider error, and a clean run is never paused for a stale 429 that
//!    the CLI retried.
//! 5. A recognised `STATUS:` line of a run that failed without a limit: [`RunEnd::Reported`].
//! 6. Anything else: [`RunEnd::NoStatus`] with the exit code.
//!
//! The pause of a limit lasts until the time the CLI gives — a Unix time after `limit reached|`, a duration after
//! `resets in`, `try again in`, `retry after`, an ISO time, a date such as `Nov 12th, 2026 3:04 PM` or a time of day
//! such as `3pm` in the machine's local time — plus [`RESET_MARGIN`]; without one, or with one in the past, until
//! `[limits]` `quota_pause_minutes` or `rate_limit_pause_minutes` have passed, plus the margin. It is never longer than
//! [`MAX_PAUSE`]. Hangs and everything else pause nothing; [`after_hang`] decides whether a hung run is resumed.

use std::time::Duration;

use time::UtcOffset;

use crate::cli::{RunOutput, SessionId};
use crate::config::{Limits, Watchdog};
use crate::state::{RunEnd, Timestamp};
use crate::status::RunOutcome;

/// Seconds added to every pause, so that a run does not start a moment before the limit really ends.
pub const RESET_MARGIN: Duration = Duration::from_mins(1);

/// The longest pause a limit causes, whatever reset time the CLI gives: a bound on untrusted text. Eight days cover a
/// weekly limit.
pub const MAX_PAUSE: Duration = Duration::from_hours(8 * 24);

/// Longest part of the line that showed a limit that a [`Pause`] keeps, in characters.
pub const MAX_EVIDENCE_CHARS: usize = 200;

/// What the final message of a run says about its status.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StatusLine {
    /// No line of the message is a `STATUS:` line.
    Absent,
    /// The first `STATUS:` line has one of the words of [`RunOutcome`].
    Reported(RunOutcome),
    /// The first `STATUS:` line has a word that is no [`RunOutcome`], such as the dropped `ROUND_DONE`.
    Unknown(String),
}

/// How the CLI process ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExitStatus {
    /// It exited with this code.
    Code(i32),
    /// A signal ended it: no code.
    Signal,
}

/// Who ended a run before the CLI did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StoppedBy {
    /// Nobody: the CLI ended by itself.
    Nobody,
    /// The watchdog, after the run showed no activity for too long.
    Watchdog,
    /// The human, with `agent stop`.
    Human,
}

/// How a run's process ended, as the runtime saw it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RunFinish {
    /// The CLI's exit status.
    pub exit: ExitStatus,
    /// Whether somebody ended the run.
    pub stopped_by: StoppedBy,
}

/// What [`classify_outcome`] needs to turn a limit into a pause.
#[derive(Debug, Clone, Copy)]
pub struct LimitContext<'a> {
    /// The configured default pauses.
    pub limits: &'a Limits,
    /// When the run ended.
    pub now: Timestamp,
    /// The machine's offset from UTC, with which a time of day or a date without a zone is read.
    pub local_offset: UtcOffset,
}

/// Why a task is paused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, strum::Display, strum::IntoStaticStr)]
#[strum(serialize_all = "snake_case")]
pub enum PauseReason {
    /// The quota, usage limit or credits are exhausted.
    Quota,
    /// The provider limits the rate of requests, or its model is overloaded.
    RateLimit,
}

/// A pause a run's end implies.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pause {
    /// When the task is queued again.
    pub until: Timestamp,
    /// Why it waits.
    pub reason: PauseReason,
    /// The line of the CLI's output that showed the limit, trimmed and cut after [`MAX_EVIDENCE_CHARS`] characters,
    /// for the notification.
    pub evidence: String,
}

/// How a run ended and what that implies.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Classified {
    /// What to record as the end of the run.
    pub end: RunEnd,
    /// The pause the task takes; only a limit has one.
    pub pause: Option<Pause>,
}

/// Why a run cannot be classified.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ClassifyError {
    /// The pause would end after the last time a timestamp can hold, year 9999.
    #[error("a pause from {now} would end after the year 9999")]
    PauseOverflow {
        /// When the run ended.
        now: Timestamp,
    },
}

/// The status line of a final message: the first line that starts with `STATUS:` once the markup characters `*` and
/// `` ` `` are dropped, followed by a word of capital letters and `_`.
#[must_use]
pub fn parse_status_line(message: &str) -> StatusLine {
    let _ = message;
    StatusLine::Absent
}

/// Classifies a finished run.
///
/// # Errors
/// [`ClassifyError::PauseOverflow`] when the pause would end beyond the years a timestamp can hold.
pub fn classify_outcome(
    output: &RunOutput,
    finish: RunFinish,
    context: &LimitContext<'_>,
) -> Result<Classified, ClassifyError> {
    let _ = (output, finish, context);
    Ok(Classified {
        end: RunEnd::Hung,
        pause: None,
    })
}

/// What happens to a task whose run hung.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AfterHang {
    /// The run is started again in the same session.
    Resume,
    /// The task is marked hung and waits for the human.
    MarkHung,
}

/// Whether a hung run is resumed: the watchdog's `hang_strikes` hangs in a row without progress mark the task hung,
/// and so does a hang of a run with no session to resume.
///
/// `hangs_in_a_row` counts this hang too; whoever counts resets it when a run made progress.
#[must_use]
pub fn after_hang(hangs_in_a_row: u32, session: Option<&SessionId>, watchdog: &Watchdog) -> AfterHang {
    let _ = (hangs_in_a_row, session, watchdog);
    AfterHang::Resume
}

#[cfg(test)]
mod tests {
    use std::num::NonZeroU32;

    use super::*;
    use crate::cli::{ErrorTail, StreamOutcome};
    use crate::config::Config;

    /// When the runs of these tests end.
    const NOW: &str = "2026-11-12T10:00:00Z";

    /// The machine's offset in these tests: one hour ahead of UTC.
    fn local() -> UtcOffset {
        UtcOffset::from_hms(1, 0, 0).unwrap()
    }

    fn at(text: &str) -> Timestamp {
        text.parse().unwrap()
    }

    /// The configuration's default limits: a quota pause of 30 minutes and a rate limit pause of 5.
    fn default_config() -> Config {
        Config::default()
    }

    /// A run that ended by itself with `exit`, `stream`, the final message and the error output given.
    fn classify_with(
        config: &Config,
        message: Option<&str>,
        stderr: &str,
        exit: ExitStatus,
        stream: StreamOutcome,
        stopped_by: StoppedBy,
    ) -> Classified {
        let output = RunOutput {
            final_message: message.map(str::to_owned),
            session_id: None,
            stream,
            error_tail: ErrorTail::from_text(stderr, config.limits().tail_lines),
        };
        let context = LimitContext {
            limits: config.limits(),
            now: at(NOW),
            local_offset: local(),
        };
        classify_outcome(&output, RunFinish { exit, stopped_by }, &context).unwrap()
    }

    /// A run that failed with exit code 1: the stream ended with an error result.
    fn failed(message: Option<&str>, stderr: &str) -> Classified {
        classify_with(
            &default_config(),
            message,
            stderr,
            ExitStatus::Code(1),
            StreamOutcome::Failed,
            StoppedBy::Nobody,
        )
    }

    /// A run that ended well: exit code 0 and a successful result.
    fn clean(message: Option<&str>, stderr: &str) -> Classified {
        classify_with(
            &default_config(),
            message,
            stderr,
            ExitStatus::Code(0),
            StreamOutcome::Succeeded,
            StoppedBy::Nobody,
        )
    }

    fn quota(resets_at: Option<&str>) -> RunEnd {
        RunEnd::QuotaExhausted {
            resets_at: resets_at.map(at),
        }
    }

    /// The pause of `class`: its end time and reason; panics when there is none.
    fn paused(class: &Classified) -> (Timestamp, PauseReason) {
        let pause = class.pause.as_ref().unwrap();
        (pause.until, pause.reason)
    }

    // Rule: the status line is the first line `STATUS: WORD`, markup and indentation aside.
    #[test]
    fn the_status_line_names_one_of_the_run_outcomes() {
        for (text, outcome) in [
            ("DONE", RunOutcome::Done),
            ("QUESTIONS", RunOutcome::Questions),
            ("FAILED", RunOutcome::Failed),
            ("SLEEPING", RunOutcome::Sleeping),
            ("EXCLUDED", RunOutcome::Excluded),
        ] {
            assert_eq!(
                parse_status_line(&format!("Did things.\n\nSTATUS: {text}\nRole: member")),
                StatusLine::Reported(outcome)
            );
        }
    }

    #[test]
    fn the_status_line_ignores_markup_indentation_and_spacing() {
        for message in [
            "**STATUS:** DONE",
            "`STATUS: DONE`",
            "  STATUS:   DONE  ",
            "text\r\nSTATUS: DONE\r\n",
            "STATUS:DONE",
            "**STATUS: DONE**",
        ] {
            assert_eq!(
                parse_status_line(message),
                StatusLine::Reported(RunOutcome::Done),
                "{message:?}"
            );
        }
    }

    #[test]
    fn the_first_status_line_wins() {
        assert_eq!(
            parse_status_line("STATUS: QUESTIONS\n\nlater STATUS: DONE\nSTATUS: DONE"),
            StatusLine::Reported(RunOutcome::Questions)
        );
    }

    #[test]
    fn a_status_in_the_middle_of_a_line_or_in_another_case_is_no_status_line() {
        for message in [
            "",
            "all done",
            "The line STATUS: DONE is what you print.",
            "status: DONE",
            "STATUS:",
            "STATUS: done",
            "STATUS 1",
            "żółć STATUS: DONE",
        ] {
            assert_eq!(parse_status_line(message), StatusLine::Absent, "{message:?}");
        }
    }

    #[test]
    fn a_status_word_of_the_old_harness_is_unknown() {
        assert_eq!(
            parse_status_line("STATUS: ROUND_DONE"),
            StatusLine::Unknown("ROUND_DONE".to_owned())
        );
        assert_eq!(
            parse_status_line("STATUS: INVESTIGATING"),
            StatusLine::Unknown("INVESTIGATING".to_owned())
        );
    }

    // Rule: the agent's own status is the end of a run that succeeded.
    #[test]
    fn a_reported_status_ends_a_clean_run() {
        for (word, outcome) in [
            ("DONE", RunOutcome::Done),
            ("QUESTIONS", RunOutcome::Questions),
            ("FAILED", RunOutcome::Failed),
            ("SLEEPING", RunOutcome::Sleeping),
            ("EXCLUDED", RunOutcome::Excluded),
        ] {
            let class = clean(Some(&format!("Work.\n\nSTATUS: {word}")), "");
            assert_eq!(
                class,
                Classified {
                    end: RunEnd::Reported { outcome },
                    pause: None
                }
            );
        }
    }

    // Rule: the human's stop ends a run whatever it said.
    #[test]
    fn the_humans_stop_is_stopped_whatever_the_run_said() {
        let class = classify_with(
            &default_config(),
            Some("STATUS: DONE"),
            "429 Too Many Requests",
            ExitStatus::Signal,
            StreamOutcome::Incomplete,
            StoppedBy::Human,
        );
        assert_eq!(
            class,
            Classified {
                end: RunEnd::Stopped,
                pause: None
            }
        );
    }

    // Rule: a run the watchdog killed is hung, and pauses nothing.
    #[test]
    fn a_run_the_watchdog_killed_is_hung() {
        let class = classify_with(
            &default_config(),
            None,
            "",
            ExitStatus::Signal,
            StreamOutcome::Incomplete,
            StoppedBy::Watchdog,
        );
        assert_eq!(
            class,
            Classified {
                end: RunEnd::Hung,
                pause: None
            }
        );
    }

    // Rule: hung wins over a limit line in the tail of a silent run.
    #[test]
    fn a_hung_run_is_not_a_limit() {
        let class = classify_with(
            &default_config(),
            None,
            "rate limit exceeded",
            ExitStatus::Signal,
            StreamOutcome::Incomplete,
            StoppedBy::Watchdog,
        );
        assert_eq!(class.end, RunEnd::Hung);
        assert_eq!(class.pause, None);
    }

    // Rule: a run that reported a status before the watchdog ended it did report it.
    #[test]
    fn a_status_reported_before_the_watchdog_killed_the_run_stands() {
        let class = classify_with(
            &default_config(),
            Some("STATUS: DONE"),
            "",
            ExitStatus::Signal,
            StreamOutcome::Incomplete,
            StoppedBy::Watchdog,
        );
        assert_eq!(
            class.end,
            RunEnd::Reported {
                outcome: RunOutcome::Done
            }
        );
    }

    // Rule: a run without a recognised status ends as "no status", with its exit code.
    #[test]
    fn a_run_without_a_status_has_no_status_and_its_exit_code() {
        let ended = |exit, stream| {
            classify_with(
                &default_config(),
                Some("May I continue?"),
                "",
                exit,
                stream,
                StoppedBy::Nobody,
            )
        };
        assert_eq!(
            ended(ExitStatus::Code(0), StreamOutcome::Succeeded).end,
            RunEnd::NoStatus { exit_code: Some(0) }
        );
        assert_eq!(
            ended(ExitStatus::Code(3), StreamOutcome::Failed).end,
            RunEnd::NoStatus { exit_code: Some(3) }
        );
        assert_eq!(
            ended(ExitStatus::Signal, StreamOutcome::Incomplete).end,
            RunEnd::NoStatus { exit_code: None }
        );
        assert_eq!(
            classify_with(
                &default_config(),
                None,
                "",
                ExitStatus::Code(0),
                StreamOutcome::Succeeded,
                StoppedBy::Nobody
            )
            .pause,
            None
        );
    }

    // Rule: a status word the harness dropped is no status, and the message is not read for limits either.
    #[test]
    fn an_unknown_status_word_is_no_status() {
        let class = failed(Some("Handled the rate limit.\nSTATUS: ROUND_DONE"), "");
        assert_eq!(
            class,
            Classified {
                end: RunEnd::NoStatus { exit_code: Some(1) },
                pause: None
            }
        );
    }

    // Rule: a recognised status of a run that failed without a limit still stands.
    #[test]
    fn a_reported_status_of_a_failed_run_without_a_limit_stands() {
        let class = failed(Some("Work.\nSTATUS: FAILED"), "some warning");
        assert_eq!(
            class.end,
            RunEnd::Reported {
                outcome: RunOutcome::Failed
            }
        );
        assert_eq!(class.pause, None);
    }

    // Rule: a stream that ended with a successful result stands even when the CLI exited with an error code.
    #[test]
    fn a_successful_result_stands_with_a_failed_exit_code() {
        let class = classify_with(
            &default_config(),
            Some("Work.\nSTATUS: DONE"),
            "usage limit reached",
            ExitStatus::Code(1),
            StreamOutcome::Succeeded,
            StoppedBy::Nobody,
        );
        assert_eq!(
            class,
            Classified {
                end: RunEnd::Reported {
                    outcome: RunOutcome::Done
                },
                pause: None
            }
        );
    }

    // Rule: an exhausted quota pauses the task for the configured default when the CLI gives no reset time.
    #[test]
    fn an_exhausted_quota_pauses_for_the_default() {
        let class = failed(
            None,
            "Error: Individual quota reached. Please upgrade your subscription.",
        );
        assert_eq!(class.end, quota(None));
        assert_eq!(paused(&class), (at("2026-11-12T10:31:00Z"), PauseReason::Quota));
        assert_eq!(
            class.pause.map(|p| p.evidence),
            Some("Error: Individual quota reached. Please upgrade your subscription.".to_owned())
        );
    }

    // Rule: every provider limit of the old harness is a quota.
    #[test]
    fn the_words_of_an_exhausted_quota() {
        for line in [
            "Quota exceeded for quota metric 'Generate Content API requests'",
            "quota exhausted",
            "You have hit your usage limit.",
            "5-hour limit reached",
            "Claude AI usage limit reached",
            "RESOURCE_EXHAUSTED: check quota",
            "Resource has been exhausted (e.g. check quota).",
            "Error: out of credits",
            "Insufficient credits. Add more at https://example.com/credits",
            "insufficient balance on the account",
            "Workspace monthly budget of $200.00 exceeded.",
            "Your budget is exhausted",
            "budget reached",
            "Monthly spending limit reached",
            "spend limit exceeded",
            "HTTP 402 Payment Required",
            "You have exhausted your capacity on this model.",
            "status: 402",
        ] {
            let class = failed(None, line);
            assert_eq!(class.end, quota(None), "{line}");
            assert_eq!(class.pause.map(|p| p.reason), Some(PauseReason::Quota), "{line}");
        }
    }

    // Rule: a rate limit is a short pause, not an exhausted quota.
    #[test]
    fn a_rate_limit_pauses_for_its_default() {
        for line in [
            "429 Too Many Requests: rate limit exceeded",
            "Error: rate_limit_error",
            "rate-limit hit",
            "RateLimit exceeded",
            "Too many requests, slow down",
            "HTTP 429",
            "stream error: exceeded retry limit, last status: 429",
        ] {
            let class = failed(None, line);
            assert_eq!(class.end, RunEnd::RateLimited, "{line}");
            assert_eq!(
                paused(&class),
                (at("2026-11-12T10:06:00Z"), PauseReason::RateLimit),
                "{line}"
            );
        }
    }

    // Rule: a model overloaded at the provider pauses like a rate limit.
    #[test]
    fn an_overloaded_model_pauses_like_a_rate_limit() {
        for line in [
            "Selected model is at capacity right now",
            "API Error: 529 Overloaded",
            "overloaded_error",
            "HTTP 503 Service Unavailable",
            "The service is temporarily unavailable",
        ] {
            let class = failed(None, line);
            assert_eq!(class.end, RunEnd::RateLimited, "{line}");
            assert_eq!(class.pause.map(|p| p.reason), Some(PauseReason::RateLimit), "{line}");
        }
    }

    // Rule: a quota line wins over a rate limit line, in one line or in two.
    #[test]
    fn a_quota_wins_over_a_rate_limit() {
        assert_eq!(failed(None, "429 RESOURCE_EXHAUSTED").end, quota(None));
        assert_eq!(
            failed(None, "429 Too Many Requests\nQuota exceeded for today").end,
            quota(None)
        );
        assert_eq!(
            failed(None, "Quota exceeded for today\n429 Too Many Requests").end,
            quota(None)
        );
    }

    // Rule: "rate limit reached" is a rate limit, though "limit reached" alone is a quota.
    #[test]
    fn rate_limit_reached_is_a_rate_limit() {
        assert_eq!(failed(None, "Rate limit reached for requests").end, RunEnd::RateLimited);
        assert_eq!(failed(None, "weekly limit reached").end, quota(None));
    }

    // Rule: a context window that is full is nothing a pause helps.
    #[test]
    fn a_context_limit_is_no_provider_limit() {
        let class = failed(None, "Context window limit reached");
        assert_eq!(
            class,
            Classified {
                end: RunEnd::NoStatus { exit_code: Some(1) },
                pause: None
            }
        );
    }

    // Rule: an error that is no limit is no pause.
    #[test]
    fn an_error_that_is_no_limit_has_no_pause() {
        let class = failed(None, "Error: invalid argument --frobnicate");
        assert_eq!(
            class,
            Classified {
                end: RunEnd::NoStatus { exit_code: Some(1) },
                pause: None
            }
        );
    }

    // Rule: errors of the machine are no provider's limit, however they are worded.
    #[test]
    fn a_local_error_is_no_limit() {
        for line in [
            "fork: Resource temporarily unavailable",
            "fork: retry: no child processes; too many requests",
            "cannot allocate memory: rate limit",
            "too many open files",
            "no space left on device (quota exceeded)",
            "dial unix /run/user/1000/agent.sock: i/o timeout, 429",
            "read unix @->/tmp/x: use of closed connection: overloaded",
            "listening on unix:/tmp/agentbus.sock: usage limit",
            "connect: /home/dev/agents/bus.sock: resource exhausted",
        ] {
            let class = failed(None, line);
            assert_eq!(
                class,
                Classified {
                    end: RunEnd::NoStatus { exit_code: Some(1) },
                    pause: None
                },
                "{line}"
            );
        }
    }

    // Rule: a local error line does not hide a provider's limit in another line.
    #[test]
    fn a_local_error_does_not_hide_a_limit_line() {
        let class = failed(None, "dial unix /tmp/x.sock: connection refused\nusage limit reached");
        assert_eq!(class.end, quota(None));
    }

    // Rule: a run that ended well is not paused for a limit line of its error output.
    #[test]
    fn a_clean_run_is_not_paused_for_a_stale_429() {
        let class = clean(Some("Work.\nSTATUS: DONE"), "429 Too Many Requests (retrying in 2s)");
        assert_eq!(
            class,
            Classified {
                end: RunEnd::Reported {
                    outcome: RunOutcome::Done
                },
                pause: None
            }
        );
        let class = clean(Some("May I continue?"), "429 Too Many Requests (retrying in 2s)");
        assert_eq!(
            class,
            Classified {
                end: RunEnd::NoStatus { exit_code: Some(0) },
                pause: None
            }
        );
    }

    // Rule: the agent's own report is never read as a provider error.
    #[test]
    fn the_agents_own_report_is_not_read_for_limits() {
        let class = failed(Some("Implemented the rate limit handling.\n\nSTATUS: DONE"), "");
        assert_eq!(
            class.end,
            RunEnd::Reported {
                outcome: RunOutcome::Done
            }
        );
        assert_eq!(class.pause, None);
    }

    // Rule: a final message without a status is read as an error when the run failed: it is where some CLIs put it.
    #[test]
    fn the_final_message_of_a_failed_run_without_status_is_read_for_limits() {
        let class = failed(Some("Claude AI usage limit reached|1794478200"), "");
        assert_eq!(class.end, quota(Some("2026-11-12T10:10:00Z")));
    }

    // Rule: a stream that ended with an error result is a failed run whatever the exit code.
    #[test]
    fn a_stream_that_ended_with_an_error_is_a_failed_run() {
        let class = classify_with(
            &default_config(),
            None,
            "Error: 429 rate limit",
            ExitStatus::Code(0),
            StreamOutcome::Failed,
            StoppedBy::Nobody,
        );
        assert_eq!(class.end, RunEnd::RateLimited);
        let class = classify_with(
            &default_config(),
            None,
            "Error: 429 rate limit",
            ExitStatus::Code(0),
            StreamOutcome::Incomplete,
            StoppedBy::Nobody,
        );
        assert_eq!(class.end, RunEnd::RateLimited);
    }

    /// The end and the pause end of a quota failure whose error output is `line`.
    fn reset_of(line: &str) -> (RunEnd, Timestamp) {
        let class = failed(None, line);
        (class.end.clone(), class.pause.unwrap().until)
    }

    // Rule: a Unix time after "limit reached|" is the reset time; the pause ends a minute after it.
    #[test]
    fn a_unix_reset_time_is_read() {
        assert_eq!(
            reset_of("Claude AI usage limit reached|1794478200"),
            (quota(Some("2026-11-12T10:10:00Z")), at("2026-11-12T10:11:00Z"))
        );
    }

    // Rule: a reset time that has passed is no reset time: the default pause applies.
    #[test]
    fn a_reset_time_in_the_past_gives_the_default_pause() {
        assert_eq!(
            reset_of("Claude AI usage limit reached|1794474000"),
            (quota(None), at("2026-11-12T10:31:00Z"))
        );
        assert_eq!(
            reset_of("usage limit reached; resets 2026-11-01T00:00:00Z"),
            (quota(None), at("2026-11-12T10:31:00Z"))
        );
    }

    // Rule: a duration after "resets in", "try again in", "retry after" is the time until the reset.
    #[test]
    fn a_reset_duration_is_read() {
        for (line, resets_at) in [
            ("Individual quota reached. Resets in 24m46s.", "2026-11-12T10:24:46Z"),
            (
                "Error: You have hit your usage limit. Try again in 2 hours 13 minutes.",
                "2026-11-12T12:13:00Z",
            ),
            ("usage limit reached, retry after 90s", "2026-11-12T10:01:30Z"),
            (
                "You have exhausted your capacity on this model. Your quota will reset after 2h28m14s.",
                "2026-11-12T12:28:14Z",
            ),
            (
                "Quota exceeded for quota metric 'requests per minute'. Please retry in 36.99s",
                "2026-11-12T10:00:37Z",
            ),
            (
                "usage limit: try again in 2 days 4 hours 5 minutes",
                "2026-11-14T14:05:00Z",
            ),
            (
                r#"{"error": {"status": "RESOURCE_EXHAUSTED", "retryDelay": "36s"}}"#,
                "2026-11-12T10:00:36Z",
            ),
            ("usage limit reached. Try again in 1h 5m", "2026-11-12T11:05:00Z"),
            (
                "usage limit reached, try again in 3 hours, 4 minutes and 5 seconds",
                "2026-11-12T13:04:05Z",
            ),
        ] {
            let (end, until) = reset_of(line);
            assert_eq!(end, quota(Some(resets_at)), "{line}");
            let expected_until = at(resets_at).as_utc() + time::Duration::seconds(60);
            assert_eq!(until.as_utc(), expected_until, "{line}");
        }
    }

    // Rule: a rate limit that says when to retry pauses until then; the end records no reset time.
    #[test]
    fn a_rate_limit_with_a_retry_time_pauses_until_it() {
        let class = failed(None, "429 Too Many Requests. Please retry after 90s");
        assert_eq!(class.end, RunEnd::RateLimited);
        assert_eq!(paused(&class), (at("2026-11-12T10:02:30Z"), PauseReason::RateLimit));
    }

    // Rule: an ISO time after "resets" is the reset time, in UTC unless it says its offset.
    #[test]
    fn an_iso_reset_time_is_read() {
        for (line, resets_at) in [
            (
                "Workspace monthly budget of $200.00 exceeded; resets 2026-11-15T00:00:00.000Z.",
                "2026-11-15T00:00:00Z",
            ),
            (
                "usage limit reached, resets at 2026-11-15 03:00:00+02:00",
                "2026-11-15T01:00:00Z",
            ),
            (
                "usage limit reached, resets on 2026-11-15T03:00:00-0100",
                "2026-11-15T04:00:00Z",
            ),
            ("usage limit reached, resets 2026-11-15T03:00", "2026-11-15T02:00:00Z"),
            (
                "usage limit reached, resets 2026-11-15 03:00 UTC",
                "2026-11-15T03:00:00Z",
            ),
        ] {
            let (end, until) = reset_of(line);
            assert_eq!(end, quota(Some(resets_at)), "{line}");
            assert_eq!(
                until.as_utc(),
                at(resets_at).as_utc() + time::Duration::seconds(60),
                "{line}"
            );
        }
    }

    // Rule: a date with a month name, as Codex writes it, is read in the machine's local time.
    #[test]
    fn a_date_with_a_month_name_is_read_in_local_time() {
        for (line, resets_at) in [
            (
                "You've hit your usage limit. Try again at Nov 12th, 2026 3:04 PM.",
                "2026-11-12T14:04:00Z",
            ),
            (
                "usage limit reached, try again at December 1st, 2026 12:00 am",
                "2026-11-30T23:00:00Z",
            ),
            (
                "usage limit reached, try again at Jan 3rd, 2027 9:30 AM.",
                "2027-01-03T08:30:00Z",
            ),
            (
                "usage limit reached, try again at Nov 13, 2026 12:15 PM",
                "2026-11-13T11:15:00Z",
            ),
        ] {
            let (end, _) = reset_of(line);
            assert_eq!(end, quota(Some(resets_at)), "{line}");
        }
    }

    // Rule: a time of day is read in the machine's local time: the next time it is that time.
    #[test]
    fn a_time_of_day_is_the_next_one_in_local_time() {
        for (line, resets_at) in [
            ("5-hour limit reached · resets 3pm", "2026-11-12T14:00:00Z"),
            (
                "5-hour limit reached · resets 3pm (Europe/Warsaw)",
                "2026-11-12T14:00:00Z",
            ),
            ("You've hit your limit · resets 3:30 PM", "2026-11-12T14:30:00Z"),
            ("usage limit reached, will reset at 15:30", "2026-11-12T14:30:00Z"),
            ("usage limit reached, resets 9am", "2026-11-13T08:00:00Z"),
            ("usage limit reached, resets 12am", "2026-11-12T23:00:00Z"),
            ("usage limit reached, resets 11:59 pm", "2026-11-12T22:59:00Z"),
            ("usage limit reached, resets 3pm (UTC)", "2026-11-12T15:00:00Z"),
        ] {
            let (end, _) = reset_of(line);
            assert_eq!(end, quota(Some(resets_at)), "{line}");
        }
    }

    // Rule: text that is not a reset time gives the default pause, not an error.
    #[test]
    fn text_that_is_no_reset_time_gives_the_default_pause() {
        for line in [
            "usage limit reached, resets soon",
            "usage limit reached, resets 3 times a day",
            "usage limit reached, try again later",
            "usage limit reached, resets 25pm",
            "usage limit reached, resets 2026-13-45T99:99:99Z",
            "usage limit reached, retry in 5 parsecs",
            "usage limit reached, try again at Smarch 3rd, 2026 3:04 PM",
            "usage limit reached, resets 2026-02-30T10:00:00Z",
            "usage limit reached|12345",
        ] {
            assert_eq!(reset_of(line), (quota(None), at("2026-11-12T10:31:00Z")), "{line}");
        }
    }

    // Rule: the pause is never longer than MAX_PAUSE, however far the CLI puts the reset; the end keeps the time given.
    #[test]
    fn a_far_reset_time_is_cut_to_the_longest_pause() {
        assert_eq!(
            reset_of("usage limit reached; resets 2027-01-01T00:00:00Z"),
            (quota(Some("2027-01-01T00:00:00Z")), at("2026-11-20T10:01:00Z"))
        );
        assert_eq!(
            reset_of("usage limit reached, try again in 400 days"),
            (quota(Some("2027-12-17T10:00:00Z")), at("2026-11-20T10:01:00Z"))
        );
    }

    // Rule: the default pauses come from the configuration's [limits].
    #[test]
    fn the_default_pauses_come_from_the_configuration() {
        let config =
            Config::from_toml_str("[limits]\nquota_pause_minutes = 60\nrate_limit_pause_minutes = 2\n").unwrap();
        let run = |stderr| {
            classify_with(
                &config,
                None,
                stderr,
                ExitStatus::Code(1),
                StreamOutcome::Failed,
                StoppedBy::Nobody,
            )
        };
        assert_eq!(paused(&run("usage limit reached")).0, at("2026-11-12T11:01:00Z"));
        assert_eq!(paused(&run("429 too many requests")).0, at("2026-11-12T10:03:00Z"));
    }

    // Rule: the limit lines are those of the error tail the adapter kept: a line before it is not read.
    #[test]
    fn only_the_error_tail_is_read() {
        let config = Config::from_toml_str("[limits]\ntail_lines = 2\n").unwrap();
        let stderr = "429 too many requests\nretrying\nstill going\n";
        let class = classify_with(
            &config,
            None,
            stderr,
            ExitStatus::Code(1),
            StreamOutcome::Failed,
            StoppedBy::Nobody,
        );
        assert_eq!(
            class,
            Classified {
                end: RunEnd::NoStatus { exit_code: Some(1) },
                pause: None
            }
        );
    }

    // Rule: the evidence is the first line that showed the limit, trimmed and cut after 200 characters.
    #[test]
    fn the_evidence_is_the_first_limit_line_cut_to_size() {
        let class = failed(None, "  noise\n\t 429 first limit line  \n429 second\n");
        assert_eq!(class.pause.map(|p| p.evidence), Some("429 first limit line".to_owned()));
        let long = format!("usage limit reached {}", "ż".repeat(300));
        let evidence = failed(None, &long).pause.unwrap().evidence;
        assert_eq!(evidence.chars().count(), MAX_EVIDENCE_CHARS);
        assert!(long.starts_with(&evidence));
    }

    // Rule: a quota whose reset the CLI gives and a rate limit that has none show the same reasons as their texts.
    #[test]
    fn the_pause_reasons_have_their_texts() {
        assert_eq!(PauseReason::Quota.to_string(), "quota");
        assert_eq!(PauseReason::RateLimit.to_string(), "rate_limit");
    }

    // Rule: a pause that would end after the last year a timestamp holds is an error, not a wrapped time.
    #[test]
    fn a_pause_beyond_the_last_year_is_an_error() {
        let config = default_config();
        let output = RunOutput {
            final_message: None,
            session_id: None,
            stream: StreamOutcome::Failed,
            error_tail: ErrorTail::from_text("usage limit reached", NonZeroU32::MIN),
        };
        let now = at("9999-12-31T23:59:00Z");
        let context = LimitContext {
            limits: config.limits(),
            now,
            local_offset: local(),
        };
        let result = classify_outcome(
            &output,
            RunFinish {
                exit: ExitStatus::Code(1),
                stopped_by: StoppedBy::Nobody,
            },
            &context,
        );
        assert_eq!(result, Err(ClassifyError::PauseOverflow { now }));
    }

    /// The watchdog of the default configuration: three strikes.
    fn watchdog() -> Watchdog {
        default_config().watchdog().clone()
    }

    fn session() -> SessionId {
        "ses_1".parse().unwrap()
    }

    // Rule: a hung run is resumed until the strikes are used.
    #[test]
    fn a_hung_run_is_resumed_until_the_strikes_are_used() {
        let session = session();
        assert_eq!(after_hang(1, Some(&session), &watchdog()), AfterHang::Resume);
        assert_eq!(after_hang(2, Some(&session), &watchdog()), AfterHang::Resume);
        assert_eq!(after_hang(3, Some(&session), &watchdog()), AfterHang::MarkHung);
        assert_eq!(after_hang(4, Some(&session), &watchdog()), AfterHang::MarkHung);
    }

    // Rule: a run with no session cannot be resumed: its hang marks the task hung at once.
    #[test]
    fn a_hung_run_without_a_session_is_marked_hung() {
        assert_eq!(after_hang(1, None, &watchdog()), AfterHang::MarkHung);
    }

    // Rule: the strikes come from the configuration's [watchdog].
    #[test]
    fn the_strikes_come_from_the_watchdog_configuration() {
        let config = Config::from_toml_str("[watchdog]\nhang_strikes = 1\n").unwrap();
        let session = session();
        assert_eq!(after_hang(1, Some(&session), config.watchdog()), AfterHang::MarkHung);
    }
}
