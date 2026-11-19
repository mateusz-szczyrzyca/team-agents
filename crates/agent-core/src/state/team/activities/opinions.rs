//! Reconstructs opinion assignments without consuming or changing their scheduling ledger.
use super::{Activity, ActivityEnd, ActivityOutcome, ActivityRequest, Team};
use crate::ids::RoundId;
use crate::state::{ItemProgress, OpinionAttempt, OpinionStage, RunState, Timestamp};

impl Team {
    /// Collects retained attempts, including assignments from before attempt timestamps existed.
    pub(super) fn opinion_activities(&self, item: &ItemProgress, rounds: &[RoundId], now: Timestamp) -> Vec<Activity> {
        let mut found = Vec::new();
        let Some(ledger) = item.findings() else {
            return found;
        };
        for (number, opinion) in ledger.opinions() {
            let legacy;
            let attempts = if ledger.opinion_attempts(*number).is_empty() {
                legacy = match &opinion.stage {
                    OpinionStage::Waiting => Vec::new(),
                    OpinionStage::Assigned { investigator, task }
                    | OpinionStage::Reported { investigator, task, .. } => vec![OpinionAttempt {
                        investigator: investigator.clone(),
                        task: task.clone(),
                        asked_at: None,
                        failure: None,
                    }],
                };
                &legacy[..]
            } else {
                ledger.opinion_attempts(*number)
            };
            for attempt in attempts {
                let member = self.find(&attempt.task).map(|(_, member)| member);
                // Earlier schemas lack assignment times: the first run, or team creation, is the earliest evidence.
                let asked_at = attempt
                    .asked_at
                    .or_else(|| member.and_then(|m| m.runs().first().map(crate::state::Run::started_at)))
                    .unwrap_or(self.created_at);
                let ended_at = member
                    .and_then(|m| m.runs().last())
                    .and_then(|run| match run.state() {
                        RunState::Ended { at, .. } => Some(*at),
                        RunState::Running => None,
                    })
                    .unwrap_or(now)
                    .max(asked_at);
                let end = match attempt.failure {
                    Some(reason) => Some(ActivityEnd::OpinionFailed { reason }),
                    None => match &opinion.stage {
                        OpinionStage::Reported { task, .. } if *task == attempt.task => Some(ActivityEnd::Reported),
                        OpinionStage::Reported { .. } | OpinionStage::Assigned { .. } | OpinionStage::Waiting => None,
                    },
                };
                found.push(Activity {
                    investigator: attempt.investigator.clone(),
                    task: attempt.task.clone(),
                    rounds: rounds.to_vec(),
                    request: ActivityRequest::SecondOpinion {
                        pass: opinion.request.pass,
                        finding: *number,
                    },
                    asked_at,
                    outcome: end.map_or(ActivityOutcome::Working, |end| ActivityOutcome::Finished {
                        at: ended_at,
                        end,
                    }),
                });
            }
        }
        found
    }
}
