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
use agent_core::state::{InvalidTimestamp, PrNumber, Timestamp};
use agent_runtime::git::{GitError, GitOperation};
use agent_runtime::github::Outlook;
use time::Duration;

/// How many steps may wait for GitHub at once: the owner's memory stays bounded whatever fails. Beyond it the wait with
/// the smallest key (step, round) gives way; that step is then tried at its next look, as after a start of the daemon.
pub(in crate::daemon::owner) const MAX_WAITS: usize = 256;

/// Whether a failed step may pass at a later try, from what it failed with: a GitHub failure says so itself, and a
/// git call that reaches GitHub (a fetch, a push, a branch read on the remote) may, while git's own refusals (a lock, a
/// missing branch of the checkout, a rejected configuration) will not.
pub(in crate::daemon::owner) fn outlook_of(error: &super::super::PublishError) -> Outlook {
    use super::super::PublishError;
    match error {
        PublishError::GitHub(error) => error.outlook(),
        PublishError::Git(GitError::Run { operation, .. } | GitError::Failed { operation, .. })
            if matches!(
                operation,
                GitOperation::Fetch | GitOperation::Push | GitOperation::RemoteContains
            ) =>
        {
            Outlook::MayPass
        }
        PublishError::Git(_)
        | PublishError::Findings(_)
        | PublishError::Path { .. }
        | PublishError::Description { .. }
        | PublishError::Origin(_)
        | PublishError::Store(_)
        | PublishError::Text(_)
        | PublishError::Render(_)
        | PublishError::Variable(_)
        | PublishError::Round(_)
        | PublishError::RepoGone { .. }
        | PublishError::Id(_)
        | PublishError::Name(_)
        | PublishError::Branch(_)
        | PublishError::Notification(_)
        | PublishError::Checks(_)
        | PublishError::Questions(_)
        | PublishError::Write(_)
        | PublishError::QuestionNumbers
        | PublishError::Reviewer(_)
        | PublishError::Asking(_)
        | PublishError::Replacing(_)
        | PublishError::Lost => Outlook::WillNotPass,
    }
}

/// The time of day `time` as a reader of the local clock sees it, `HH:MM`.
#[must_use]
pub(in crate::daemon::owner) fn clock_time(time: Timestamp, offset: time::UtcOffset) -> String {
    let local = time.as_utc().to_offset(offset);
    format!("{:02}:{:02}", local.hour(), local.minute())
}

/// Which step of a round that calls GitHub waits: its key in the waits, its words in the status and the log, and the
/// notice of its refusal. One variant per place that waits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(in crate::daemon::owner) enum WaitingStep {
    /// Rewriting the description of the open pull requests of a team.
    RefreshDescriptions,
    /// Making or moving the worktree of the investigator that gives a second opinion.
    AskOpinion,
}

impl WaitingStep {
    /// The step's words, for the status line and the log.
    #[must_use]
    pub(in crate::daemon::owner) const fn operation(self) -> &'static str {
        match self {
            Self::RefreshDescriptions => "refresh the description of the pull requests",
            Self::AskOpinion => "prepare a second opinion",
        }
    }
}

impl std::fmt::Display for WaitingStep {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.operation())
    }
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
    /// Which step it is.
    pub(super) step: WaitingStep,
    /// What the step does with its pull request, in the words of `agent status`: `prepare a second opinion #138`.
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
        step: WaitingStep,
        pr: Option<PrNumber>,
        answer: String,
        outlook: Outlook,
        now: Timestamp,
        pause: Duration,
    ) -> Result<Self, InvalidTimestamp> {
        let tries = NonZeroU32::MIN;
        let next_at = match outlook {
            Outlook::MayPass => Some(later(now, pause)?),
            Outlook::WillNotPass => None,
        };
        Ok(Self {
            team,
            operation: named(step, pr),
            step,
            answer,
            outlook,
            tries,
            next_at,
        })
    }

    /// The next failure of the same step, at `now`.
    fn again(
        &mut self,
        answer: String,
        outlook: Outlook,
        now: Timestamp,
        pause: Duration,
    ) -> Result<(), InvalidTimestamp> {
        self.tries = self.tries.saturating_add(1);
        self.answer = answer;
        self.outlook = outlook;
        self.next_at = match outlook {
            Outlook::MayPass => Some(later(now, pause)?),
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

    /// What `agent status` says of it: `prepare a second opinion #138, tried 4 times, next try 01:07`; for a step the
    /// human resumed `..., tried 4 times, tried again at the next look`; for a refusal `..., refused: <answer>`.
    pub(super) fn line(&self, offset: time::UtcOffset) -> String {
        let tried = self.tries.get();
        let times = if tried == 1 { "time" } else { "times" };
        match (self.outlook, self.next_at) {
            (Outlook::MayPass, Some(at)) => format!(
                "{}, tried {tried} {times}, next try {}",
                self.operation,
                minute_shown(at, offset)
            ),
            (Outlook::MayPass, None) => format!(
                "{}, tried {tried} {times}, tried again at the next look",
                self.operation
            ),
            (Outlook::WillNotPass, _) => format!("{}, refused: {}", self.operation, self.answer),
        }
    }
}

/// `operation #pr`, or `operation` for a step that is about no one pull request.
fn named(step: WaitingStep, pr: Option<PrNumber>) -> String {
    match pr {
        Some(number) => format!("{} #{number}", step.operation()),
        None => step.operation().to_owned(),
    }
}

/// The minute a time shows, rounded up: 25 seconds past 00:04 shows 00:05, as the human reads a time in the log.
fn minute_shown(at: Timestamp, offset: time::UtcOffset) -> String {
    let second = i64::from(at.as_utc().second());
    let shown = at
        .as_utc()
        .saturating_add(Duration::seconds(60_i64.saturating_sub(second)));
    match Timestamp::from_utc(shown) {
        Ok(shown) => clock_time(shown, offset),
        // A next try in the last minute a time records is shown as it is: there is no later minute to round to.
        Err(_past_the_last_year) => clock_time(at, offset),
    }
}

/// `now` plus `span`.
///
/// # Errors
/// [`InvalidTimestamp`] when the time after the pause cannot be recorded.
fn later(now: Timestamp, span: Duration) -> Result<Timestamp, InvalidTimestamp> {
    Timestamp::from_utc(now.as_utc().saturating_add(span))
}

/// What a failure did to the wait of its step.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::daemon::owner) enum Failure {
    /// The step starts to wait: its first failure.
    First,
    /// It waited for a pause that is over and failed again: the pause grows.
    Again,
    /// It stopped being worth trying: it waits for the human now.
    Starts,
    /// It waits for the human, as it did.
    RefusedAgain,
}

/// The steps that failed and wait, by round and by step.
#[derive(Debug, Default)]
pub(in crate::daemon::owner) struct Waits {
    /// The waits of each round, and of each team's rounds as a whole, by step.
    steps: BTreeMap<(Step, WaitingStep), Wait>,
}

impl Waits {
    /// The wait of the round `round` for `operation`, if it has one.
    #[cfg(test)]
    pub(super) fn for_round(&self, round: &RoundId, step: WaitingStep) -> Option<&Wait> {
        self.steps.get(&(Step::Round { round: round.clone() }, step))
    }

    /// Whether `step` is to be tried now: it has not failed, its pause is over, or it waits for the human.
    pub(in crate::daemon::owner) fn due(&self, place: &Step, step: WaitingStep, now: Timestamp) -> bool {
        match self.steps.get(&(place.clone(), step)) {
            Some(wait) => wait.due(now),
            None => true,
        }
    }

    /// Records that `step` failed at `now` with `answer`: the first failure starts the wait, a further one grows the
    /// pause. Returns what the failure did to the wait, and the wait as it is now.
    ///
    /// # Errors
    /// [`InvalidTimestamp`] when the next try cannot be recorded.
    #[expect(
        clippy::too_many_arguments,
        reason = "the committed tests of the waits call it with each value on its own; a struct of them is a refactoring of its own"
    )]
    pub(super) fn failed(
        &mut self,
        place: &Step,
        team: &TeamId,
        step: WaitingStep,
        pr: Option<PrNumber>,
        answer: String,
        outlook: Outlook,
        now: Timestamp,
    ) -> Result<(Failure, Wait), InvalidTimestamp> {
        let key = (place.clone(), step);
        let failure = match self.steps.get(&key).map(|wait| wait.outlook) {
            Some(Outlook::MayPass) if outlook == Outlook::MayPass => Failure::Again,
            Some(Outlook::WillNotPass) if outlook == Outlook::WillNotPass => Failure::RefusedAgain,
            Some(Outlook::MayPass | Outlook::WillNotPass) => Failure::Starts,
            None => Failure::First,
        };
        let wait = match self.steps.remove(&key) {
            Some(mut wait) => {
                wait.again(answer, outlook, now, pause(wait.tries.saturating_add(1)))?;
                wait
            }
            None => Wait::first(team.clone(), step, pr, answer, outlook, now, pause(NonZeroU32::MIN))?,
        };
        if self.steps.len() >= MAX_WAITS
            && let Some(oldest) = self.steps.keys().next().cloned()
        {
            self.steps.remove(&oldest);
        }
        self.steps.insert(key, wait.clone());
        Ok((failure, wait))
    }

    /// Forgets the wait of `step` for `operation`, which is done; `true` when it waited and is no longer.
    pub(super) fn done(&mut self, place: &Step, step: WaitingStep) -> bool {
        self.steps.remove(&(place.clone(), step)).is_some()
    }

    /// How many waits it keeps.
    #[cfg(test)]
    pub(super) fn kept(&self) -> usize {
        self.steps.len()
    }

    /// Makes every wait of `team` due, so that the human's `agent resume` tries its steps at the next look.
    ///
    /// A wait is kept, due at once, rather than dropped: the step that then goes through is logged as going through,
    /// and one that fails again starts to wait again, with its tries counted on.
    pub(super) fn forget_team(&mut self, team: &TeamId) {
        for wait in self.steps.values_mut().filter(|wait| &wait.team == team) {
            wait.outlook = Outlook::MayPass;
            wait.next_at = None;
        }
    }

    /// What `agent status` says `round` of `team` waits for: the step of the round itself, else a step of all the
    /// team's rounds; the one tried last when several wait, and a refusal before all of them.
    pub(in crate::daemon::owner) fn line_of(
        &self,
        team: &TeamId,
        round: &RoundId,
        offset: time::UtcOffset,
    ) -> Option<String> {
        let own = Step::Round { round: round.clone() };
        let shared = Step::Team { team: team.clone() };
        self.steps
            .iter()
            .filter(|((place, _), wait)| *place == own || (*place == shared && &wait.team == team))
            .map(|(_, wait)| wait)
            .max_by_key(|wait| (wait.outlook == Outlook::WillNotPass, wait.next_at))
            .map(|wait| wait.line(offset))
    }
}
