//! Questions: a questions file a task asked the human to answer, and whether it was answered.
//!
//! The questions and the answers are in the file ([`crate::questions`]); the state records that it was asked and
//! answered, when, and through which channel.

use std::num::NonZeroU32;

use serde::{Deserialize, Serialize};

use super::values::Timestamp;
use crate::config::AbsolutePath;

/// One round of questions of a task: `questions-<round>.md`, asked at a time, still open or answered.
///
/// Made by [`Question::new`] and answered by [`Question::answer`] (through
/// [`Member::answer_question`](super::Member::answer_question)), so an answer is never earlier than the question;
/// reading a state checks that too.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "QuestionWire")]
pub struct Question {
    /// The round, counted from 1 for each task: the number in the file's name.
    round: NonZeroU32,
    /// The questions file.
    file: AbsolutePath,
    /// When the questions were put to the human.
    asked_at: Timestamp,
    /// Whether the human has answered.
    state: QuestionState,
}

/// What a [`Question`] is written as, before its checks.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct QuestionWire {
    /// See [`Question`].
    round: NonZeroU32,
    /// See [`Question`].
    file: AbsolutePath,
    /// See [`Question`].
    asked_at: Timestamp,
    /// See [`Question`].
    state: QuestionState,
}

/// Whether the human has answered a [`Question`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum QuestionState {
    /// The human has not answered yet.
    Open,
    /// The human answered.
    Answered {
        /// When the answers arrived.
        at: Timestamp,
        /// Through which channel they arrived.
        via: AnsweredVia,
    },
}

/// The channel an answer came through.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AnsweredVia {
    /// `agent answer`, in the editor or with `--file`.
    Command,
    /// The file was saved with the `**ANSWERED**` marker.
    SavedFile,
    /// The dashboard's form.
    Dashboard,
}

/// An answer that is earlier than its question.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("the answer at {answered_at} is earlier than the question of round {round}, asked at {asked_at}")]
pub struct AnswerBeforeQuestion {
    /// The round.
    pub round: NonZeroU32,
    /// When the questions were asked.
    pub asked_at: Timestamp,
    /// The refused time of the answer.
    pub answered_at: Timestamp,
}

impl Question {
    /// Questions that have just been asked.
    #[must_use]
    pub const fn new(round: NonZeroU32, file: AbsolutePath, asked_at: Timestamp) -> Self {
        Self {
            round,
            file,
            asked_at,
            state: QuestionState::Open,
        }
    }

    /// The round, counted from 1 for each task.
    #[must_use]
    pub const fn round(&self) -> NonZeroU32 {
        self.round
    }

    /// The questions file.
    #[must_use]
    pub const fn file(&self) -> &AbsolutePath {
        &self.file
    }

    /// When the questions were put to the human.
    #[must_use]
    pub const fn asked_at(&self) -> Timestamp {
        self.asked_at
    }

    /// Whether the human has answered.
    #[must_use]
    pub const fn state(&self) -> &QuestionState {
        &self.state
    }

    /// Whether the human has not answered yet.
    #[must_use]
    pub const fn is_open(&self) -> bool {
        matches!(self.state, QuestionState::Open)
    }

    /// Records the human's answer.
    ///
    /// # Errors
    /// [`AnswerBeforeQuestion`] when `at` is earlier than the time the questions were asked.
    pub fn answer(&mut self, at: Timestamp, via: AnsweredVia) -> Result<(), AnswerBeforeQuestion> {
        let _ = (at, via);
        Ok(())
    }
}

impl TryFrom<QuestionWire> for Question {
    type Error = AnswerBeforeQuestion;

    fn try_from(wire: QuestionWire) -> Result<Self, Self::Error> {
        Ok(Self {
            round: wire.round,
            file: wire.file,
            asked_at: wire.asked_at,
            state: wire.state,
        })
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::state::fixtures::{at, nz, path};

    /// Questions of round 1 asked at 5 seconds.
    fn asked() -> Question {
        Question::new(nz(1), path("/home/u/agents/tasks/task-1/questions-1.md"), at(5))
    }

    #[test]
    fn new_questions_are_open_and_have_what_they_were_given() {
        let question = asked();

        assert_eq!(question.round(), nz(1));
        assert_eq!(
            question.file().as_path().to_str(),
            Some("/home/u/agents/tasks/task-1/questions-1.md")
        );
        assert_eq!(question.asked_at(), at(5));
        assert_eq!(question.state(), &QuestionState::Open);
        assert!(question.is_open());
    }

    #[test]
    fn answering_records_when_and_how() {
        let mut question = asked();

        question.answer(at(60), AnsweredVia::SavedFile).unwrap();

        assert_eq!(
            question.state(),
            &QuestionState::Answered {
                at: at(60),
                via: AnsweredVia::SavedFile
            }
        );
        assert!(!question.is_open());
    }

    #[test]
    fn an_answer_at_the_moment_of_the_question_is_allowed() {
        let mut question = asked();

        question.answer(at(5), AnsweredVia::Command).unwrap();

        assert!(!question.is_open());
    }

    #[test]
    fn an_answer_before_the_question_is_refused_and_the_question_stays_open() {
        let mut question = asked();

        let err = question.answer(at(4), AnsweredVia::Dashboard).unwrap_err();

        assert_eq!(
            err,
            AnswerBeforeQuestion {
                round: nz(1),
                asked_at: at(5),
                answered_at: at(4)
            }
        );
        assert!(question.is_open());
    }

    #[test]
    fn the_channels_have_their_texts() {
        for (via, text) in [
            (AnsweredVia::Command, "command"),
            (AnsweredVia::SavedFile, "saved_file"),
            (AnsweredVia::Dashboard, "dashboard"),
        ] {
            assert_eq!(serde_json::to_value(via).unwrap(), json!(text));
            assert_eq!(serde_json::from_value::<AnsweredVia>(json!(text)).unwrap(), via);
        }
        assert!(serde_json::from_value::<AnsweredVia>(json!("telepathy")).is_err());
    }

    #[test]
    fn open_and_answered_questions_round_trip_through_json() {
        let open = asked();
        let mut answered = asked();
        answered.answer(at(60), AnsweredVia::Command).unwrap();

        assert_eq!(
            serde_json::to_value(&open).unwrap(),
            json!({
                "round": 1,
                "file": "/home/u/agents/tasks/task-1/questions-1.md",
                "asked_at": "2026-11-11T19:15:05Z",
                "state": {"status": "open"}
            })
        );
        assert_eq!(
            serde_json::to_value(&answered).unwrap()["state"],
            json!({"status": "answered", "at": "2026-11-11T19:16:00Z", "via": "command"})
        );
        for question in [open, answered] {
            let value = serde_json::to_value(&question).unwrap();
            assert_eq!(serde_json::from_value::<Question>(value).unwrap(), question);
        }
    }

    #[test]
    fn questions_answered_before_they_were_asked_are_refused_when_read() {
        let value = json!({
            "round": 1,
            "file": "/f/questions-1.md",
            "asked_at": "2026-11-11T19:15:05Z",
            "state": {"status": "answered", "at": "2026-11-11T19:15:04Z", "via": "command"}
        });

        assert!(serde_json::from_value::<Question>(value).is_err());
    }

    #[test]
    fn questions_with_round_zero_a_relative_file_or_an_unknown_field_are_refused() {
        let good =
            json!({"round": 1, "file": "/f/q.md", "asked_at": "2026-11-11T19:15:05Z", "state": {"status": "open"}});
        serde_json::from_value::<Question>(good.clone()).unwrap();
        for (key, bad) in [
            ("round", json!(0)),
            ("round", json!(-1)),
            ("file", json!("questions-1.md")),
            ("file", json!("")),
            ("extra", json!(true)),
        ] {
            let mut value = good.clone();
            value[key] = bad;

            assert!(serde_json::from_value::<Question>(value.clone()).is_err(), "{value}");
        }
    }
}
