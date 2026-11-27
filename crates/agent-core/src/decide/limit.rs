//! What a provider's refusal of a run does to the agent that met it: the pause and a retry after it, or a change of
//! agent. A refusal is a rate limit, an overload or an exhausted quota; the retries after the pause are counted in a
//! row, and a run that ends in any other way clears the count.
//!
//! [`decide`] is the whole rule and touches nothing: the daemon feeds it what the run showed and carries out the answer.

use time::Duration;

use crate::state::RunEnd;

#[cfg(test)]
mod tests;

/// What the provider's refusal showed about its limit.
///
/// Written to the state as `{"kind": "rate_limit"}` or `{"kind": "quota", "reset_in_nanoseconds": N}` (the key is
/// absent without a reset; `N` is positive).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Limit {
    /// A rate limit or an overload: the provider names no reset.
    RateLimit,
    /// An exhausted quota. `reset_in` is how far away its named reset is; `None` when the CLI names none.
    Quota {
        /// The time until the reset the provider named, strictly positive when present.
        reset_in: Option<Duration>,
    },
}

/// The kind of a [`Limit`] as it is written.
#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
enum LimitKind {
    /// A [`Limit::RateLimit`].
    RateLimit,
    /// A [`Limit::Quota`].
    Quota,
}

/// How a [`Limit`] is written: its kind and, for a quota with a named reset, how far away the reset was in nanoseconds.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct LimitWire {
    /// Which limit.
    kind: LimitKind,
    /// How far away the named reset was; absent without one. A rate limit has none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    reset_in_nanoseconds: Option<i128>,
}

/// Why a written limit is no [`Limit`].
#[derive(Debug, thiserror::Error)]
enum LimitWireError {
    /// A rate limit names no reset.
    #[error("a rate limit has no reset")]
    RateLimitWithReset,
    /// The reset of a quota is not strictly positive.
    #[error("the reset of a quota must be positive, not {nanoseconds} nanoseconds")]
    NotPositive {
        /// What was written.
        nanoseconds: i128,
    },
    /// The reset is further away than a duration can be.
    #[error("the reset of a quota, {nanoseconds} nanoseconds, is longer than a duration can be")]
    TooLong {
        /// What was written.
        nanoseconds: i128,
    },
}

/// Nanoseconds in a second.
const NANOSECONDS_PER_SECOND: i128 = 1_000_000_000;

/// The duration of a strictly positive number of `nanoseconds`.
fn positive_duration(nanoseconds: i128) -> Result<Duration, LimitWireError> {
    if nanoseconds <= 0 {
        return Err(LimitWireError::NotPositive { nanoseconds });
    }
    let too_long = |_| LimitWireError::TooLong { nanoseconds };
    let seconds = i64::try_from(nanoseconds.div_euclid(NANOSECONDS_PER_SECOND)).map_err(too_long)?;
    let rest = i32::try_from(nanoseconds.rem_euclid(NANOSECONDS_PER_SECOND)).map_err(too_long)?;
    Ok(Duration::new(seconds, rest))
}

impl From<Limit> for LimitWire {
    fn from(limit: Limit) -> Self {
        match limit {
            Limit::RateLimit => Self {
                kind: LimitKind::RateLimit,
                reset_in_nanoseconds: None,
            },
            Limit::Quota { reset_in } => Self {
                kind: LimitKind::Quota,
                reset_in_nanoseconds: reset_in.map(Duration::whole_nanoseconds),
            },
        }
    }
}

impl TryFrom<LimitWire> for Limit {
    type Error = LimitWireError;

    fn try_from(wire: LimitWire) -> Result<Self, Self::Error> {
        match (wire.kind, wire.reset_in_nanoseconds) {
            (LimitKind::RateLimit, None) => Ok(Self::RateLimit),
            (LimitKind::RateLimit, Some(_)) => Err(LimitWireError::RateLimitWithReset),
            (LimitKind::Quota, None) => Ok(Self::Quota { reset_in: None }),
            (LimitKind::Quota, Some(nanoseconds)) => Ok(Self::Quota {
                reset_in: Some(positive_duration(nanoseconds)?),
            }),
        }
    }
}

impl serde::Serialize for Limit {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        LimitWire::from(*self).serialize(serializer)
    }
}

impl<'de> serde::Deserialize<'de> for Limit {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let wire = LimitWire::deserialize(deserializer)?;
        Self::try_from(wire).map_err(serde::de::Error::custom)
    }
}

/// The facts of one refused run, as the daemon has them after the run ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Refusal {
    /// What the refusal showed.
    pub limit: Limit,
    /// How many refusals in a row the task has had, this one included; `1` for the first.
    pub refusals: u32,
    /// `[replacement] limit_retries`: how many retries after the pause the task gets.
    pub retries: u32,
    /// `[replacement] limit_wait_minutes` as a duration: the longest wait for a named reset; zero never waits for one.
    pub wait_limit: Duration,
}

/// What the rule says to do with a refused task.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    /// The task waits for the pause and is tried again in the same session.
    Pause,
    /// The task changes to an agent of another provider: the next refusal is not waited out.
    ChangeAgent,
}

/// The count of consecutive provider refusals after this run; other endings clear it.
#[must_use]
pub const fn next_refusals(previous: u32, end: &RunEnd) -> u32 {
    match end {
        RunEnd::RateLimited | RunEnd::QuotaExhausted { .. } => previous.saturating_add(1),
        RunEnd::Reported { .. } | RunEnd::NoStatus { .. } | RunEnd::Hung | RunEnd::TurnLimit | RunEnd::Stopped => 0,
    }
}

/// What the rule says about a refused task: the change of agent when its retries are used up, or when the provider's
/// named reset is further away than the configured wait; the pause otherwise.
#[must_use]
pub fn decide(refusal: &Refusal) -> Decision {
    if let Limit::Quota {
        reset_in: Some(reset_in),
    } = refusal.limit
        && reset_in > refusal.wait_limit
    {
        return Decision::ChangeAgent;
    }
    if refusal.refusals > refusal.retries {
        Decision::ChangeAgent
    } else {
        Decision::Pause
    }
}
