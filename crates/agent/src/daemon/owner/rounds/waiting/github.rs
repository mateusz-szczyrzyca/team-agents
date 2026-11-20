//! A step of the owner that calls GitHub or git and failed: which round it holds, what it was doing, what GitHub
//! answered, how often it has failed and when it is tried again.
//!
//! The waits live in the owner's memory beside the rest of what the autopilot remembers between two looks. A daemon
//! that starts has none, so every step of every round is tried at once at its first look: a pause of the network that
//! outlasted the daemon costs no round more than its next start.

use std::collections::BTreeMap;
use std::num::NonZeroU32;

use agent_core::decide::retry::pause;
use agent_core::ids::{RoundId, TeamId};
use agent_core::state::{PrNumber, Timestamp};
use agent_runtime::github::Outlook;
use time::Duration;

/// How many steps may wait for GitHub at once; beyond it the oldest wait gives way, as a loop with no memory would.
pub(in crate::daemon::owner) const MAX_WAITS: usize = 256;

/// The time of day `time` as a reader of the local clock sees it, `HH:MM`.
#[must_use]
pub(in crate::daemon::owner) fn clock_time(time: Timestamp, offset: time::UtcOffset) -> String {
    let local = time.as_utc().to_offset(offset);
    format!("{:02}:{:02}", local.hour(), local.minute())
}

/// What a step of the owner was doing when it failed, for the status and the log.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(in crate::daemon::owner) enum Step {
    /// The step belongs to one round.
    Round {
        /// The round.
        round: RoundId,
    },
    /// The step belongs to every open round of a team, as the description of its pull requests does.
    Team {
        /// The team.
        team: TeamId,
    },
}

impl std::fmt::Display for Step {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Round { round } => write!(f, "{round}"),
            Self::Team { team } => write!(f, "the rounds of {team}"),
        }
    }
}

/// A step that failed and waits to be tried again.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::daemon::owner) struct Wait {
    /// The team whose rounds it holds.
    pub(super) team: TeamId,
    /// What the step does, in the words of `agent status`: `get pull request #138`.
    pub(super) operation: String,
    /// What GitHub or git answered, with its cause chain.
    pub(super) answer: String,
    /// Whether a later try may do better.
    pub(super) outlook: Outlook,
    /// How many times it has failed.
    pub(super) tries: NonZeroU32,
    /// When it is tried again; none for a refusal that waits for the human's `agent resume`.
    pub(super) next_at: Option<Timestamp>,
}

impl Wait {
    /// The first failure of `operation`, at `now`, answered with `answer`; `pr` when the step is about a pull request.
    fn first(
        team: TeamId,
        operation: String,
        pr: Option<PrNumber>,
        answer: String,
        outlook: Outlook,
        now: Timestamp,
    ) -> Result<Self, DurationOverflow> {
        let tries = NonZeroU32::MIN;
        let next_at = match outlook {
            Outlook::MayPass => Some(later(now, pause(tries))?),
            Outlook::WillNotPass => None,
        };
        Ok(Self {
            team,
            operation: named(operation, pr),
            answer,
            outlook,
            tries,
            next_at,
        })
    }

    /// The next failure of the same step, at `now`.
    fn again(&mut self, answer: String, outlook: Outlook, now: Timestamp) -> Result<(), DurationOverflow> {
        self.tries = self.tries.saturating_add(1);
        self.answer = answer;
        self.outlook = outlook;
        self.next_at = match outlook {
            Outlook::MayPass => Some(later(now, pause(self.tries))?),
            Outlook::WillNotPass => None,
        };
        Ok(())
    }

    /// Whether it is to be tried now.
    fn due(&self, now: Timestamp) -> bool {
        match (self.outlook, self.next_at) {
            (Outlook::MayPass, Some(at)) => at <= now,
            (Outlook::WillNotPass, _) => false,
            (Outlook::MayPass, None) => true,
        }
    }

    /// What `agent status` says of it: `get pull request #138, tried 4 times, next try 01:07`, or for a refusal
    /// `get pull request #138, refused: ...`.
    pub(super) fn line(&self, offset: time::UtcOffset) -> String {
        let tried = self.tries.get();
        match self.next_at {
            Some(at) => format!(
                "{}, tried {tried} time{}, next try {}",
                self.operation,
                if tried == 1 { "" } else { "s" },
                clock_time(at, offset)
            ),
            None => format!("{}, refused: {}", self.operation, self.answer),
        }
    }
}

/// `operation #pr`, or `operation` for a step that is about no one pull request.
fn named(operation: String, pr: Option<PrNumber>) -> String {
    match pr {
        Some(number) => format!("{operation} #{number}"),
        None => operation,
    }
}

/// `now` plus `span`.
///
/// # Errors
/// [`DurationOverflow`] when the time cannot be recorded.
fn later(now: Timestamp, span: Duration) -> Result<Timestamp, DurationOverflow> {
    Timestamp::from_utc(now.as_utc().saturating_add(span)).map_err(|_| DurationOverflow)
}

/// The pause carries the time past the last year a [`Timestamp`] records.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("the time after the pause cannot be recorded")]
pub(in crate::daemon::owner) struct DurationOverflow;

/// The steps that failed and wait, by round and by operation.
#[derive(Debug, Default)]
pub(in crate::daemon::owner) struct Waits {
    /// The waits of each round, and of each team's rounds as a whole, by operation.
    pub(super) steps: BTreeMap<(Step, String), Wait>,
}

impl Waits {
    /// The wait of `step` for `operation`, if it has one.
    pub(super) fn get(&self, step: &Step, operation: &str) -> Option<&Wait> {
        self.steps.get(&(step.clone(), operation.to_owned()))
    }

    /// The wait of the round `round` for `operation`, if it has one.
    pub(super) fn for_round<'a>(&'a self, round: &RoundId, operation: &str) -> Option<&'a Wait> {
        self.steps
            .get(&(Step::Round { round: round.clone() }, operation.to_owned()))
    }

    /// Whether `step` is to be tried now: it has not failed, its pause is over, or it waits for the human.
    pub(super) fn due(&self, step: &Step, operation: &str, now: Timestamp) -> bool {
        match self.steps.get(&(step.clone(), operation.to_owned())) {
            Some(wait) => wait.due(now),
            None => true,
        }
    }

    /// Records that `step` failed at `now` with `answer`: the first failure starts the wait, a further one grows the
    /// pause. Returns `Some` when the step starts to wait and `None` when it already waited: one event per wait.
    ///
    /// # Errors
    /// [`DurationOverflow`] when the next try cannot be recorded.
    pub(super) fn failed(
        &mut self,
        step: &Step,
        team: &TeamId,
        operation: &str,
        pr: Option<PrNumber>,
        answer: String,
        outlook: Outlook,
        now: Timestamp,
    ) -> Result<Option<&Wait>, DurationOverflow> {
        let key = (step.clone(), operation.to_owned());
        let (starts, kept) = match self.steps.remove(&key) {
            Some(mut wait) => {
                let starts = wait.outlook != outlook;
                wait.again(answer, outlook, now)?;
                (starts, wait)
            }
            None => {
                let wait = Wait::first(team.clone(), operation.to_owned(), pr, answer, outlook, now)?;
                (true, wait)
            }
        };
        if self.steps.len() >= MAX_WAITS
            && let Some(oldest) = self.steps.keys().next().cloned()
        {
            self.steps.remove(&oldest);
        }
        self.steps.insert(key.clone(), kept);
        Ok(starts.then(|| self.steps.get(&key)).flatten())
    }

    /// Forgets the wait of `step` for `operation`, which is done; `true` when it waited and is no longer.
    pub(super) fn done(&mut self, step: &Step, operation: &str) -> bool {
        self.steps.remove(&(step.clone(), operation.to_owned())).is_some()
    }

    /// Forgets every wait of `team`, so that the human's `agent resume` tries its steps at once.
    pub(super) fn forget_team(&mut self, team: &TeamId) {
        self.steps.retain(|_, wait| &wait.team != team);
    }

    /// The wait of each open round of `team`, by round, as `agent status` tells it: the one that is due last.
    pub(super) fn of_team(&self, team: &TeamId, offset: time::UtcOffset) -> Vec<(RoundId, String)> {
        let mut found: BTreeMap<RoundId, (Option<Timestamp>, String)> = BTreeMap::new();
        for ((step, _), wait) in &self.steps {
            if &wait.team != team {
                continue;
            }
            let Step::Round { round } = step else {
                continue;
            };
            let line = wait.line(offset);
            match found.get_mut(round) {
                Some((at, _)) if *at >= wait.next_at => {}
                Some(entry) => *entry = (wait.next_at, line),
                None => {
                    found.insert(round.clone(), (wait.next_at, line));
                }
            }
        }
        found.into_iter().map(|(round, (_, line))| (round, line)).collect()
    }
}
