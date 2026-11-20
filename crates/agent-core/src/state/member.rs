//! Members: an agent or investigator and the task it works in, with the task's status, runs and questions.

use std::collections::HashSet;
use std::fmt;
use std::num::NonZeroU32;

use serde::{Deserialize, Serialize};

use super::checked::CheckedMut;
use super::plan::{PlanKeeping, PlanReason};
use super::question::{AnswerBeforeQuestion, AnsweredVia, Question};
use super::renewal;
use super::run::{Run, RunEnd, RunError, RunState};
use super::strict::checked;
use super::values::{SessionId, Timestamp};
use crate::decide::ladder;
use crate::ids::{AgentName, InvestigatorName, RoundId, RunId, TaskId};
use crate::kinds::Tier;
use crate::status::{InvalidTransition, StatusMachine, TaskStatus};

/// Who works in a task: a normal agent, a worker or an investigator, by the tier of the configuration they are in.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "tier", content = "name", rename_all = "snake_case", deny_unknown_fields)]
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

    /// The participant's name as an agent name: the configuration has one namespace for all three tiers, and the bus
    /// addresses each of them by it.
    #[must_use]
    pub fn agent_name(&self) -> AgentName {
        match self {
            Self::Normal(name) | Self::Worker(name) => name.clone(),
            Self::Investigator(name) => AgentName::from(name),
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
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
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
    /// What the daemon keeps of the task's plan; `None` for a task created before plans.
    #[serde(skip_serializing_if = "Option::is_none")]
    plan: Option<PlanKeeping>,
    /// How many times the task was started in a new session again; `None` until it is, and for a task of a state
    /// written before renewals existed.
    #[serde(skip_serializing_if = "Option::is_none")]
    renewals: Option<renewal::Renewals>,
}

/// How a [`Member`] is read; see [`checked!`].
#[derive(Deserialize)]
#[serde(remote = "Member", deny_unknown_fields)]
struct MemberWire {
    /// See [`Member`].
    task: TaskId,
    /// See [`Member`].
    agent: Participant,
    /// See [`Member`].
    status: TaskStatus,
    /// See [`Member`]; a member written without runs has none.
    #[serde(default)]
    runs: Vec<Run>,
    /// See [`Member`]; a member written without questions has none.
    #[serde(default)]
    questions: Vec<Question>,
    /// See [`Member`].
    #[serde(default)]
    plan: Option<PlanKeeping>,
    /// See [`Member`]; a member written before renewals existed has none.
    #[serde(default)]
    renewals: Option<renewal::Renewals>,
}

checked!(Member, MemberWire, check_member);

/// A member inside a state or a team, to change through its operations only.
///
/// It reads like a [`Member`] (it dereferences to one) but is not a `&mut Member`: the member cannot be overwritten
/// with another one, so its task and its agent stay as the crew and the state checked them.
pub type MemberMut<'a> = CheckedMut<'a, Member>;

impl CheckedMut<'_, Member> {
    /// See [`Member::transition`].
    ///
    /// # Errors
    /// [`InvalidTransition`] when the status's table does not allow the change.
    pub fn transition(&mut self, next: TaskStatus) -> Result<(), InvalidTransition<TaskStatus>> {
        self.value.transition(next)
    }

    /// See [`Member::begin_run`].
    ///
    /// # Errors
    /// [`RunError::AlreadyRunning`] when a run is going; [`RunError::DuplicateRun`] when the task has a run with
    /// this ID.
    pub fn begin_run(&mut self, id: RunId, at: Timestamp, session: Option<SessionId>) -> Result<(), RunError> {
        self.value.begin_run(id, at, session)
    }

    /// See [`Member::renew`].
    ///
    /// # Errors
    /// [`RenewalError::NotAfterTheLastRun`] when no run of the task started before `at`.
    pub fn renew(&mut self, trouble: ladder::Trouble, at: Timestamp) -> Result<bool, renewal::RenewalError> {
        self.value.renew(trouble, at)
    }

    /// See [`Member::attach_session`].
    ///
    /// # Errors
    /// [`RunError::NotRunning`] when no run is going.
    pub fn attach_session(&mut self, session: SessionId) -> Result<(), RunError> {
        self.value.attach_session(session)
    }

    /// See [`Member::end_run`].
    ///
    /// # Errors
    /// [`RunError::NotRunning`] when no run is going; [`RunError::EndsBeforeStart`] when `at` is before the run
    /// started.
    pub fn end_run(&mut self, at: Timestamp, end: RunEnd) -> Result<(), RunError> {
        self.value.end_run(at, end)
    }

    /// See [`Member::ask_question`].
    ///
    /// # Errors
    /// [`QuestionError`]: the last questions are open, their number is not the next one, or the numbers are used up.
    pub fn ask_question(&mut self, question: Question) -> Result<(), QuestionError> {
        self.value.ask_question(question)
    }

    /// See [`Member::answer_question`].
    ///
    /// # Errors
    /// [`QuestionError::NoSuchQuestions`], [`QuestionError::AlreadyAnswered`], or
    /// [`QuestionError::AnswerBeforeQuestion`].
    pub fn answer_question(
        &mut self,
        number: NonZeroU32,
        at: Timestamp,
        via: AnsweredVia,
    ) -> Result<(), QuestionError> {
        self.value.answer_question(number, at, via)
    }

    /// See [`Member::send_back`].
    pub fn send_back(&mut self, reason: PlanReason) {
        self.value.send_back(reason);
    }

    /// See [`Member::plan_asked`].
    pub fn plan_asked(&mut self, question: NonZeroU32) {
        self.value.plan_asked(question);
    }

    /// See [`Member::take_plan_reason`].
    pub fn take_plan_reason(&mut self) -> Option<PlanReason> {
        self.value.take_plan_reason()
    }

    /// See [`Member::send_back_once_more`].
    pub fn send_back_once_more(&mut self) {
        self.value.send_back_once_more();
    }

    /// See [`Member::plan_answered`].
    pub fn plan_answered(&mut self, question: NonZeroU32) {
        self.value.plan_answered(question);
    }

    /// See [`Member::take_unread_plan_answers`].
    pub fn take_unread_plan_answers(&mut self) -> Option<NonZeroU32> {
        self.value.take_unread_plan_answers()
    }

    /// See [`Member::release_plan`].
    pub fn release_plan(&mut self) {
        self.value.release_plan();
    }
}

/// Why questions could not be added or answered.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum QuestionError {
    /// The last questions are still open: answer them first.
    #[error("questions number {number} are still open")]
    StillOpen {
        /// The number of the open questions.
        number: NonZeroU32,
    },
    /// Questions are numbered 1, 2, 3, …: this is not the next number.
    #[error("the next questions number is {expected}, not {got}")]
    WrongNumber {
        /// The next number.
        expected: NonZeroU32,
        /// The refused number.
        got: NonZeroU32,
    },
    /// The last possible number was asked and answered: there is no number after it.
    #[error("questions number {last} was the last possible number; the task cannot ask more")]
    NumbersExhausted {
        /// The last number, the largest a questions file can have.
        last: NonZeroU32,
    },
    /// The task has no such questions.
    #[error("the task has no questions number {number}")]
    NoSuchQuestions {
        /// The number asked for.
        number: NonZeroU32,
    },
    /// The questions were answered already.
    #[error("questions number {number} are already answered")]
    AlreadyAnswered {
        /// The number of the questions.
        number: NonZeroU32,
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
    /// The renewals do not fit the runs.
    #[error("task {task}: its renewals do not fit its runs")]
    Renewal {
        /// The task.
        task: TaskId,
        /// What is wrong.
        #[source]
        source: renewal::RenewalError,
    },
}

impl Member {
    /// A member whose task is queued and has no runs yet, created by this version: it keeps a plan.
    #[must_use]
    pub fn new(task: TaskId, agent: Participant) -> Self {
        Self {
            task,
            agent,
            status: TaskStatus::Queued,
            runs: Vec::new(),
            questions: Vec::new(),
            plan: Some(PlanKeeping::default()),
            renewals: None,
        }
    }

    /// A member of a task created before this version kept plans: its runs are never sent back for a plan.
    #[must_use]
    pub const fn without_plan(task: TaskId, agent: Participant) -> Self {
        Self {
            task,
            agent,
            status: TaskStatus::Queued,
            runs: Vec::new(),
            questions: Vec::new(),
            plan: None,
            renewals: None,
        }
    }

    getter!("The task's ID.", pub const fn task(&self) -> &TaskId = &self.task);

    getter!("Who works in the task.", pub const fn agent(&self) -> &Participant = &self.agent);

    getter!("Where the task is in its life.", pub const fn status(&self) -> TaskStatus = self.status);

    getter!("The runs, oldest first.", pub fn runs(&self) -> &[Run] = &self.runs);

    getter!(
        "How many times the task was started in a new session again; `None` until it was.",
        pub const fn renewals(&self) -> Option<&renewal::Renewals> = self.renewals.as_ref()
    );

    getter!("The questions asked, oldest first.", pub fn questions(&self) -> &[Question] = &self.questions);

    getter!("The run that is going, if one is.", pub fn current_run(&self) -> Option<&Run> = self.runs.last().filter(|run| run.is_running()));

    getter!(
        "The questions that wait for the human, if there are any.",
        pub fn open_question(&self) -> Option<&Question> = self.questions.last().filter(|question| question.is_open())
    );

    getter!(
        "The session of the latest run since the task was last started anew that has one: the one a new run resumes. \
         No session of a run before that renewal is resumed.",
        pub fn last_session(&self) -> Option<&SessionId> = renewal::runs_since(
            &self.runs,
            self.renewals.as_ref().map(renewal::Renewals::at)
        )
        .iter()
        .rev()
        .find_map(Run::session)
    );

    getter!(
        "What the task keeps of its plan; `None` for a task created before plans.",
        pub const fn plan(&self) -> Option<&PlanKeeping> = self.plan.as_ref()
    );

    /// Counts a send-back of the task for its plan, which its next run delivers with `reason`; nothing for a task
    /// created before plans.
    pub fn send_back(&mut self, reason: PlanReason) {
        if let Some(keeping) = &mut self.plan {
            keeping.send_back(reason);
        }
    }

    /// Records that the human was asked about the task's plan in questions `question`; nothing for a task created
    /// before plans.
    pub fn plan_asked(&mut self, question: NonZeroU32) {
        if let Some(keeping) = &mut self.plan {
            keeping.asked(question);
        }
    }

    /// Counts one send-back the human granted beyond `[plan] bounces`; nothing for a task created before plans.
    pub fn grant_another_send_back(&mut self) {
        if let Some(keeping) = &mut self.plan {
            keeping.grant_another();
        }
    }

    /// The reason of the send-back that waits for the next run, taken out: the run that starts is told it, and no later
    /// run is. `None` when nothing waits, and for a task created before plans.
    pub fn take_plan_reason(&mut self) -> Option<PlanReason> {
        self.plan.as_mut().and_then(PlanKeeping::take_pending)
    }

    /// The human said to send the run back once more: one more send-back is granted and counted, and the plan's
    /// stage is open again, so that a run that still ends against the plan asks the human anew; nothing for a task
    /// created before plans.
    pub fn send_back_once_more(&mut self) {
        if let Some(keeping) = &mut self.plan {
            keeping.once_more();
        }
    }

    /// The human answered questions `question` about the task's plan: the next run is to be given the answers, which
    /// no run asked for; nothing for a task created before plans.
    pub fn plan_answered(&mut self, question: NonZeroU32) {
        if let Some(keeping) = &mut self.plan {
            keeping.answered(question);
        }
    }

    /// The questions about the plan whose answers no run has been given yet, taken out: the run that starts is given
    /// them, and no later run is.
    pub fn take_unread_plan_answers(&mut self) -> Option<NonZeroU32> {
        self.plan.as_mut().and_then(PlanKeeping::take_unread_answers)
    }

    /// The human said to go on as it is: no run of the task is sent back for its plan again.
    pub fn release_plan(&mut self) {
        if let Some(keeping) = &mut self.plan {
            keeping.release();
        }
    }

    /// Changes the task's status.
    ///
    /// # Errors
    /// [`InvalidTransition`] when the status's table does not allow the change.
    pub fn transition(&mut self, next: TaskStatus) -> Result<(), InvalidTransition<TaskStatus>> {
        self.status = self.status.transition_to(next)?;
        Ok(())
    }

    /// Records that the task is started anew, in a new session, after `trouble` made the runs of the session it had go
    /// bad: the runs before the new one are not counted or resumed any more, and the new session's run starts in no
    /// session. A worker or a reviewer is never started anew, and a task whose session had no run to go bad from.
    ///
    /// Returns whether the task was started anew: `false` when nothing changed.
    ///
    /// # Errors
    /// [`RenewalError::NotAfterTheLastRun`] when no run of the task started before `at`.
    pub fn renew(&mut self, trouble: ladder::Trouble, at: Timestamp) -> Result<bool, renewal::RenewalError> {
        if !matches!(self.agent.tier(), Tier::Normal | Tier::Investigator) || self.runs.is_empty() {
            return Ok(false);
        }
        let Some(last) = self.runs.last() else {
            return Ok(false);
        };
        if last.started_at().as_utc() >= at.as_utc() {
            return Err(renewal::RenewalError::NotAfterTheLastRun { at, started_at: at });
        }
        let mut renewals = self.renewals.unwrap_or(renewal::Renewals::first(at));
        if self.renewals.is_some() {
            renewals.record(at);
        }
        let _ = trouble;
        self.renewals = Some(renewals);
        Ok(true)
    }

    /// Records that a run has started.
    ///
    /// # Errors
    /// [`RunError::AlreadyRunning`] when a run is going; [`RunError::DuplicateRun`] when the task has a run with
    /// this ID.
    pub fn begin_run(&mut self, id: RunId, at: Timestamp, session: Option<SessionId>) -> Result<(), RunError> {
        self.begin_run_for(id, at, session, Vec::new())
    }

    /// Records that a run has started that works for `rounds`; whether they are rounds of the team is for the caller to
    /// have checked.
    ///
    /// # Errors
    /// As [`Member::begin_run`]; [`RunError::RoundTwice`] when `rounds` names a round twice.
    pub(super) fn begin_run_for(
        &mut self,
        id: RunId,
        at: Timestamp,
        session: Option<SessionId>,
        rounds: Vec<RoundId>,
    ) -> Result<(), RunError> {
        if let Some(run) = self.current_run() {
            return Err(RunError::AlreadyRunning { run: run.id().clone() });
        }
        if self.runs.iter().any(|run| run.id() == &id) {
            return Err(RunError::DuplicateRun { run: id });
        }
        let run = Run::started_for(id, at, session, rounds)?;
        // The run that carries a renewal begins the new session: it resumes nothing.
        let begins_a_new_session = self
            .renewals
            .as_ref()
            .is_some_and(|renewals| at.as_utc() >= renewals.at().as_utc());
        if begins_a_new_session && run.session().is_some() {
            return Err(RunError::SessionOfANewSession);
        }
        self.runs.push(run);
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

    /// See [`Member::renew`], and how many of the latest runs went bad in a row since the task was started anew.
    #[must_use]
    pub fn bad_runs_since_renewal(&self) -> u32 {
        let Some(end) = self.runs.last().and_then(|run| match run.state() {
            RunState::Ended { end, .. } => Some(end),
            RunState::Running => None,
        }) else {
            return 0;
        };
        renewal::trouble_of(end).map_or(0, |trouble| {
            renewal::bad_runs_in_a_row(&self.runs, self.renewals.as_ref().map(renewal::Renewals::at), trouble)
        })
    }

    /// The run that is going, to change it.
    fn current_run_mut(&mut self) -> Result<&mut Run, RunError> {
        self.runs
            .last_mut()
            .filter(|run| run.is_running())
            .ok_or(RunError::NotRunning)
    }

    /// Records that questions were put to the human.
    ///
    /// # Errors
    /// [`QuestionError::StillOpen`] when the last questions are open; [`QuestionError::WrongNumber`] when the number is
    /// not the next one; [`QuestionError::NumbersExhausted`] when the last number is the largest there is.
    pub fn ask_question(&mut self, question: Question) -> Result<(), QuestionError> {
        check_next_question(self.questions.last(), &question)?;
        self.questions.push(question);
        Ok(())
    }

    /// Records the human's answer to questions.
    ///
    /// # Errors
    /// [`QuestionError::NoSuchQuestions`], [`QuestionError::AlreadyAnswered`], or
    /// [`QuestionError::AnswerBeforeQuestion`].
    pub fn answer_question(
        &mut self,
        number: NonZeroU32,
        at: Timestamp,
        via: AnsweredVia,
    ) -> Result<(), QuestionError> {
        let question = self
            .questions
            .iter_mut()
            .find(|question| question.number() == number)
            .ok_or(QuestionError::NoSuchQuestions { number })?;
        if !question.is_open() {
            return Err(QuestionError::AlreadyAnswered { number });
        }
        question.answer(at, via)?;
        Ok(())
    }
}

/// Checks that `question` may follow `last`, the latest questions so far: they are answered and `question` has the next
/// number.
fn check_next_question(last: Option<&Question>, question: &Question) -> Result<(), QuestionError> {
    if let Some(open) = last.filter(|last| last.is_open()) {
        return Err(QuestionError::StillOpen { number: open.number() });
    }
    let expected = match last {
        None => NonZeroU32::MIN,
        Some(last) => last
            .number()
            .checked_add(1)
            .ok_or(QuestionError::NumbersExhausted { last: last.number() })?,
    };
    if question.number() != expected {
        return Err(QuestionError::WrongNumber {
            expected,
            got: question.number(),
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

/// Checks the questions of task `task`: they are numbered 1, 2, 3, … and only the last may be open.
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

/// Checks the run, question and renewal invariants of a complete member.
fn check_member(member: &Member) -> Result<(), MemberError> {
    check_runs(&member.task, &member.runs)?;
    check_questions(&member.task, &member.questions)?;
    if let Some(renewals) = &member.renewals {
        renewal::check_renewals(renewals, &member.runs).map_err(|source| MemberError::Renewal {
            task: member.task.clone(),
            source,
        })?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::kinds::tests::assert_keywords;
    use crate::state::fixtures::{assert_refused, at, inv, item, member, normal, nz, path, run, task, worker};
    use crate::state::{PlanStage, QuestionState};
    use crate::status::RunOutcome;

    /// A member's serialized shape, used to construct states that the public constructors refuse.
    #[derive(serde::Deserialize)]
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
        /// See [`Member`].
        #[serde(default)]
        plan: Option<PlanKeeping>,
    }

    impl TryFrom<MemberWire> for Member {
        type Error = MemberError;

        fn try_from(wire: MemberWire) -> Result<Self, Self::Error> {
            let member = Self {
                task: wire.task,
                agent: wire.agent,
                status: wire.status,
                runs: wire.runs,
                questions: wire.questions,
                plan: wire.plan,
                renewals: None,
            };
            check_member(&member)?;
            Ok(member)
        }
    }

    /// A queued member of task 1: `codex-1`.
    fn codex() -> Member {
        member(1, normal("codex-1"))
    }

    /// Questions number `number` asked at `seconds`.
    fn asked(number: u32, seconds: u32) -> Question {
        Question::new(
            nz(number),
            path(&format!("/home/u/tasks/task-1/questions-{number}.md")),
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
        assert_eq!(member.open_question().map(Question::number), Some(nz(1)));
        member.answer_question(nz(1), at(9), AnsweredVia::Command).unwrap();
        assert!(member.open_question().is_none());
        member.ask_question(asked(2, 20)).unwrap();

        assert_eq!(member.questions().len(), 2);
        assert_eq!(member.open_question().map(Question::number), Some(nz(2)));
    }

    #[test]
    fn a_new_round_waits_for_the_answer_to_the_last() {
        let mut member = codex();
        member.ask_question(asked(1, 5)).unwrap();

        let err = member.ask_question(asked(2, 6)).unwrap_err();

        assert_eq!(err, QuestionError::StillOpen { number: nz(1) });
        assert_eq!(member.questions().len(), 1);
    }

    #[test]
    fn rounds_count_from_one_without_gaps_or_repeats() {
        let mut member = codex();
        assert_eq!(
            member.ask_question(asked(2, 5)),
            Err(QuestionError::WrongNumber {
                expected: nz(1),
                got: nz(2)
            })
        );
        member.ask_question(asked(1, 5)).unwrap();
        member.answer_question(nz(1), at(6), AnsweredVia::Dashboard).unwrap();

        for wrong in [1, 3] {
            assert_eq!(
                member.ask_question(asked(wrong, 7)),
                Err(QuestionError::WrongNumber {
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
            Err(QuestionError::NoSuchQuestions { number: nz(1) })
        );
        member.ask_question(asked(1, 5)).unwrap();

        let early = member.answer_question(nz(1), at(4), AnsweredVia::Command).unwrap_err();

        assert_eq!(
            early,
            QuestionError::AnswerBeforeQuestion(AnswerBeforeQuestion {
                number: nz(1),
                asked_at: at(5),
                answered_at: at(4)
            })
        );
        assert!(member.open_question().is_some());
        member.answer_question(nz(1), at(6), AnsweredVia::Command).unwrap();
        assert_eq!(
            member.answer_question(nz(1), at(7), AnsweredVia::SavedFile),
            Err(QuestionError::AlreadyAnswered { number: nz(1) })
        );
        assert_eq!(
            member.answer_question(nz(2), at(7), AnsweredVia::SavedFile),
            Err(QuestionError::NoSuchQuestions { number: nz(2) })
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

        // A member written before plans keeps none; the one written now keeps theirs.
        assert_eq!(
            serde_json::from_value::<Member>(without).unwrap(),
            Member::without_plan(task(1), normal("codex-1"))
        );
        assert_eq!(serde_json::from_value::<Member>(value).unwrap(), member);
    }

    /// The member of `busy`, as JSON, to be spoiled.
    fn busy_json() -> serde_json::Value {
        serde_json::to_value(busy()).unwrap()
    }

    #[test]
    fn the_answers_about_a_plan_wait_for_one_run_and_survive_json() {
        let mut member = Member::new(task(1), normal("codex-1"));

        member.plan_answered(nz(3));

        assert_eq!(member.plan().and_then(PlanKeeping::unread_answers), Some(nz(3)));
        let value = serde_json::to_value(&member).unwrap();
        let mut read: Member = serde_json::from_value(value).unwrap();
        assert_eq!(read, member);
        assert_eq!(read.take_unread_plan_answers(), Some(nz(3)));
        assert_eq!(read.take_unread_plan_answers(), None);
    }

    #[test]
    fn a_task_created_before_plans_has_no_unread_answers_about_one() {
        let mut member = Member::without_plan(task(1), normal("codex-1"));

        member.plan_answered(nz(3));

        assert_eq!(member.take_unread_plan_answers(), None);
    }

    #[test]
    fn the_reason_that_waits_is_taken_by_one_run_only() {
        let mut member = Member::new(task(1), normal("codex-1"));
        member.send_back(PlanReason::Unfinished {
            steps: vec![item("1.2")],
        });

        let first = member.take_plan_reason();
        let second = member.take_plan_reason();

        assert_eq!(
            first,
            Some(PlanReason::Unfinished {
                steps: vec![item("1.2")]
            })
        );
        assert_eq!(second, None);
        assert_eq!(member.plan().map(PlanKeeping::sent_back), Some(1));
    }

    #[test]
    fn sending_back_once_more_grants_and_counts_one_send_back_and_opens_the_plan_again() {
        let mut member = Member::new(task(1), normal("codex-1"));
        member.send_back(PlanReason::Missing);
        assert_eq!(member.take_plan_reason(), Some(PlanReason::Missing));
        member.plan_asked(nz(2));

        member.send_back_once_more();

        let keeping = member.plan().expect("the member keeps its plan");
        assert_eq!(
            (
                keeping.sent_back(),
                keeping.granted(),
                keeping.stage(),
                keeping.pending()
            ),
            (2, 1, PlanStage::Open, None)
        );
    }

    #[test]
    fn a_task_created_before_plans_has_no_reason_and_nothing_to_send_back_once_more() {
        let mut member = Member::without_plan(task(1), normal("codex-1"));

        member.send_back_once_more();

        assert_eq!(member.take_plan_reason(), None);
        assert_eq!(member.plan(), None);
    }

    #[test]
    fn a_member_written_before_the_plans_keeps_none_and_loads() {
        let wire = json!({"task": "task-20261111-153001-k3x9", "agent": {"tier": "normal", "name": "codex-1"}, "status": "queued"});

        let member: Member = serde_json::from_value(wire).unwrap();

        assert_eq!(member.plan(), None);
    }

    #[test]
    fn a_new_member_keeps_its_plan_with_nothing_sent_back_and_writes_it_with_the_member() {
        let member = Member::new(task(1), normal("codex-1"));

        let value = serde_json::to_value(&member).unwrap();

        assert_eq!(value["plan"], json!({"sent_back": 0, "granted": 0, "stage": "open"}));
        assert_eq!(serde_json::from_value::<Member>(value).unwrap(), member);
    }

    #[test]
    fn a_member_sent_back_keeps_the_count_and_the_reason_until_a_run_takes_it() {
        let mut member = Member::new(task(1), normal("codex-1"));
        let reason = PlanReason::Missing;

        member.send_back(reason.clone());

        let keeping = member.plan().expect("the member keeps its plan");
        assert_eq!(keeping.sent_back(), 1);
        assert_eq!(keeping.stage(), PlanStage::Open);
        assert_eq!(keeping.pending(), Some(&reason));
        let value = serde_json::to_value(&member).unwrap();
        assert_eq!(serde_json::from_value::<Member>(value).unwrap(), member);
    }

    #[test]
    fn a_task_created_before_plans_never_keeps_one() {
        let mut member = Member::without_plan(task(1), normal("codex-1"));

        member.send_back(PlanReason::Missing);
        member.plan_asked(nz(1));
        member.grant_another_send_back();
        member.release_plan();

        assert_eq!(member.plan(), None);
    }

    #[test]
    fn the_human_gives_back_one_more_send_back_and_releases_the_plan_for_good() {
        let mut member = Member::new(task(1), normal("codex-1"));
        member.ask_question(asked(1, 10)).unwrap();

        member.grant_another_send_back();
        let keeping = member.plan().expect("the member keeps its plan");
        assert_eq!(keeping.granted(), 1);
        assert_eq!(keeping.stage(), PlanStage::Open);

        member.answer_question(nz(1), at(20), AnsweredVia::Command).unwrap();
        member.release_plan();
        let keeping = member.plan().expect("the member keeps its plan");
        assert_eq!(keeping.stage(), PlanStage::Released);
        assert_eq!(keeping.pending(), None);
    }

    #[test]
    fn a_question_about_a_plan_moves_the_stage_while_it_waits() {
        let mut member = Member::new(task(1), normal("codex-1"));

        member.plan_asked(nz(4));

        assert_eq!(
            member.plan().map(PlanKeeping::stage),
            Some(PlanStage::Asked { question: nz(4) })
        );
        let reason = PlanReason::Missing;
        member.send_back(reason.clone());
        assert_eq!(member.plan().and_then(PlanKeeping::pending), Some(&reason));
    }

    #[test]
    fn the_reason_that_waits_for_a_run_survives_json() {
        for reason in [
            PlanReason::Missing,
            PlanReason::Unreadable {
                line: Some(nz(3)),
                detail: "invalid stage number".to_owned(),
            },
            PlanReason::Unfinished {
                steps: vec![item("2.1")],
            },
            PlanReason::LostStep {
                steps: vec![item("1.1")],
            },
        ] {
            let mut member = Member::new(task(1), normal("codex-1"));
            member.send_back(reason.clone());
            member.plan_asked(nz(2));

            let text = serde_json::to_string(&member).unwrap();
            let back: Member = serde_json::from_str(&text).unwrap();

            assert_eq!(back, member, "{text}");
            assert_eq!(back.plan().and_then(PlanKeeping::pending), Some(&reason));
        }
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
        skipping["questions"][1]["number"] = json!(3);
        let mut open_first = busy_json();
        open_first["questions"][0]["state"] = json!({"status": "open"});

        let skipping: MemberWire = serde_json::from_value(skipping).unwrap();
        let open_first: MemberWire = serde_json::from_value(open_first).unwrap();

        assert_eq!(
            Member::try_from(skipping),
            Err(MemberError::Questions {
                task: task(1),
                source: QuestionError::WrongNumber {
                    expected: nz(2),
                    got: nz(3)
                }
            })
        );
        assert_eq!(
            Member::try_from(open_first),
            Err(MemberError::Questions {
                task: task(1),
                source: QuestionError::StillOpen { number: nz(1) }
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

    // ---- the last possible number -------------------------------------------------------------------------------

    #[test]
    fn after_the_last_possible_round_has_been_answered_no_further_round_is_accepted() {
        let mut last = asked(u32::MAX, 10);
        last.answer(at(20), AnsweredVia::Command).unwrap();

        let err = check_next_question(Some(&last), &asked(u32::MAX, 30)).unwrap_err();

        assert_eq!(err, QuestionError::NumbersExhausted { last: nz(u32::MAX) });
    }

    #[test]
    fn the_round_before_the_last_possible_one_is_still_followed_by_it() {
        let mut before = asked(u32::MAX - 1, 10);
        before.answer(at(20), AnsweredVia::Command).unwrap();

        assert_eq!(check_next_question(Some(&before), &asked(u32::MAX, 30)), Ok(()));
        assert_eq!(
            check_next_question(Some(&before), &asked(u32::MAX - 1, 30)),
            Err(QuestionError::WrongNumber {
                expected: nz(u32::MAX),
                got: nz(u32::MAX - 1)
            })
        );
    }
}
