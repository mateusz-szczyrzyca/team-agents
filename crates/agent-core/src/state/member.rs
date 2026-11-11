//! Members: an agent or investigator and the task it works in, with the task's status, runs and questions.

use std::collections::HashSet;
use std::fmt;
use std::num::NonZeroU32;

use serde::{Deserialize, Serialize};

use super::question::{AnswerBeforeQuestion, AnsweredVia, Question};
use super::run::{Run, RunEnd, RunError};
use super::values::{SessionId, Timestamp};
use crate::ids::{AgentName, InvestigatorName, RunId, TaskId};
use crate::kinds::Tier;
use crate::status::{InvalidTransition, StatusMachine, TaskStatus};

/// Who works in a task: a normal agent, a worker or an investigator, by the tier of the configuration they are in.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "tier", content = "name", rename_all = "snake_case")]
pub enum Participant {
    /// A normal agent.
    Normal(AgentName),
    /// A worker.
    Worker(AgentName),
    /// An investigator.
    Investigator(InvestigatorName),
}

impl fmt::Display for Participant {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} ({})", self.name(), self.tier())
    }
}

impl Participant {
    /// The tier the participant is configured in.
    #[must_use]
    pub const fn tier(&self) -> Tier {
        match self {
            Self::Normal(_) => Tier::Normal,
            Self::Worker(_) => Tier::Worker,
            Self::Investigator(_) => Tier::Investigator,
        }
    }

    /// The participant's name; the configuration has one namespace for all three tiers.
    #[must_use]
    pub fn name(&self) -> &str {
        match self {
            Self::Normal(name) | Self::Worker(name) => name.as_str(),
            Self::Investigator(name) => name.as_str(),
        }
    }
}

/// A participant and its task: the status of the task, its runs and its rounds of questions.
///
/// The status changes only through [`Member::transition`]; runs and questions are added and ended through the
/// methods below, which keep them in order: at most the last run is going, at most the last question is open, and
/// question rounds count 1, 2, 3, … Reading a state checks the same.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "MemberWire")]
pub struct Member {
    /// The task's ID.
    task: TaskId,
    /// Who works in it.
    agent: Participant,
    /// Where the task is in its life.
    status: TaskStatus,
    /// Its runs, oldest first.
    runs: Vec<Run>,
    /// Its rounds of questions, oldest first.
    questions: Vec<Question>,
}

/// What a [`Member`] is written as, before its checks.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MemberWire {
    /// See [`Member`].
    task: TaskId,
    /// See [`Member`].
    agent: Participant,
    /// See [`Member`].
    status: TaskStatus,
    /// See [`Member`].
    #[serde(default)]
    runs: Vec<Run>,
    /// See [`Member`].
    #[serde(default)]
    questions: Vec<Question>,
}

/// Why a round of questions could not be added or answered.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum QuestionError {
    /// The last round is still open: answer it first.
    #[error("round {round} of the questions is still open")]
    StillOpen {
        /// The open round.
        round: NonZeroU32,
    },
    /// Rounds count 1, 2, 3, …: this is not the next one.
    #[error("the next round of questions is {expected}, not {got}")]
    WrongRound {
        /// The next round.
        expected: NonZeroU32,
        /// The refused round.
        got: NonZeroU32,
    },
    /// The task has no such round.
    #[error("the task has no round {round} of questions")]
    NoSuchRound {
        /// The round asked for.
        round: NonZeroU32,
    },
    /// The round was answered already.
    #[error("round {round} of the questions is already answered")]
    AlreadyAnswered {
        /// The round.
        round: NonZeroU32,
    },
    /// The answer is earlier than the question.
    #[error(transparent)]
    AnswerBeforeQuestion(#[from] AnswerBeforeQuestion),
}

/// Why a member's runs or questions are out of order, found when reading a state.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum MemberError {
    /// A run that is not the last is still going.
    #[error("task {task}: run {run} is still going but is not the last")]
    RunningNotLast {
        /// The task.
        task: TaskId,
        /// The run.
        run: RunId,
    },
    /// Two runs have one ID.
    #[error("task {task}: the run {run} appears twice")]
    DuplicateRun {
        /// The task.
        task: TaskId,
        /// The repeated ID.
        run: RunId,
    },
    /// The question rounds are not 1, 2, 3, … or an earlier one is still open.
    #[error("task {task}: the rounds of questions are out of order")]
    Questions {
        /// The task.
        task: TaskId,
        /// What is wrong.
        source: QuestionError,
    },
}

impl Member {
    /// A member whose task is queued and has no runs yet.
    #[must_use]
    pub const fn new(task: TaskId, agent: Participant) -> Self {
        Self {
            task,
            agent,
            status: TaskStatus::Queued,
            runs: Vec::new(),
            questions: Vec::new(),
        }
    }

    /// The task's ID.
    #[must_use]
    pub const fn task(&self) -> &TaskId {
        &self.task
    }

    /// Who works in the task.
    #[must_use]
    pub const fn agent(&self) -> &Participant {
        &self.agent
    }

    /// Where the task is in its life.
    #[must_use]
    pub const fn status(&self) -> TaskStatus {
        self.status
    }

    /// The runs, oldest first.
    #[must_use]
    pub fn runs(&self) -> &[Run] {
        &self.runs
    }

    /// The rounds of questions, oldest first.
    #[must_use]
    pub fn questions(&self) -> &[Question] {
        &self.questions
    }

    /// The run that is going, if one is.
    #[must_use]
    pub fn current_run(&self) -> Option<&Run> {
        self.runs.last().filter(|run| run.is_running())
    }

    /// The round of questions that waits for the human, if there is one.
    #[must_use]
    pub fn open_question(&self) -> Option<&Question> {
        self.questions.last().filter(|question| question.is_open())
    }

    /// The session of the latest run that has one: the one a new run resumes.
    #[must_use]
    pub fn last_session(&self) -> Option<&SessionId> {
        self.runs.iter().rev().find_map(Run::session)
    }

    /// Changes the task's status.
    ///
    /// # Errors
    /// [`InvalidTransition`] when the status's table does not allow the change.
    pub fn transition(&mut self, next: TaskStatus) -> Result<(), InvalidTransition<TaskStatus>> {
        self.status = self.status.transition_to(next)?;
        Ok(())
    }

    /// Records that a run has started.
    ///
    /// # Errors
    /// [`RunError::AlreadyRunning`] when a run is going; [`RunError::DuplicateRun`] when the task has a run with
    /// this ID.
    pub fn begin_run(&mut self, id: RunId, at: Timestamp, session: Option<SessionId>) -> Result<(), RunError> {
        if let Some(run) = self.current_run() {
            return Err(RunError::AlreadyRunning { run: run.id().clone() });
        }
        if self.runs.iter().any(|run| run.id() == &id) {
            return Err(RunError::DuplicateRun { run: id });
        }
        self.runs.push(Run::started(id, at, session));
        Ok(())
    }

    /// Records the session the CLI reported for the run that is going.
    ///
    /// # Errors
    /// [`RunError::NotRunning`] when no run is going.
    pub fn attach_session(&mut self, session: SessionId) -> Result<(), RunError> {
        self.current_run_mut()?.attach_session(session);
        Ok(())
    }

    /// Records how the run that is going ended.
    ///
    /// # Errors
    /// [`RunError::NotRunning`] when no run is going; [`RunError::EndsBeforeStart`] when `at` is before the run
    /// started.
    pub fn end_run(&mut self, at: Timestamp, end: RunEnd) -> Result<(), RunError> {
        self.current_run_mut()?.end(at, end)
    }

    /// The run that is going, to change it.
    fn current_run_mut(&mut self) -> Result<&mut Run, RunError> {
        self.runs
            .last_mut()
            .filter(|run| run.is_running())
            .ok_or(RunError::NotRunning)
    }

    /// Records that a round of questions was put to the human.
    ///
    /// # Errors
    /// [`QuestionError::StillOpen`] when the last round is open; [`QuestionError::WrongRound`] when the round is not
    /// the next one.
    pub fn ask_question(&mut self, question: Question) -> Result<(), QuestionError> {
        check_next_question(self.questions.last(), &question)?;
        self.questions.push(question);
        Ok(())
    }

    /// Records the human's answer to a round of questions.
    ///
    /// # Errors
    /// [`QuestionError::NoSuchRound`], [`QuestionError::AlreadyAnswered`], or
    /// [`QuestionError::AnswerBeforeQuestion`].
    pub fn answer_question(&mut self, round: NonZeroU32, at: Timestamp, via: AnsweredVia) -> Result<(), QuestionError> {
        let question = self
            .questions
            .iter_mut()
            .find(|question| question.round() == round)
            .ok_or(QuestionError::NoSuchRound { round })?;
        if !question.is_open() {
            return Err(QuestionError::AlreadyAnswered { round });
        }
        question.answer(at, via)?;
        Ok(())
    }
}

/// Checks that `question` may follow `last`, the latest round so far: it is answered and `question` is the next round.
fn check_next_question(last: Option<&Question>, question: &Question) -> Result<(), QuestionError> {
    if let Some(open) = last.filter(|last| last.is_open()) {
        return Err(QuestionError::StillOpen { round: open.round() });
    }
    let expected = last.map_or(NonZeroU32::MIN, |last| last.round().saturating_add(1));
    if question.round() != expected {
        return Err(QuestionError::WrongRound {
            expected,
            got: question.round(),
        });
    }
    Ok(())
}

/// Checks the order of the runs of task `task`: no ID twice, and only the last may be going.
fn check_runs(task: &TaskId, runs: &[Run]) -> Result<(), MemberError> {
    let mut seen = HashSet::new();
    for (index, run) in runs.iter().enumerate() {
        if !seen.insert(run.id()) {
            return Err(MemberError::DuplicateRun {
                task: task.clone(),
                run: run.id().clone(),
            });
        }
        if run.is_running() && index + 1 != runs.len() {
            return Err(MemberError::RunningNotLast {
                task: task.clone(),
                run: run.id().clone(),
            });
        }
    }
    Ok(())
}

/// Checks the rounds of questions of task `task`: they count 1, 2, 3, … and only the last may be open.
fn check_questions(task: &TaskId, questions: &[Question]) -> Result<(), MemberError> {
    let mut previous: Option<&Question> = None;
    for question in questions {
        check_next_question(previous, question).map_err(|source| MemberError::Questions {
            task: task.clone(),
            source,
        })?;
        previous = Some(question);
    }
    Ok(())
}

impl TryFrom<MemberWire> for Member {
    type Error = MemberError;

    fn try_from(wire: MemberWire) -> Result<Self, Self::Error> {
        check_runs(&wire.task, &wire.runs)?;
        check_questions(&wire.task, &wire.questions)?;
        Ok(Self {
            task: wire.task,
            agent: wire.agent,
            status: wire.status,
            runs: wire.runs,
            questions: wire.questions,
        })
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::kinds::tests::assert_keywords;
    use crate::state::QuestionState;
    use crate::state::fixtures::{assert_refused, at, inv, member, normal, nz, path, run, task, worker};
    use crate::status::RunOutcome;

    /// A queued member of task 1: `codex-1`.
    fn codex() -> Member {
        member(1, normal("codex-1"))
    }

    /// Questions of round `round` asked at `seconds`.
    fn asked(round: u32, seconds: u32) -> Question {
        Question::new(
            nz(round),
            path(&format!("/home/u/tasks/task-1/questions-{round}.md")),
            at(seconds),
        )
    }

    // ---- participants -------------------------------------------------------------------------------------------

    #[test]
    fn a_participant_has_its_tier_and_name() {
        assert_eq!(
            (normal("codex-1").tier(), normal("codex-1").name()),
            (Tier::Normal, "codex-1")
        );
        assert_eq!(
            (worker("worker-1").tier(), worker("worker-1").name()),
            (Tier::Worker, "worker-1")
        );
        assert_eq!(
            (inv("inv-opus").tier(), inv("inv-opus").name()),
            (Tier::Investigator, "inv-opus")
        );
    }

    #[test]
    fn a_participant_is_written_with_its_tier_and_name_and_reads_back() {
        for (participant, expected) in [
            (normal("codex-1"), json!({"tier": "normal", "name": "codex-1"})),
            (worker("worker-1"), json!({"tier": "worker", "name": "worker-1"})),
            (inv("inv-opus"), json!({"tier": "investigator", "name": "inv-opus"})),
        ] {
            let value = serde_json::to_value(&participant).unwrap();

            assert_eq!(value, expected);
            assert_eq!(serde_json::from_value::<Participant>(value).unwrap(), participant);
        }
    }

    #[test]
    fn the_tiers_of_participants_are_written_like_the_tiers_of_the_configuration() {
        let texts: Vec<(Tier, &str)> = Tier::ALL.iter().map(|&tier| (tier, tier.as_str())).collect();
        assert_keywords(&texts);
        for participant in [normal("a"), worker("b"), inv("c")] {
            let value = serde_json::to_value(&participant).unwrap();

            assert_eq!(value["tier"], json!(participant.tier().as_str()));
        }
    }

    #[test]
    fn a_participant_with_an_unknown_tier_or_a_bad_name_is_refused() {
        for value in [
            json!({"tier": "lead", "name": "codex-1"}),
            json!({"tier": "normal", "name": "Codex 1"}),
            json!({"tier": "normal", "name": ""}),
            json!({"tier": "normal"}),
            json!({"name": "codex-1"}),
        ] {
            assert_refused::<Participant>(&value.clone());
        }
    }

    // ---- the status ---------------------------------------------------------------------------------------------

    #[test]
    fn a_new_member_is_queued_with_no_runs_and_no_questions() {
        let member = codex();

        assert_eq!(
            (member.task(), member.agent(), member.status()),
            (&task(1), &normal("codex-1"), TaskStatus::Queued)
        );
        assert!(member.runs().is_empty() && member.questions().is_empty());
        assert!(member.current_run().is_none() && member.open_question().is_none() && member.last_session().is_none());
    }

    #[test]
    fn the_status_follows_the_table_of_task_statuses() {
        let mut member = codex();

        for next in [
            TaskStatus::Running,
            TaskStatus::Questions,
            TaskStatus::Queued,
            TaskStatus::Running,
            TaskStatus::Done,
        ] {
            member.transition(next).unwrap();

            assert_eq!(member.status(), next);
        }
    }

    #[test]
    fn a_status_change_the_table_does_not_allow_is_refused_and_changes_nothing() {
        let mut member = codex();

        let err = member.transition(TaskStatus::Done).unwrap_err();

        assert_eq!(
            err,
            InvalidTransition {
                from: TaskStatus::Queued,
                to: TaskStatus::Done
            }
        );
        assert_eq!(member.status(), TaskStatus::Queued);
        assert_eq!(
            member.transition(TaskStatus::Queued).unwrap_err().to,
            TaskStatus::Queued
        );
    }

    // ---- runs ---------------------------------------------------------------------------------------------------

    #[test]
    fn a_run_is_begun_and_is_the_current_one_until_it_ends() {
        let mut member = codex();

        member.begin_run(run(1), at(5), None).unwrap();

        assert_eq!(member.runs().len(), 1);
        assert_eq!(member.current_run().map(Run::id), Some(&run(1)));
        member
            .end_run(
                at(9),
                RunEnd::Reported {
                    outcome: RunOutcome::Done,
                },
            )
            .unwrap();
        assert!(member.current_run().is_none());
        assert_eq!(member.runs().len(), 1);
    }

    #[test]
    fn only_one_run_goes_at_a_time() {
        let mut member = codex();
        member.begin_run(run(1), at(5), None).unwrap();

        let err = member.begin_run(run(2), at(6), None).unwrap_err();

        assert_eq!(err, RunError::AlreadyRunning { run: run(1) });
        assert_eq!(member.runs().len(), 1);
    }

    #[test]
    fn a_run_id_is_not_used_twice() {
        let mut member = codex();
        member.begin_run(run(1), at(5), None).unwrap();
        member.end_run(at(6), RunEnd::Hung).unwrap();

        let err = member.begin_run(run(1), at(7), None).unwrap_err();

        assert_eq!(err, RunError::DuplicateRun { run: run(1) });
        member.begin_run(run(2), at(7), None).unwrap();
        assert_eq!(member.runs().len(), 2);
    }

    #[test]
    fn ending_needs_a_run_that_goes_and_a_time_not_before_its_start() {
        let mut member = codex();
        assert_eq!(member.end_run(at(1), RunEnd::Stopped), Err(RunError::NotRunning));
        member.begin_run(run(1), at(5), None).unwrap();

        let err = member.end_run(at(4), RunEnd::Stopped).unwrap_err();

        assert_eq!(
            err,
            RunError::EndsBeforeStart {
                run: run(1),
                started_at: at(5),
                at: at(4)
            }
        );
        assert!(member.current_run().is_some());
        member.end_run(at(5), RunEnd::Stopped).unwrap();
        assert_eq!(member.end_run(at(6), RunEnd::Stopped), Err(RunError::NotRunning));
    }

    #[test]
    fn the_session_of_the_latest_run_that_has_one_is_the_one_to_resume() {
        let mut member = codex();
        member.begin_run(run(1), at(5), None).unwrap();
        assert_eq!(member.attach_session("ses-1".parse().unwrap()), Ok(()));
        member.end_run(at(6), RunEnd::Hung).unwrap();
        member.begin_run(run(2), at(7), None).unwrap();
        assert_eq!(member.last_session().map(SessionId::as_str), Some("ses-1"));

        member.attach_session("ses-2".parse().unwrap()).unwrap();

        assert_eq!(member.last_session().map(SessionId::as_str), Some("ses-2"));
        member.end_run(at(8), RunEnd::Hung).unwrap();
        member.begin_run(run(3), at(9), Some("ses-3".parse().unwrap())).unwrap();
        assert_eq!(member.last_session().map(SessionId::as_str), Some("ses-3"));
    }

    #[test]
    fn a_session_cannot_be_attached_when_no_run_goes() {
        let mut member = codex();
        assert_eq!(
            member.attach_session("ses-1".parse().unwrap()),
            Err(RunError::NotRunning)
        );
        member.begin_run(run(1), at(5), None).unwrap();
        member.end_run(at(6), RunEnd::Hung).unwrap();

        assert_eq!(
            member.attach_session("ses-1".parse().unwrap()),
            Err(RunError::NotRunning)
        );
        assert!(member.last_session().is_none());
    }

    // ---- questions ----------------------------------------------------------------------------------------------

    #[test]
    fn rounds_of_questions_are_asked_and_answered_one_after_another() {
        let mut member = codex();

        member.ask_question(asked(1, 5)).unwrap();
        assert_eq!(member.open_question().map(Question::round), Some(nz(1)));
        member.answer_question(nz(1), at(9), AnsweredVia::Command).unwrap();
        assert!(member.open_question().is_none());
        member.ask_question(asked(2, 20)).unwrap();

        assert_eq!(member.questions().len(), 2);
        assert_eq!(member.open_question().map(Question::round), Some(nz(2)));
    }

    #[test]
    fn a_new_round_waits_for_the_answer_to_the_last() {
        let mut member = codex();
        member.ask_question(asked(1, 5)).unwrap();

        let err = member.ask_question(asked(2, 6)).unwrap_err();

        assert_eq!(err, QuestionError::StillOpen { round: nz(1) });
        assert_eq!(member.questions().len(), 1);
    }

    #[test]
    fn rounds_count_from_one_without_gaps_or_repeats() {
        let mut member = codex();
        assert_eq!(
            member.ask_question(asked(2, 5)),
            Err(QuestionError::WrongRound {
                expected: nz(1),
                got: nz(2)
            })
        );
        member.ask_question(asked(1, 5)).unwrap();
        member.answer_question(nz(1), at(6), AnsweredVia::Dashboard).unwrap();

        for wrong in [1, 3] {
            assert_eq!(
                member.ask_question(asked(wrong, 7)),
                Err(QuestionError::WrongRound {
                    expected: nz(2),
                    got: nz(wrong)
                })
            );
        }
        assert_eq!(member.questions().len(), 1);
    }

    #[test]
    fn answering_needs_an_open_round_and_a_time_not_before_the_question() {
        let mut member = codex();
        assert_eq!(
            member.answer_question(nz(1), at(6), AnsweredVia::Command),
            Err(QuestionError::NoSuchRound { round: nz(1) })
        );
        member.ask_question(asked(1, 5)).unwrap();

        let early = member.answer_question(nz(1), at(4), AnsweredVia::Command).unwrap_err();

        assert_eq!(
            early,
            QuestionError::AnswerBeforeQuestion(AnswerBeforeQuestion {
                round: nz(1),
                asked_at: at(5),
                answered_at: at(4)
            })
        );
        assert!(member.open_question().is_some());
        member.answer_question(nz(1), at(6), AnsweredVia::Command).unwrap();
        assert_eq!(
            member.answer_question(nz(1), at(7), AnsweredVia::SavedFile),
            Err(QuestionError::AlreadyAnswered { round: nz(1) })
        );
        assert_eq!(
            member.answer_question(nz(2), at(7), AnsweredVia::SavedFile),
            Err(QuestionError::NoSuchRound { round: nz(2) })
        );
        let kept = member.questions().first().map(|q| q.state().clone());
        assert_eq!(
            kept,
            Some(QuestionState::Answered {
                at: at(6),
                via: AnsweredVia::Command
            })
        );
    }

    // ---- json ---------------------------------------------------------------------------------------------------

    /// A member with a finished run, a going run and two rounds of questions.
    fn busy() -> Member {
        let mut member = codex();
        member.begin_run(run(1), at(5), Some("ses-1".parse().unwrap())).unwrap();
        member
            .end_run(
                at(9),
                RunEnd::Reported {
                    outcome: RunOutcome::Questions,
                },
            )
            .unwrap();
        member.ask_question(asked(1, 9)).unwrap();
        member.answer_question(nz(1), at(30), AnsweredVia::SavedFile).unwrap();
        member.ask_question(asked(2, 40)).unwrap();
        member.begin_run(run(2), at(31), None).unwrap();
        member.transition(TaskStatus::Running).unwrap();
        member
    }

    #[test]
    fn a_member_with_runs_and_questions_round_trips_through_json() {
        let member = busy();

        let value = serde_json::to_value(&member).unwrap();

        assert_eq!(value["task"], json!("task-20261111-153001-k3x9"));
        assert_eq!(value["agent"], json!({"tier": "normal", "name": "codex-1"}));
        assert_eq!(value["status"], json!("running"));
        assert_eq!(value["runs"].as_array().map(Vec::len), Some(2));
        assert_eq!(value["questions"].as_array().map(Vec::len), Some(2));
        assert_eq!(serde_json::from_value::<Member>(value).unwrap(), member);
    }

    #[test]
    fn a_member_without_runs_or_questions_is_written_and_read_with_them_empty() {
        let member = codex();

        let value = serde_json::to_value(&member).unwrap();
        let without = json!({"task": "task-20261111-153001-k3x9", "agent": {"tier": "normal", "name": "codex-1"}, "status": "queued"});

        assert_eq!(serde_json::from_value::<Member>(without).unwrap(), member);
        assert_eq!(serde_json::from_value::<Member>(value).unwrap(), member);
    }

    /// The member of `busy`, as JSON, to be spoiled.
    fn busy_json() -> serde_json::Value {
        serde_json::to_value(busy()).unwrap()
    }

    #[test]
    fn a_run_that_goes_but_is_not_the_last_is_refused_when_read() {
        let mut value = busy_json();
        value["runs"][0]["state"] = json!({"status": "running"});

        let wire: MemberWire = serde_json::from_value(value).unwrap();

        assert_eq!(
            Member::try_from(wire),
            Err(MemberError::RunningNotLast {
                task: task(1),
                run: run(1)
            })
        );
    }

    #[test]
    fn two_runs_with_one_id_are_refused_when_read() {
        let mut value = busy_json();
        value["runs"][1]["id"] = value["runs"][0]["id"].clone();

        let wire: MemberWire = serde_json::from_value(value).unwrap();

        assert_eq!(
            Member::try_from(wire),
            Err(MemberError::DuplicateRun {
                task: task(1),
                run: run(1)
            })
        );
    }

    #[test]
    fn rounds_that_are_out_of_order_or_open_before_the_last_are_refused_when_read() {
        let mut skipping = busy_json();
        skipping["questions"][1]["round"] = json!(3);
        let mut open_first = busy_json();
        open_first["questions"][0]["state"] = json!({"status": "open"});

        let skipping: MemberWire = serde_json::from_value(skipping).unwrap();
        let open_first: MemberWire = serde_json::from_value(open_first).unwrap();

        assert_eq!(
            Member::try_from(skipping),
            Err(MemberError::Questions {
                task: task(1),
                source: QuestionError::WrongRound {
                    expected: nz(2),
                    got: nz(3)
                }
            })
        );
        assert_eq!(
            Member::try_from(open_first),
            Err(MemberError::Questions {
                task: task(1),
                source: QuestionError::StillOpen { round: nz(1) }
            })
        );
    }

    #[test]
    fn a_member_with_an_unknown_field_or_status_is_refused() {
        let mut extra = busy_json();
        extra["extra"] = json!(1);
        let mut status = busy_json();
        status["status"] = json!("sleeping");

        assert_refused::<Member>(&extra);
        assert_refused::<Member>(&status);
    }
}
