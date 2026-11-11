//! Statuses: how a run ended, a review's verdict, and the life of a task and of a task-list item.
//!
//! [`TaskStatus`] and [`ItemStatus`] are state machines: [`StatusMachine::transition_to`] is the one way from a status
//! to the next, and it refuses every change its table does not allow with an [`InvalidTransition`].

use std::fmt;

use crate::kinds::keyword_enum;

keyword_enum! {
    /// How the agent says its run ended: the word after `STATUS:` in its final message.
    ///
    /// The Bash `agent` also knew `ROUND_DONE` (a round is a task-list item now, ended with `DONE`) and
    /// `INVESTIGATING` (standalone investigation requests are not part of this harness); both are refused.
    pub enum RunOutcome: "run outcome" {
        /// The work asked for is finished.
        Done => "DONE",
        /// The agent wrote questions for the human and waits for the answers.
        Questions => "QUESTIONS",
        /// The agent could not do the work.
        Failed => "FAILED",
        /// A team member has nothing to do until a message wakes it.
        Sleeping => "SLEEPING",
        /// A member the coordinator excluded from the team acknowledges it and stops.
        Excluded => "EXCLUDED",
    }
}

keyword_enum! {
    /// A reviewer's verdict on a pull request: the `VERDICT:` line of the review rules' report.
    pub enum ReviewVerdict: "review verdict" {
        /// No blocker: the pull request may be merged.
        Approve => "APPROVE",
        /// At least one blocker: the team gets a fix pass.
        RequestChanges => "REQUEST_CHANGES",
    }
}

keyword_enum! {
    /// Where a task is in its life: one agent's work, solo or in a team, made of model runs.
    ///
    /// Allowed changes ([`StatusMachine::successors`]):
    ///
    /// | from | to |
    /// |---|---|
    /// | `queued` | `running`, `paused`, `stopped` |
    /// | `running` | `queued`, `waiting`, `questions`, `paused`, `stopped`, `failed`, `done` |
    /// | `waiting` | `queued`, `paused`, `stopped`, `done` |
    /// | `questions` | `queued`, `stopped` |
    /// | `paused` | `queued`, `stopped` |
    /// | `stopped` | `queued` |
    /// | `failed` | `queued`, `stopped` |
    /// | `done` | — |
    pub enum TaskStatus: "task status" {
        /// Its next run is due and waits to start.
        Queued => "queued",
        /// A model run is in progress.
        Running => "running",
        /// A team member waits for a message, an assignment or a merge.
        Waiting => "waiting",
        /// It waits for the human's answers to its questions.
        Questions => "questions",
        /// Paused by the human or by an exhausted quota; it is queued again when unpaused.
        Paused => "paused",
        /// Stopped by the human; it is queued again when resumed.
        Stopped => "stopped",
        /// Its last run failed; it is queued again when resumed.
        Failed => "failed",
        /// Finished. Nothing follows.
        Done => "done",
    }
}

keyword_enum! {
    /// How far an item of a team's task list is.
    ///
    /// Allowed changes ([`StatusMachine::successors`]):
    ///
    /// | from | to |
    /// |---|---|
    /// | `pending` | `in_progress` |
    /// | `in_progress` | `pending`, `in_review` |
    /// | `in_review` | `in_progress`, `approved` |
    /// | `approved` | `done` |
    /// | `done` | — |
    pub enum ItemStatus: "item status" {
        /// Not started: an earlier item of its feature or a feature it depends on is not finished, or no member is
        /// free.
        Pending => "pending",
        /// A member works on it, or the coordinator integrates it, until its pull request is open; a fix pass
        /// brings it back here.
        InProgress => "in_progress",
        /// Its pull request is open and waits for its two reviews.
        InReview => "in_review",
        /// Both reviews approve; it waits for its merge (with `--no-merge`, by the human).
        Approved => "approved",
        /// Merged, or marked done (`[x]`) in the task list. Nothing follows.
        Done => "done",
    }
}

/// A status with a table of allowed changes.
pub trait StatusMachine: Copy + PartialEq + fmt::Debug + fmt::Display + 'static {
    /// The statuses this one may change to.
    fn successors(self) -> &'static [Self];

    /// Changes this status to `next`.
    ///
    /// # Errors
    /// [`InvalidTransition`] when `next` is not one of [`Self::successors`], a change to the same status included.
    fn transition_to(self, next: Self) -> Result<Self, InvalidTransition<Self>> {
        if self.successors().contains(&next) {
            Ok(next)
        } else {
            Err(InvalidTransition { from: self, to: next })
        }
    }
}

impl StatusMachine for TaskStatus {
    fn successors(self) -> &'static [Self] {
        &[]
    }
}

impl StatusMachine for ItemStatus {
    fn successors(self) -> &'static [Self] {
        &[]
    }
}

/// A change of status that the status's table does not allow.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("cannot change the status from {from} to {to}")]
pub struct InvalidTransition<S: StatusMachine> {
    /// The status that was to change.
    pub from: S,
    /// The refused next status.
    pub to: S,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kinds::tests::{assert_keywords, assert_refuses};

    const RUN_OUTCOMES: &[(RunOutcome, &str)] = &[
        (RunOutcome::Done, "DONE"),
        (RunOutcome::Questions, "QUESTIONS"),
        (RunOutcome::Failed, "FAILED"),
        (RunOutcome::Sleeping, "SLEEPING"),
        (RunOutcome::Excluded, "EXCLUDED"),
    ];

    const REVIEW_VERDICTS: &[(ReviewVerdict, &str)] = &[
        (ReviewVerdict::Approve, "APPROVE"),
        (ReviewVerdict::RequestChanges, "REQUEST_CHANGES"),
    ];

    const TASK_STATUSES: &[(TaskStatus, &str)] = &[
        (TaskStatus::Queued, "queued"),
        (TaskStatus::Running, "running"),
        (TaskStatus::Waiting, "waiting"),
        (TaskStatus::Questions, "questions"),
        (TaskStatus::Paused, "paused"),
        (TaskStatus::Stopped, "stopped"),
        (TaskStatus::Failed, "failed"),
        (TaskStatus::Done, "done"),
    ];

    const ITEM_STATUSES: &[(ItemStatus, &str)] = &[
        (ItemStatus::Pending, "pending"),
        (ItemStatus::InProgress, "in_progress"),
        (ItemStatus::InReview, "in_review"),
        (ItemStatus::Approved, "approved"),
        (ItemStatus::Done, "done"),
    ];

    #[test]
    fn run_outcomes_are_the_status_words_of_the_protocol() {
        assert_eq!(
            RunOutcome::ALL,
            RUN_OUTCOMES.iter().map(|&(o, _)| o).collect::<Vec<_>>()
        );
        assert_keywords(RUN_OUTCOMES);
    }

    #[test]
    fn run_outcomes_refuse_the_words_this_harness_dropped_and_unknown_ones() {
        assert_refuses(
            "run outcome",
            RUN_OUTCOMES,
            &["ROUND_DONE", "INVESTIGATING", "STATUS: DONE", "OK"],
        );
    }

    #[test]
    fn review_verdicts_are_the_verdict_words_of_the_review_rules() {
        assert_eq!(
            ReviewVerdict::ALL,
            REVIEW_VERDICTS.iter().map(|&(v, _)| v).collect::<Vec<_>>()
        );
        assert_keywords(REVIEW_VERDICTS);
    }

    #[test]
    fn review_verdicts_refuse_unknown_words() {
        assert_refuses(
            "review verdict",
            REVIEW_VERDICTS,
            &["APPROVED", "REQUEST CHANGES", "COMMENT", "LGTM"],
        );
    }

    #[test]
    fn task_statuses_have_their_texts_and_round_trip() {
        assert_eq!(
            TaskStatus::ALL,
            TASK_STATUSES.iter().map(|&(s, _)| s).collect::<Vec<_>>()
        );
        assert_keywords(TASK_STATUSES);
    }

    #[test]
    fn task_statuses_refuse_unknown_texts() {
        assert_refuses("task status", TASK_STATUSES, &["sleeping", "active", "closed"]);
    }

    #[test]
    fn item_statuses_have_their_texts_and_round_trip() {
        assert_eq!(
            ItemStatus::ALL,
            ITEM_STATUSES.iter().map(|&(s, _)| s).collect::<Vec<_>>()
        );
        assert_keywords(ITEM_STATUSES);
    }

    #[test]
    fn item_statuses_refuse_unknown_texts() {
        assert_refuses(
            "item status",
            ITEM_STATUSES,
            &["in-progress", "inprogress", "merged", "todo"],
        );
    }

    /// Asserts that exactly the changes in `allowed` succeed, each to its target, and that every other change between
    /// two of `all` — a change to the same status included — is refused with its exact [`InvalidTransition`].
    fn assert_transitions<S: StatusMachine>(all: &[S], allowed: &[(S, S)]) {
        for &from in all {
            for &to in all {
                let expected = if allowed.contains(&(from, to)) {
                    Ok(to)
                } else {
                    Err(InvalidTransition { from, to })
                };
                assert_eq!(from.transition_to(to), expected, "{from} -> {to}");
            }
        }
    }

    #[test]
    fn task_statuses_change_only_along_their_table() {
        use TaskStatus::{Done, Failed, Paused, Questions, Queued, Running, Stopped, Waiting};
        let allowed = [
            (Queued, Running),
            (Queued, Paused),
            (Queued, Stopped),
            (Running, Queued),
            (Running, Waiting),
            (Running, Questions),
            (Running, Paused),
            (Running, Stopped),
            (Running, Failed),
            (Running, Done),
            (Waiting, Queued),
            (Waiting, Paused),
            (Waiting, Stopped),
            (Waiting, Done),
            (Questions, Queued),
            (Questions, Stopped),
            (Paused, Queued),
            (Paused, Stopped),
            (Stopped, Queued),
            (Failed, Queued),
            (Failed, Stopped),
        ];
        assert_transitions(TaskStatus::ALL, &allowed);
    }

    #[test]
    fn item_statuses_change_only_along_their_table() {
        use ItemStatus::{Approved, Done, InProgress, InReview, Pending};
        let allowed = [
            (Pending, InProgress),
            (InProgress, Pending),
            (InProgress, InReview),
            (InReview, InProgress),
            (InReview, Approved),
            (Approved, Done),
        ];
        assert_transitions(ItemStatus::ALL, &allowed);
    }

    #[test]
    fn done_tasks_and_items_have_no_successor() {
        assert_eq!(TaskStatus::Done.successors(), &[]);
        assert_eq!(ItemStatus::Done.successors(), &[]);
    }
}
