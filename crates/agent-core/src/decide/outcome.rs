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
//! 3. The run failed — a failed exit status, or a stream that did not end with a successful result — and its error
//!    tail shows a provider limit: [`RunEnd::QuotaExhausted`] for an exhausted quota, usage limit or credits,
//!    [`RunEnd::RateLimited`] for a rate limit (HTTP 429) or an overloaded model (HTTP 503 and 529: a pause as short
//!    as a rate limit's). A line that is no provider's — the machine ran out of processes, memory or disk, a Unix
//!    socket failed — is never a limit. The final message is read as an error — its last `tail_lines` lines, like the
//!    error tail — only when it has no `STATUS:` line: the agent's own report is never taken for a provider error, and a
//!    clean run is never paused for a stale 429 that the CLI retried.
//! 4. The watchdog ended the run, which showed no limit: [`RunEnd::Hung`]. A run the watchdog killed while the provider
//!    refused it is a limit, paused until the reset, not a hang.
//! 5. A recognised `STATUS:` line of a run that failed without a limit: [`RunEnd::Reported`].
//! 6. Anything else: [`RunEnd::NoStatus`] with the exit code.
//!
//! The pause of a limit lasts until the time the CLI gives — a Unix time after `limit reached|`, a duration after
//! `resets in`, `try again in`, `retry after`, an ISO time, a date such as `Nov 12th, 2026 3:04 PM` or a time of day
//! such as `3pm` in the machine's local time — plus [`RESET_MARGIN`]; without one, or with one in the past, until
//! `[limits]` `quota_pause_minutes` or `rate_limit_pause_minutes` have passed, plus the margin. It is never longer than
//! [`MAX_PAUSE`]. Hangs and everything else pause nothing; [`after_hang`] decides whether a hung run is resumed.

use time::error::ConversionRange;
use time::{Date, Duration, Month, OffsetDateTime, PrimitiveDateTime, Time, UtcDateTime, UtcOffset};

use crate::cli::{ErrorTail, RunOutput, SessionId, StreamOutcome};
use crate::config::{Limits, Watchdog};
use crate::state::{InvalidTimestamp, RunEnd, Timestamp};
use crate::status::RunOutcome;

/// Seconds added to every pause, so that a run does not start a moment before the limit really ends.
pub const RESET_MARGIN: Duration = Duration::minutes(1);

/// The longest pause a limit causes, whatever reset time the CLI gives: a bound on untrusted text. Eight days cover a
/// weekly limit.
pub const MAX_PAUSE: Duration = Duration::days(8);

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
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ClassifyError {
    /// The pause would end after the last time that can be represented, year 9999.
    #[error("a pause from {now} would end after the year 9999")]
    PauseOverflow {
        /// When the run ended.
        now: Timestamp,
    },
    /// The time the pause would end at is not one a timestamp can hold.
    #[error("the pause from {now} would end at a time that cannot be recorded")]
    PauseOutOfRange {
        /// When the run ended.
        now: Timestamp,
        /// What is wrong with the time.
        #[source]
        source: InvalidTimestamp,
    },
    /// The configured default pause is longer than a duration can be.
    #[error("the configured default pause is too long")]
    DefaultPauseTooLong {
        /// What the conversion refused.
        #[source]
        source: ConversionRange,
    },
}

/// The words, in lower case, that show an exhausted quota, usage limit or credits.
const QUOTA_PHRASES: [&str; 14] = [
    "quota reached",
    "quota exceeded",
    "quota exhausted",
    "usage limit",
    "resource exhausted",
    "resource_exhausted",
    "resource has been exhausted",
    "out of credits",
    "insufficient credits",
    "insufficient balance",
    "exhausted your capacity",
    "exhausted your quota",
    "spend limit",
    "spending limit",
];

/// The words, in lower case, that show a provider limiting the rate of requests.
const RATE_PHRASES: [&str; 5] = [
    "rate limit",
    "rate_limit",
    "rate-limit",
    "ratelimit",
    "too many requests",
];

/// The words, in lower case, that show a model overloaded at the provider.
const OVERLOAD_PHRASES: [&str; 3] = ["at capacity", "overloaded", "temporarily unavailable"];

/// The words, in lower case, that show a failure of the machine itself: out of processes, memory, descriptors or disk.
/// Such a line is never a provider's limit, whatever else it says.
const LOCAL_ERROR_PHRASES: [&str; 5] = [
    "resource temporarily unavailable",
    "fork: ",
    "cannot allocate memory",
    "too many open files",
    "no space left on device",
];

/// The HTTP status of a payment problem, which providers use for exhausted credits.
const STATUS_PAYMENT_REQUIRED: &str = "402";

/// The HTTP status of a rate limit.
const STATUS_TOO_MANY_REQUESTS: &str = "429";

/// The HTTP statuses of an overloaded service: unavailable, and Anthropic's overloaded.
const STATUSES_OVERLOADED: [&str; 2] = ["503", "529"];

/// The words, in lower case, after which a CLI tells when a limit ends. `retrydelay` is Gemini's JSON field.
const RESET_TRIGGERS: [&str; 5] = ["retrydelay", "reset", "retry", "try again", "available again"];

/// Words that may stand between a trigger and the time it introduces: `resets at`, `try again in`, `retry after`.
const FILLER_WORDS: [&str; 6] = ["it", "in", "after", "at", "on", "about"];

/// The length of a Unix time in seconds that follows `limit reached|`: ten digits are the years 2001 to 2286.
const UNIX_TIME_DIGITS: usize = 10;

/// What `limit reached|` is followed by in the final message of an old Claude Code: the Unix time of the reset.
const UNIX_RESET_MARKER: &str = "limit reached|";

/// Digits of a year in a date such as `Nov 12th, 2026`.
const YEAR_DIGITS: usize = 4;

/// Seconds in one day.
const SECS_PER_DAY: i64 = 86_400;

/// Seconds in one hour.
const SECS_PER_HOUR: i64 = 3_600;

/// Seconds in one minute.
const SECS_PER_MINUTE: i64 = 60;

/// Hours of a day, which a time of day with am or pm counts to twelve.
const HOURS_PER_MERIDIEM: u8 = 12;

/// The months by name, for a date such as `Nov 12th, 2026`: a text of three letters or more that starts a name is it.
const MONTHS: [(&str, Month); 12] = [
    ("january", Month::January),
    ("february", Month::February),
    ("march", Month::March),
    ("april", Month::April),
    ("may", Month::May),
    ("june", Month::June),
    ("july", Month::July),
    ("august", Month::August),
    ("september", Month::September),
    ("october", Month::October),
    ("november", Month::November),
    ("december", Month::December),
];

/// The status line of a final message: the first line that starts with `STATUS:` once the markup characters `*` and
/// `` ` `` are dropped, followed by a word of capital letters and `_`.
#[must_use]
pub fn parse_status_line(message: &str) -> StatusLine {
    message
        .lines()
        .find_map(|line| {
            let plain: String = line.chars().filter(|c| !matches!(c, '*' | '`')).collect();
            let word: String = plain
                .trim_start()
                .strip_prefix("STATUS:")?
                .trim_start()
                .chars()
                .take_while(|c| c.is_ascii_uppercase() || *c == '_')
                .collect();
            (!word.is_empty()).then_some(word)
        })
        .map_or(StatusLine::Absent, |word| {
            word.parse::<RunOutcome>()
                .map_or_else(|unknown| StatusLine::Unknown(unknown.value), StatusLine::Reported)
        })
}

/// Classifies a finished run.
///
/// # Errors
/// [`ClassifyError`] when the pause would end beyond the years a timestamp can hold, or the configured default pause
/// cannot be a duration.
pub fn classify_outcome(
    output: &RunOutput,
    finish: RunFinish,
    context: &LimitContext<'_>,
) -> Result<Classified, ClassifyError> {
    let done = |end| Ok(Classified { end, pause: None });
    if finish.stopped_by == StoppedBy::Human {
        return done(RunEnd::Stopped);
    }
    let status = output
        .final_message
        .as_deref()
        .map_or(StatusLine::Absent, parse_status_line);
    let reported = match &status {
        StatusLine::Reported(outcome) => Some(*outcome),
        StatusLine::Absent | StatusLine::Unknown(_) => None,
    };
    let succeeded = output.stream == StreamOutcome::Succeeded;
    if let Some(outcome) = reported
        && (succeeded || finish.stopped_by == StoppedBy::Watchdog)
    {
        return done(RunEnd::Reported { outcome });
    }
    if !succeeded || finish.exit != ExitStatus::Code(0) {
        let message = match (status, &output.final_message) {
            (StatusLine::Absent, Some(message)) => ErrorTail::from_text(message, context.limits.tail_lines),
            (StatusLine::Absent, None) | (StatusLine::Reported(_) | StatusLine::Unknown(_), _) => ErrorTail::default(),
        };
        if let Some(limit) = find_limit(output.error_tail.lines().iter().chain(message.lines())) {
            return limit.classify(context);
        }
    }
    // After the limits: a run the watchdog killed while the provider refused it is paused until the reset, not resumed
    // straight into the limit.
    if finish.stopped_by == StoppedBy::Watchdog {
        return done(RunEnd::Hung);
    }
    match reported {
        Some(outcome) => done(RunEnd::Reported { outcome }),
        None => done(RunEnd::NoStatus {
            exit_code: match finish.exit {
                ExitStatus::Code(code) => Some(code),
                ExitStatus::Signal => None,
            },
        }),
    }
}

/// A provider limit found in the output of a run.
struct Limit {
    /// Which limit it is.
    reason: PauseReason,
    /// The first line that showed it, trimmed and cut.
    evidence: String,
    /// The lines of that kind, in lower case: where the reset time is looked for.
    lines: Vec<String>,
}

/// The provider limit that `lines` — the error tail and the tail of the final message of a failed run — show, if any: an
/// exhausted quota wins over a rate limit, as in the old harness.
fn find_limit<'a>(lines: impl Iterator<Item = &'a String>) -> Option<Limit> {
    let shown: Vec<(&str, String, PauseReason)> = lines
        .map(|line| line.trim())
        .filter(|line| !line.is_empty())
        .filter_map(|line| {
            let lower = line.to_lowercase();
            if is_local_error(&lower) {
                return None;
            }
            line_reason(&lower).map(|reason| (line, lower, reason))
        })
        .collect();
    let reason = if shown.iter().any(|(_, _, reason)| *reason == PauseReason::Quota) {
        PauseReason::Quota
    } else {
        PauseReason::RateLimit
    };
    let evidence = shown
        .iter()
        .find(|(_, _, shown)| *shown == reason)
        .map(|(line, _, _)| line.chars().take(MAX_EVIDENCE_CHARS).collect::<String>())?;
    let lines = shown
        .into_iter()
        .filter(|(_, _, shown)| *shown == reason)
        .map(|(_, lower, _)| lower)
        .collect();
    Some(Limit {
        reason,
        evidence,
        lines,
    })
}

/// What a line, in lower case, says about the provider's limits: a quota first, then a rate limit or an overload.
fn line_reason(lower: &str) -> Option<PauseReason> {
    let rate_words = RATE_PHRASES.iter().any(|phrase| lower.contains(phrase));
    let quota = QUOTA_PHRASES.iter().any(|phrase| lower.contains(phrase))
        || has_number(lower, STATUS_PAYMENT_REQUIRED)
        || is_budget_spent(lower)
        || ((lower.contains("limit reached") || lower.contains("hit your")) && lower.contains("limit"))
            && !rate_words
            && !lower.contains("context");
    if quota {
        return Some(PauseReason::Quota);
    }
    let rate = rate_words
        || has_number(lower, STATUS_TOO_MANY_REQUESTS)
        || OVERLOAD_PHRASES.iter().any(|phrase| lower.contains(phrase))
        || STATUSES_OVERLOADED.iter().any(|status| has_number(lower, status));
    rate.then_some(PauseReason::RateLimit)
}

/// Whether `lower` says a budget is used up: `budget exceeded`, `budget of $200.00 is exhausted`, `budget reached`.
fn is_budget_spent(lower: &str) -> bool {
    let mut rest = lower;
    while let Some((_, after)) = rest.split_once("budget") {
        let mut words = after
            .split_whitespace()
            .map(|word| word.trim_matches(|c: char| !c.is_alphanumeric()));
        let mut word = words.next();
        if word == Some("of") {
            words.next();
            word = words.next();
        }
        if word == Some("is") {
            word = words.next();
        }
        if matches!(word, Some("exceeded" | "exhausted" | "reached")) {
            return true;
        }
        rest = after;
    }
    false
}

/// Whether `lower` is a failure of the machine, not of the provider.
fn is_local_error(lower: &str) -> bool {
    LOCAL_ERROR_PHRASES.iter().any(|phrase| lower.contains(phrase))
        || lower
            .split("unix")
            .skip(1)
            .any(|after| after.strip_prefix(':').unwrap_or(after).starts_with([' ', '/', '@']))
        || lower.split(".sock").skip(1).any(|after| after.starts_with([':', ' ']))
}

/// Whether `text` has `number` as a whole number: not a part of a longer one.
fn has_number(text: &str, number: &str) -> bool {
    text.match_indices(number).any(|(at, _)| {
        let before = text.get(..at).and_then(|head| head.chars().next_back());
        let after = text
            .get(at.saturating_add(number.len())..)
            .and_then(|tail| tail.chars().next());
        !before.is_some_and(|c| c.is_ascii_digit()) && !after.is_some_and(|c| c.is_ascii_digit())
    })
}

impl Limit {
    /// The end and the pause of a run that stopped on this limit.
    fn classify(self, context: &LimitContext<'_>) -> Result<Classified, ClassifyError> {
        let now = context.now.as_utc();
        let reset = reset_time(&self.lines, now, context.local_offset);
        let wait = match reset {
            Some(reset) => (reset - now).min(MAX_PAUSE),
            None => self.default_pause(context.limits)?,
        };
        let until = now
            .checked_add(wait)
            .and_then(|end| end.checked_add(RESET_MARGIN))
            .ok_or(ClassifyError::PauseOverflow { now: context.now })
            .and_then(|end| recorded(end, context.now))?;
        let end = match self.reason {
            PauseReason::Quota => RunEnd::QuotaExhausted {
                resets_at: reset.map(|reset| recorded(reset, context.now)).transpose()?,
            },
            PauseReason::RateLimit => RunEnd::RateLimited,
        };
        Ok(Classified {
            end,
            pause: Some(Pause {
                until,
                reason: self.reason,
                evidence: self.evidence,
            }),
        })
    }

    /// How long this limit pauses a task when the CLI does not say when it ends: the configured default.
    fn default_pause(&self, limits: &Limits) -> Result<Duration, ClassifyError> {
        let minutes = match self.reason {
            PauseReason::Quota => limits.quota_pause_minutes,
            PauseReason::RateLimit => limits.rate_limit_pause_minutes,
        };
        Duration::try_from(minutes.as_duration()).map_err(|source| ClassifyError::DefaultPauseTooLong { source })
    }
}

/// `time` as a [`Timestamp`], for the run that ended at `now`.
fn recorded(time: UtcDateTime, now: Timestamp) -> Result<Timestamp, ClassifyError> {
    Timestamp::from_utc(time).map_err(|source| ClassifyError::PauseOutOfRange { now, source })
}

/// A reader of one form in which a CLI writes the time its limit ends: the line, in lower case, the time now and the
/// machine's offset in; the time out, if the line has that form. A text that is no time is not an error: the limit then
/// has the default pause, which is specified behaviour and has its test.
type ResetReader = fn(&str, UtcDateTime, UtcOffset) -> Option<UtcDateTime>;

/// The first reset time after `now` that the `lines` give, by the order of the forms: a Unix time, a duration, an ISO
/// time, a date, a time of day.
fn reset_time(lines: &[String], now: UtcDateTime, offset: UtcOffset) -> Option<UtcDateTime> {
    const READERS: [ResetReader; 5] = [
        read_unix_time,
        read_duration,
        read_iso_time,
        read_date,
        read_time_of_day,
    ];
    READERS.iter().find_map(|read| {
        lines
            .iter()
            .find_map(|line| read(line, now, offset).filter(|reset| *reset > now))
    })
}

/// `limit reached|1794478200`.
fn read_unix_time(line: &str, _now: UtcDateTime, _offset: UtcOffset) -> Option<UtcDateTime> {
    let (_, after) = line.split_once(UNIX_RESET_MARKER)?;
    let digits: String = after.chars().take_while(char::is_ascii_digit).collect();
    if digits.len() != UNIX_TIME_DIGITS {
        return None;
    }
    UtcDateTime::from_unix_timestamp(digits.parse().ok()?).ok()
}

/// What follows each of the [`RESET_TRIGGERS`] in `line`, without the words and signs that only join it to the time.
fn after_triggers(line: &str) -> Vec<&str> {
    let mut hits: Vec<(usize, usize)> = RESET_TRIGGERS
        .iter()
        .flat_map(|trigger| line.match_indices(trigger).map(|(at, found)| (at, found.len())))
        .collect();
    hits.sort_by_key(|&(at, len)| (at, std::cmp::Reverse(len)));
    let mut covered = 0;
    let mut rests = Vec::new();
    for (at, len) in hits {
        if at < covered {
            continue;
        }
        covered = at.saturating_add(len);
        rests.extend(line.get(covered..).map(skip_joins));
    }
    rests
}

/// `text` without the joining words and signs at its start: `s` of `resets`, ` in `, ` after `, `": "`.
fn skip_joins(text: &str) -> &str {
    let mut rest = text
        .strip_prefix('s')
        .filter(|after| !after.starts_with(char::is_alphanumeric))
        .unwrap_or(text);
    loop {
        rest = rest.trim_start_matches(|c: char| c.is_whitespace() || matches!(c, '"' | '\'' | ':' | '=' | ','));
        let word = FILLER_WORDS.iter().find_map(|word| {
            rest.strip_prefix(word)
                .filter(|after| after.starts_with(char::is_whitespace))
        });
        match word {
            Some(after) => rest = after,
            None => return rest,
        }
    }
}

/// `resets in 24m46s`, `try again in 2 hours 13 minutes`, `retry after 90s`, `retryDelay: "36s"`.
fn read_duration(line: &str, now: UtcDateTime, _offset: UtcOffset) -> Option<UtcDateTime> {
    after_triggers(line).into_iter().find_map(|rest| {
        let seconds = duration_seconds(rest)?;
        now.checked_add(Duration::seconds(seconds))
    })
}

/// The seconds of the duration at the start of `text`: numbers each followed by a unit, joined by spaces, commas or
/// `and`; seconds may have a fraction, which rounds up. `None` for any other start, such as `3pm`.
fn duration_seconds(text: &str) -> Option<i64> {
    let mut rest = text;
    let mut total: i64 = 0;
    let mut parts = 0_u32;
    while let Some((seconds, after)) = duration_part(rest) {
        total = total.saturating_add(seconds);
        parts = parts.saturating_add(1);
        let after = after.trim_start_matches(|c: char| c.is_whitespace() || c == ',');
        rest = after.strip_prefix("and ").unwrap_or(after);
    }
    (parts > 0 && total > 0).then_some(total)
}

/// One number and its unit at the start of `text`, in seconds, and what follows.
fn duration_part(text: &str) -> Option<(i64, &str)> {
    let (whole, rest) = take_digits(text)?;
    let (fraction, rest) = match rest.strip_prefix('.') {
        Some(after) => take_digits(after).map_or((None, rest), |(digits, after)| (Some(digits), after)),
        None => (None, rest),
    };
    let rest = rest.strip_prefix(' ').unwrap_or(rest);
    let unit_len = rest.find(|c: char| !c.is_ascii_alphabetic()).unwrap_or(rest.len());
    let (unit, rest) = rest.split_at_checked(unit_len)?;
    let per_unit = match unit {
        "d" | "day" | "days" => SECS_PER_DAY,
        "h" | "hr" | "hrs" | "hour" | "hours" => SECS_PER_HOUR,
        "m" | "min" | "mins" | "minute" | "minutes" => SECS_PER_MINUTE,
        "s" | "sec" | "secs" | "second" | "seconds" => 1,
        _ => return None,
    };
    let mut seconds = whole.parse::<i64>().ok()?.saturating_mul(per_unit);
    if let Some(fraction) = fraction {
        if per_unit != 1 {
            return None;
        }
        if fraction.bytes().any(|b| b != b'0') {
            seconds = seconds.saturating_add(1);
        }
    }
    Some((seconds, rest))
}

/// The ASCII digits at the start of `text` and what follows them; `None` without a digit.
fn take_digits(text: &str) -> Option<(&str, &str)> {
    let len = text.find(|c: char| !c.is_ascii_digit()).unwrap_or(text.len());
    let (digits, rest) = text.split_at_checked(len)?;
    (!digits.is_empty()).then_some((digits, rest))
}

/// Exactly `count` ASCII digits at the start of `text` as a number, and what follows them.
fn take_number(text: &str, count: usize) -> Option<(u32, &str)> {
    let (digits, rest) = text.split_at_checked(count)?;
    if digits.len() != count || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    Some((digits.parse().ok()?, rest))
}

/// `resets 2026-11-15T00:00:00.000Z`, `resets at 2026-11-15 03:00:00+02:00`: an ISO date and time; without an offset
/// the machine's.
fn read_iso_time(line: &str, _now: UtcDateTime, offset: UtcOffset) -> Option<UtcDateTime> {
    after_triggers(line).into_iter().find_map(|rest| {
        let (year, rest) = take_number(rest, 4)?;
        let (month, rest) = take_number(rest.strip_prefix('-')?, 2)?;
        let (day, rest) = take_number(rest.strip_prefix('-')?, 2)?;
        let rest = rest.strip_prefix('t').or_else(|| rest.strip_prefix(' '))?;
        let (hour, rest) = take_number(rest, 2)?;
        let (minute, rest) = take_number(rest.strip_prefix(':')?, 2)?;
        let (second, rest) = match rest.strip_prefix(':') {
            Some(after) => take_number(after, 2)?,
            None => (0, rest),
        };
        let rest = match rest.strip_prefix('.') {
            Some(after) => after.trim_start_matches(|c: char| c.is_ascii_digit()),
            None => rest,
        };
        let (zone, _) = read_zone(rest, offset)?;
        let date = Date::from_calendar_date(
            i32::try_from(year).ok()?,
            Month::try_from(u8::try_from(month).ok()?).ok()?,
            u8::try_from(day).ok()?,
        )
        .ok()?;
        let time = Time::from_hms(
            u8::try_from(hour).ok()?,
            u8::try_from(minute).ok()?,
            u8::try_from(second).ok()?,
        )
        .ok()?;
        Some(UtcDateTime::from(
            PrimitiveDateTime::new(date, time).assume_offset(zone),
        ))
    })
}

/// The zone at the start of `text`: `z`, `+02:00`, `-0100`, ` utc`; without one, the machine's `local` offset.
fn read_zone(text: &str, local: UtcOffset) -> Option<(UtcOffset, &str)> {
    if let Some(rest) = text
        .strip_prefix('z')
        .filter(|rest| !rest.starts_with(char::is_alphabetic))
    {
        return Some((UtcOffset::UTC, rest));
    }
    if let Some(rest) = text
        .trim_start()
        .strip_prefix("utc")
        .or_else(|| text.trim_start().strip_prefix("gmt"))
    {
        return Some((UtcOffset::UTC, rest));
    }
    let (sign, rest) = match (text.strip_prefix('+'), text.strip_prefix('-')) {
        (Some(rest), _) => (1_i8, rest),
        (None, Some(rest)) => (-1_i8, rest),
        (None, None) => return Some((local, text)),
    };
    let (hours, rest) = take_number(rest, 2)?;
    let (minutes, rest) = take_number(rest.strip_prefix(':').unwrap_or(rest), 2)?;
    let hours = i8::try_from(hours).ok()?.checked_mul(sign)?;
    let minutes = i8::try_from(minutes).ok()?.checked_mul(sign)?;
    Some((UtcOffset::from_hms(hours, minutes, 0).ok()?, rest))
}

/// `try again at Nov 12th, 2026 3:04 PM`: a date with a month name and a time of day, in the machine's local time.
fn read_date(line: &str, _now: UtcDateTime, offset: UtcOffset) -> Option<UtcDateTime> {
    after_triggers(line).into_iter().find_map(|rest| {
        let name_len = rest.find(|c: char| !c.is_ascii_alphabetic()).unwrap_or(rest.len());
        let (name, rest) = rest.split_at_checked(name_len)?;
        let month = MONTHS
            .iter()
            .find(|(full, _)| name.len() >= 3 && full.starts_with(name))
            .map(|&(_, month)| month)?;
        let (day, rest) = take_digits(rest.trim_start())?;
        let rest = ["st", "nd", "rd", "th"]
            .iter()
            .find_map(|suffix| rest.strip_prefix(suffix))
            .unwrap_or(rest);
        let (year, rest) = take_number(rest.trim_start_matches([',', ' ']), YEAR_DIGITS)?;
        let (clock, zone) = read_clock(rest.trim_start(), offset)?;
        let date = Date::from_calendar_date(i32::try_from(year).ok()?, month, day.parse().ok()?).ok()?;
        Some(UtcDateTime::from(
            PrimitiveDateTime::new(date, clock).assume_offset(zone),
        ))
    })
}

/// `resets 3pm`, `resets at 15:30`, `resets 3:30 pm (UTC)`: the next time it is that time, in the machine's local time.
fn read_time_of_day(line: &str, now: UtcDateTime, offset: UtcOffset) -> Option<UtcDateTime> {
    after_triggers(line).into_iter().find_map(|rest| {
        let (time, zone) = read_clock(rest, offset)?;
        let today = OffsetDateTime::from(now).to_offset(zone).date();
        let first = UtcDateTime::from(PrimitiveDateTime::new(today, time).assume_offset(zone));
        if first > now {
            Some(first)
        } else {
            first.checked_add(Duration::days(1))
        }
    })
}

/// A time of day at the start of `text` — `3pm`, `3:30 pm`, `15:30` — and its zone: ` utc` or `(utc)`, else the
/// `local` offset. A bare number is no time of day: it needs minutes or am or pm.
fn read_clock(text: &str, local: UtcOffset) -> Option<(Time, UtcOffset)> {
    let (hour, rest) = take_number_up_to(text, 2)?;
    let (minute, rest) = match rest.strip_prefix(':') {
        Some(after) => {
            let (minute, rest) = take_number(after, 2)?;
            (Some(minute), rest)
        }
        None => (None, rest),
    };
    let spaced = rest.trim_start();
    let whole_word = |after: &&str| !after.starts_with(char::is_alphabetic);
    let (meridiem, rest) = match (
        spaced.strip_prefix("am").filter(whole_word),
        spaced.strip_prefix("pm").filter(whole_word),
    ) {
        (Some(rest), _) => (Some(false), rest),
        (None, Some(rest)) => (Some(true), rest),
        (None, None) => (None, rest),
    };
    let hour = match meridiem {
        Some(afternoon) => {
            let hour = u8::try_from(hour).ok()?;
            if hour == 0 || hour > HOURS_PER_MERIDIEM {
                return None;
            }
            hour % HOURS_PER_MERIDIEM + if afternoon { HOURS_PER_MERIDIEM } else { 0 }
        }
        None if minute.is_some() => u8::try_from(hour).ok()?,
        None => return None,
    };
    let time = Time::from_hms(hour, u8::try_from(minute.unwrap_or(0)).ok()?, 0).ok()?;
    let rest = rest.trim_start();
    let rest = rest.strip_prefix('(').unwrap_or(rest);
    let zone = read_zone(rest, local).map_or(local, |(zone, _)| zone);
    Some((time, zone))
}

/// One or two ASCII digits at the start of `text` as a number, and what follows them.
fn take_number_up_to(text: &str, most: usize) -> Option<(u32, &str)> {
    let (digits, rest) = take_digits(text)?;
    if digits.len() > most {
        return None;
    }
    Some((digits.parse().ok()?, rest))
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
    if session.is_none() || hangs_in_a_row >= watchdog.hang_strikes.get() {
        AfterHang::MarkHung
    } else {
        AfterHang::Resume
    }
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

    // Rule: a limit shown by a run the watchdog killed is a limit: paused until the reset, as the old harness did.
    #[test]
    fn a_hung_run_that_shows_a_limit_is_paused() {
        let class = classify_with(
            &default_config(),
            None,
            "rate limit exceeded",
            ExitStatus::Signal,
            StreamOutcome::Incomplete,
            StoppedBy::Watchdog,
        );
        assert_eq!(class.end, RunEnd::RateLimited);
        assert_eq!(paused(&class), (at("2026-11-12T10:06:00Z"), PauseReason::RateLimit));
        let class = classify_with(
            &default_config(),
            None,
            "Claude AI usage limit reached|1794478200",
            ExitStatus::Signal,
            StreamOutcome::Incomplete,
            StoppedBy::Watchdog,
        );
        assert_eq!(class.end, quota(Some("2026-11-12T10:10:00Z")));
        assert_eq!(paused(&class), (at("2026-11-12T10:11:00Z"), PauseReason::Quota));
    }

    // Rule: a run the watchdog killed that shows no limit — not even a local error — is hung.
    #[test]
    fn a_hung_run_that_shows_no_limit_is_hung() {
        let class = classify_with(
            &default_config(),
            None,
            "dial unix /tmp/x.sock: connection refused",
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

    // Rule: a stream that succeeded but exited with a failed status is a failed run: its limit line counts.
    #[test]
    fn a_successful_stream_with_a_failed_exit_is_a_failed_run() {
        let run = |exit| {
            classify_with(
                &default_config(),
                None,
                "429 Too Many Requests",
                exit,
                StreamOutcome::Succeeded,
                StoppedBy::Nobody,
            )
        };
        assert_eq!(run(ExitStatus::Code(1)).end, RunEnd::RateLimited);
        assert_eq!(run(ExitStatus::Code(0)).end, RunEnd::NoStatus { exit_code: Some(0) });
    }

    // Rule: a status code is a whole number: 81429 or 4290 is no HTTP 429.
    #[test]
    fn a_number_that_contains_a_status_code_is_no_status_code() {
        for line in ["request 81429 failed", "error code 4290", "id=14291"] {
            assert_eq!(
                failed(None, line),
                Classified {
                    end: RunEnd::NoStatus { exit_code: Some(1) },
                    pause: None
                },
                "{line}"
            );
        }
        assert_eq!(failed(None, "HTTP 429").end, RunEnd::RateLimited);
        assert_eq!(failed(None, "status=429.").end, RunEnd::RateLimited);
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
            ("usage limit reached, resets 12pm", "2026-11-12T11:00:00Z"),
            // An offset after the time of day is the zone of the time, not the machine's.
            ("usage limit reached, resets 3pm +02:00", "2026-11-12T13:00:00Z"),
            ("usage limit reached, resets 3:30pm (+02:00)", "2026-11-12T13:30:00Z"),
            ("usage limit reached, resets 15:30 -05:00", "2026-11-12T20:30:00Z"),
            ("usage limit reached, resets 3pm Z", "2026-11-12T15:00:00Z"),
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
            "usage limit reached, resets 13pm",
            "usage limit reached, resets 0am",
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

    // Rule: am and pm are words of their own: the start of another word is not a time of day.
    #[test]
    fn am_and_pm_must_be_whole_words() {
        for line in [
            "usage limit reached, resets 5 amazing hours from now",
            "usage limit reached, resets 5 pmx",
            "usage limit reached, resets 5pmish",
        ] {
            assert_eq!(reset_of(line), (quota(None), at("2026-11-12T10:31:00Z")), "{line}");
        }
    }

    // Rule: the year of a date has four digits, so a time written after a day is not a year.
    #[test]
    fn a_date_without_a_year_is_no_reset_time() {
        assert_eq!(
            reset_of("usage limit reached, try again at Nov 13, 3:04 PM"),
            (quota(None), at("2026-11-12T10:31:00Z"))
        );
    }

    // Rule: of a long final message without a status only its last lines are read, as many as the error tail keeps.
    #[test]
    fn only_the_last_lines_of_the_final_message_are_read() {
        let filler = "working on it\n".repeat(40);
        let early = format!("usage limit reached\n{filler}done");
        assert_eq!(
            failed(Some(&early), ""),
            Classified {
                end: RunEnd::NoStatus { exit_code: Some(1) },
                pause: None
            }
        );
        let late = format!("{filler}usage limit reached");
        assert_eq!(failed(Some(&late), "").end, quota(None));
    }

    // Rule: a Unix time has ten digits: a longer number is no reset time, whatever date it would be.
    #[test]
    fn a_unix_time_has_ten_digits() {
        for line in ["usage limit reached|17944782000", "usage limit reached|179447820"] {
            assert_eq!(reset_of(line), (quota(None), at("2026-11-12T10:31:00Z")), "{line}");
        }
    }

    // Rule: a month is named by at least three letters of its name.
    #[test]
    fn a_month_needs_three_letters() {
        assert_eq!(
            reset_of("usage limit reached, try again at Ma 3rd, 2027 9:00 AM"),
            (quota(None), at("2026-11-12T10:31:00Z"))
        );
        assert_eq!(
            reset_of("usage limit reached, try again at Mar 3rd, 2027 9:00 AM").0,
            quota(Some("2027-03-03T08:00:00Z"))
        );
    }
}
