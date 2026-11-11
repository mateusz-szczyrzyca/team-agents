//! Runs: one start of a model CLI for a task, and how it ended.

use serde::{Deserialize, Serialize};

use super::values::{SessionId, Timestamp};
use crate::ids::RunId;
use crate::status::RunOutcome;

/// One run of a model CLI for a task: when it started, the session the CLI gave it, and how it ended.
///
/// Made by [`Member::begin_run`](super::Member::begin_run) and ended by
/// [`Member::end_run`](super::Member::end_run), so a run never ends before it started; reading a state checks that
/// too.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "RunWire")]
pub struct Run {
    /// The run's ID.
    id: RunId,
    /// When the CLI was started.
    started_at: Timestamp,
    /// The CLI's session ID, once it has reported one; a later run resumes it.
    #[serde(skip_serializing_if = "Option::is_none")]
    session: Option<SessionId>,
    /// Whether it is still going, or how it ended.
    state: RunState,
}

/// What a [`Run`] is written as, before its checks.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RunWire {
    /// See [`Run`].
    id: RunId,
    /// See [`Run`].
    started_at: Timestamp,
    /// See [`Run`].
    #[serde(default)]
    session: Option<SessionId>,
    /// See [`Run`].
    state: RunState,
}

/// Whether a run is going or over.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum RunState {
    /// The CLI process is going.
    Running,
    /// The run is over.
    Ended {
        /// When it ended.
        at: Timestamp,
        /// How it ended.
        end: RunEnd,
    },
}

/// How a run ended: what the agent reported, or what the harness found.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RunEnd {
    /// The agent's final message ended with `STATUS: <outcome>`.
    Reported {
        /// The status the agent reported.
        outcome: RunOutcome,
    },
    /// The CLI ended without a `STATUS:` line.
    NoStatus {
        /// The CLI's exit code; `None` when a signal ended it.
        exit_code: Option<i32>,
    },
    /// The account's quota is used up.
    QuotaExhausted {
        /// When the CLI says the quota is back; `None` when it says nothing.
        #[serde(skip_serializing_if = "Option::is_none")]
        resets_at: Option<Timestamp>,
    },
    /// The provider limited the rate of requests.
    RateLimited,
    /// The run showed no activity for too long and was stopped by the watchdog.
    Hung,
    /// The human stopped it.
    Stopped,
}

/// Why a run could not be started, ended or given a session.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RunError {
    /// A run is going: only one runs at a time.
    #[error("run {run} is still going")]
    AlreadyRunning {
        /// The run that is going.
        run: RunId,
    },
    /// The task already has a run with this ID.
    #[error("the task already has the run {run}")]
    DuplicateRun {
        /// The repeated ID.
        run: RunId,
    },
    /// No run is going.
    #[error("no run is going")]
    NotRunning,
    /// The end is before the start.
    #[error("run {run} cannot end at {at}, before it started at {started_at}")]
    EndsBeforeStart {
        /// The run.
        run: RunId,
        /// When it started.
        started_at: Timestamp,
        /// The refused end.
        at: Timestamp,
    },
}

impl Run {
    /// A run that has just started.
    pub(super) const fn started(id: RunId, started_at: Timestamp, session: Option<SessionId>) -> Self {
        Self {
            id,
            started_at,
            session,
            state: RunState::Running,
        }
    }

    /// The run's ID.
    #[must_use]
    pub const fn id(&self) -> &RunId {
        &self.id
    }

    /// When the CLI was started.
    #[must_use]
    pub const fn started_at(&self) -> Timestamp {
        self.started_at
    }

    /// The CLI's session ID, if it has reported one.
    #[must_use]
    pub const fn session(&self) -> Option<&SessionId> {
        self.session.as_ref()
    }

    /// Whether the run is going or how it ended.
    #[must_use]
    pub const fn state(&self) -> &RunState {
        &self.state
    }

    /// Whether the run is going.
    #[must_use]
    pub const fn is_running(&self) -> bool {
        matches!(self.state, RunState::Running)
    }

    /// Records the session the CLI reported.
    pub(super) fn attach_session(&mut self, session: SessionId) {
        self.session = Some(session);
    }

    /// Ends the run.
    pub(super) fn end(&mut self, at: Timestamp, end: RunEnd) -> Result<(), RunError> {
        if !self.is_running() {
            return Err(RunError::NotRunning);
        }
        check_end(&self.id, self.started_at, at)?;
        self.state = RunState::Ended { at, end };
        Ok(())
    }
}

/// Checks that a run that started at `started_at` may end at `at`.
fn check_end(id: &RunId, started_at: Timestamp, at: Timestamp) -> Result<(), RunError> {
    if at < started_at {
        return Err(RunError::EndsBeforeStart {
            run: id.clone(),
            started_at,
            at,
        });
    }
    Ok(())
}

impl TryFrom<RunWire> for Run {
    type Error = RunError;

    fn try_from(wire: RunWire) -> Result<Self, Self::Error> {
        if let RunState::Ended { at, .. } = &wire.state {
            check_end(&wire.id, wire.started_at, *at)?;
        }
        Ok(Self {
            id: wire.id,
            started_at: wire.started_at,
            session: wire.session,
            state: wire.state,
        })
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::state::fixtures::{assert_refused, at, run as run_id};

    /// A run that started at 5 seconds with no session yet.
    fn started() -> Run {
        Run::started(run_id(1), at(5), None)
    }

    #[test]
    fn a_new_run_is_going_and_has_the_session_it_was_given() {
        let session: SessionId = "ses-1".parse().unwrap();

        let fresh = Run::started(run_id(1), at(5), None);
        let resumed = Run::started(run_id(2), at(6), Some(session.clone()));

        assert_eq!(
            (fresh.id(), fresh.started_at(), fresh.session()),
            (&run_id(1), at(5), None)
        );
        assert_eq!(fresh.state(), &RunState::Running);
        assert!(fresh.is_running());
        assert_eq!(resumed.session(), Some(&session));
    }

    #[test]
    fn a_session_can_be_attached_while_the_run_goes() {
        let mut run = started();

        run.attach_session("ses-2".parse().unwrap());

        assert_eq!(run.session().map(SessionId::as_str), Some("ses-2"));
    }

    #[test]
    fn a_run_ends_with_its_time_and_how() {
        let mut run = started();

        run.end(
            at(65),
            RunEnd::Reported {
                outcome: RunOutcome::Done,
            },
        )
        .unwrap();

        assert_eq!(
            run.state(),
            &RunState::Ended {
                at: at(65),
                end: RunEnd::Reported {
                    outcome: RunOutcome::Done
                }
            }
        );
        assert!(!run.is_running());
    }

    #[test]
    fn a_run_may_end_at_the_moment_it_started() {
        let mut run = started();

        run.end(at(5), RunEnd::Hung).unwrap();

        assert!(!run.is_running());
    }

    #[test]
    fn a_run_cannot_end_before_it_started_and_stays_as_it_was() {
        let mut run = started();

        let err = run.end(at(4), RunEnd::Stopped).unwrap_err();

        assert_eq!(
            err,
            RunError::EndsBeforeStart {
                run: run_id(1),
                started_at: at(5),
                at: at(4)
            }
        );
        assert!(run.is_running());
    }

    #[test]
    fn a_run_that_has_ended_cannot_end_again() {
        let mut run = started();
        run.end(at(6), RunEnd::Hung).unwrap();

        assert_eq!(run.end(at(7), RunEnd::Stopped), Err(RunError::NotRunning));
        assert_eq!(
            run.state(),
            &RunState::Ended {
                at: at(6),
                end: RunEnd::Hung
            }
        );
    }

    #[test]
    fn every_way_a_run_can_end_round_trips_through_json() {
        let ends = [
            (
                RunEnd::Reported {
                    outcome: RunOutcome::Questions,
                },
                json!({"kind": "reported", "outcome": "QUESTIONS"}),
            ),
            (
                RunEnd::NoStatus { exit_code: Some(2) },
                json!({"kind": "no_status", "exit_code": 2}),
            ),
            (
                RunEnd::NoStatus { exit_code: None },
                json!({"kind": "no_status", "exit_code": null}),
            ),
            (
                RunEnd::QuotaExhausted {
                    resets_at: Some(at(3600)),
                },
                json!({"kind": "quota_exhausted", "resets_at": "2026-11-11T20:15:00Z"}),
            ),
            (
                RunEnd::QuotaExhausted { resets_at: None },
                json!({"kind": "quota_exhausted"}),
            ),
            (RunEnd::RateLimited, json!({"kind": "rate_limited"})),
            (RunEnd::Hung, json!({"kind": "hung"})),
            (RunEnd::Stopped, json!({"kind": "stopped"})),
        ];
        for (end, expected) in ends {
            let value = serde_json::to_value(&end).unwrap();

            assert_eq!(value, expected);
            assert_eq!(serde_json::from_value::<RunEnd>(value).unwrap(), end);
        }
    }

    #[test]
    fn a_run_is_written_without_a_session_until_it_has_one_and_reads_back_equal() {
        let mut run = started();

        assert_eq!(
            serde_json::to_value(&run).unwrap(),
            json!({"id": "run-20261111-153001-r7q2", "started_at": "2026-11-11T19:15:05Z", "state": {"status": "running"}})
        );
        run.attach_session("ses-1".parse().unwrap());
        run.end(at(9), RunEnd::Hung).unwrap();
        let value = serde_json::to_value(&run).unwrap();

        assert_eq!(
            value,
            json!({
                "id": "run-20261111-153001-r7q2",
                "started_at": "2026-11-11T19:15:05Z",
                "session": "ses-1",
                "state": {"status": "ended", "at": "2026-11-11T19:15:09Z", "end": {"kind": "hung"}}
            })
        );
        assert_eq!(serde_json::from_value::<Run>(value).unwrap(), run);
    }

    #[test]
    fn a_run_that_ends_before_it_started_is_refused_when_read() {
        let value = json!({
            "id": "run-20261111-153001-r7q2",
            "started_at": "2026-11-11T19:15:05Z",
            "state": {"status": "ended", "at": "2026-11-11T19:15:04Z", "end": {"kind": "hung"}}
        });

        let err = serde_json::from_value::<Run>(value).unwrap_err();

        assert_eq!(err.classify(), serde_json::error::Category::Data);
    }

    #[test]
    fn a_run_with_an_unknown_field_or_state_or_end_is_refused() {
        let bad = [
            json!({"id": "run-20261111-153001-r7q2", "started_at": "2026-11-11T19:15:05Z", "state": {"status": "running"}, "extra": 1}),
            json!({"id": "run-20261111-153001-r7q2", "started_at": "2026-11-11T19:15:05Z", "state": {"status": "paused"}}),
            json!({"id": "run-20261111-153001-r7q2", "started_at": "2026-11-11T19:15:05Z", "state": {"status": "ended", "at": "2026-11-11T19:15:06Z", "end": {"kind": "exploded"}}}),
            json!({"id": "team-20261111-153001-r7q2", "started_at": "2026-11-11T19:15:05Z", "state": {"status": "running"}}),
        ];
        for value in bad {
            assert_refused::<Run>(&value.clone());
        }
    }
}
