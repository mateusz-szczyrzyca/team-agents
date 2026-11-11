//! Questions files (`questions-<round>.md`): parsing, answering, validation and rendering (see `docs/state.md`).
//!
//! When a run ends with [`RunOutcome::Questions`](crate::status::RunOutcome::Questions), the agent leaves a file of
//! numbered questions (`## N. Title`), each with options or context, a `**Recommended:**` part and a `**Verdict:**`
//! line. The harness inserts the answering instructions (rendered from a template in `prompts/`) under the `# ...`
//! title. The human answers each question after `**Verdict:**` — `ok` accepts the recommendation, any other text is
//! the human's own answer — and puts `**ANSWERED**` on the last line when done.

use std::collections::BTreeMap;
use std::fmt;
use std::num::{NonZeroU32, NonZeroUsize, ParseIntError};
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::kinds::keyword_enum;

/// Largest questions file accepted, in bytes: a bound on untrusted input, far above any real file.
pub const MAX_QUESTIONS_BYTES: usize = 1 << 20;

/// The marker on the last non-empty line that tells the harness all questions are answered and ready to send.
pub const ANSWERED_MARKER: &str = "**ANSWERED**";

/// The word a human writes after `**Verdict:**` to accept a question's recommendation.
const OK_KEYWORD: &str = "ok";

/// Why a text cannot be an [`OwnAnswer`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, thiserror::Error)]
pub enum InvalidOwnAnswer {
    /// The text is empty or only whitespace.
    #[error("an own answer must not be empty")]
    Empty,
    /// The text is `ok` (in any case), which means [`Verdict::AcceptRecommendation`].
    #[error("\"ok\" accepts the recommendation; use Verdict::AcceptRecommendation")]
    ReservedOk,
}

/// The human's own answer text for a question: trimmed, never empty, and never bare `ok`.
///
/// The inner field is private, so an invalid [`OwnAnswer`] cannot be constructed directly:
///
/// ```compile_fail,E0603
/// let bad = agent_core::questions::OwnAnswer(String::new());
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct OwnAnswer(String);

impl OwnAnswer {
    /// The trimmed text of the answer.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for OwnAnswer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl AsRef<str> for OwnAnswer {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl FromStr for OwnAnswer {
    type Err = InvalidOwnAnswer;

    fn from_str(_text: &str) -> Result<Self, Self::Err> {
        Err(InvalidOwnAnswer::Empty)
    }
}

impl TryFrom<String> for OwnAnswer {
    type Error = InvalidOwnAnswer;

    fn try_from(text: String) -> Result<Self, Self::Error> {
        text.parse()
    }
}

impl From<OwnAnswer> for String {
    fn from(answer: OwnAnswer) -> Self {
        answer.0
    }
}

/// Why a text is not a valid [`Verdict`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, thiserror::Error)]
#[error("a verdict must not be empty")]
pub struct EmptyVerdict;

/// What the human writes after `**Verdict:**` for a question.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub enum Verdict {
    /// `ok` (any ASCII case): accepts the question's `**Recommended:**` text.
    AcceptRecommendation,
    /// The human's own answer text.
    Own(OwnAnswer),
}

impl Verdict {
    /// Builds a [`Verdict::Own`] from `text`.
    ///
    /// # Errors
    /// [`InvalidOwnAnswer::Empty`] when `text` is blank, or [`InvalidOwnAnswer::ReservedOk`] when `text` is `ok`.
    pub fn own(text: &str) -> Result<Self, InvalidOwnAnswer> {
        text.parse::<OwnAnswer>().map(Self::Own)
    }

    /// Parses the text after `**Verdict:**`: `None` when blank, [`Self::AcceptRecommendation`] for `ok` (any case),
    /// and [`Self::Own`] for any other text.
    #[must_use]
    pub fn parse_optional(_text: &str) -> Option<Self> {
        None
    }

    /// The text written after `**Verdict:**` when rendering a questions file.
    #[must_use]
    pub fn as_str(&self) -> &str {
        match self {
            Self::AcceptRecommendation => OK_KEYWORD,
            Self::Own(answer) => answer.as_str(),
        }
    }
}

impl fmt::Display for Verdict {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for Verdict {
    type Err = EmptyVerdict;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        Self::parse_optional(text).ok_or(EmptyVerdict)
    }
}

impl TryFrom<String> for Verdict {
    type Error = EmptyVerdict;

    fn try_from(text: String) -> Result<Self, Self::Error> {
        text.parse()
    }
}

impl From<Verdict> for String {
    fn from(verdict: Verdict) -> Self {
        match verdict {
            Verdict::AcceptRecommendation => OK_KEYWORD.to_owned(),
            Verdict::Own(answer) => answer.into(),
        }
    }
}

keyword_enum! {
    /// Whether a questions file is still being edited or ends with the `**ANSWERED**` marker.
    pub enum AnswerStatus: "answer status" {
        /// The file does not end with `**ANSWERED**`: saving it does not send the answers yet.
        Draft => "draft",
        /// The last non-empty line is `**ANSWERED**`: the answers are ready to send.
        Answered => "answered",
    }
}

/// One lettered choice (`a) ...`) extracted from a question's body.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QuestionOption {
    /// The option's letter (`a`–`z`).
    pub letter: char,
    /// The option's text after the letter and `)` or `.`.
    pub text: String,
}

/// One numbered question in a [`QuestionsFile`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Question {
    /// The question number from `## <number>.`, starting at 1.
    number: NonZeroU32,
    /// The title after `## <number>. `.
    title: String,
    /// The lines between the heading and `**Recommended:**`, trimmed.
    body: String,
    /// The lettered options found in `body`, in order.
    options: Vec<QuestionOption>,
    /// The text after `**Recommended:**`, trimmed and never empty.
    recommended: String,
    /// The verdict after `**Verdict:**`, if the human has written one.
    verdict: Option<Verdict>,
}

impl Question {
    /// Creates a question with a non-empty `title` and a non-empty `recommended` text.
    ///
    /// # Errors
    /// [`QuestionsError::MissingTitle`] when `title` is blank, or [`QuestionsError::MissingRecommendation`] when
    /// `recommended` is blank.
    pub fn new(
        number: NonZeroU32,
        _title: &str,
        _body: &str,
        _recommended: &str,
        _verdict: Option<Verdict>,
    ) -> Result<Self, QuestionsError> {
        Err(QuestionsError::MissingTitle {
            line: NonZeroUsize::MIN,
            number,
        })
    }

    /// The question's number (`1` in `## 1.`).
    #[must_use]
    pub const fn number(&self) -> NonZeroU32 {
        self.number
    }

    /// The question's title, after `## <number>.`.
    #[must_use]
    pub fn title(&self) -> &str {
        &self.title
    }

    /// The body lines between the heading and `**Recommended:**`.
    #[must_use]
    pub fn body(&self) -> &str {
        &self.body
    }

    /// The lettered options (`a) ...`) extracted from [`Self::body`].
    #[must_use]
    pub fn options(&self) -> &[QuestionOption] {
        &self.options
    }

    /// The recommendation text after `**Recommended:**`.
    #[must_use]
    pub fn recommended(&self) -> &str {
        &self.recommended
    }

    /// The verdict written after `**Verdict:**`, or `None` when the line is empty.
    #[must_use]
    pub const fn verdict(&self) -> Option<&Verdict> {
        self.verdict.as_ref()
    }

    /// Sets or clears the question's verdict.
    pub fn set_verdict(&mut self, verdict: Option<Verdict>) {
        self.verdict = verdict;
    }

    /// The effective answer text: [`Self::recommended`] when the verdict is [`Verdict::AcceptRecommendation`], the
    /// human's text when [`Verdict::Own`], or `None` when unanswered.
    #[must_use]
    pub fn resolved_answer(&self) -> Option<&str> {
        match &self.verdict {
            Some(Verdict::AcceptRecommendation) => Some(&self.recommended),
            Some(Verdict::Own(own)) => Some(own.as_str()),
            None => None,
        }
    }
}

/// What is wrong with the number in a `## <number>.` question heading.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum QuestionNumberProblem {
    /// It is not ASCII digits without a leading zero (`01`, `+1`, `x`).
    #[error("it must be digits without a leading zero")]
    Shape,
    /// It is `0` or above `u32::MAX`.
    #[error("it must be from 1 to {}", u32::MAX)]
    Range(#[source] ParseIntError),
}

/// Why parsing, building or validating a [`QuestionsFile`] failed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum QuestionsError {
    /// The text is longer than [`MAX_QUESTIONS_BYTES`].
    #[error("the questions file is {len} bytes: the limit is {max}")]
    TooLarge {
        /// The length of the text, in bytes.
        len: usize,
        /// The limit, in bytes.
        max: usize,
    },
    /// The file has no `## N. Title` questions.
    #[error("no questions in the \"## N. Title\" format")]
    NoQuestions,
    /// A `## ...` question heading has an invalid number.
    #[error("line {line}: invalid question number {text:?}")]
    InvalidQuestionNumber {
        /// The line of the heading.
        line: NonZeroUsize,
        /// The text before the `.`.
        text: String,
        /// What is wrong with the number.
        #[source]
        problem: QuestionNumberProblem,
    },
    /// A question heading has no title after `## N.`.
    #[error("line {line}: question {number} has no title")]
    MissingTitle {
        /// The line of the heading.
        line: NonZeroUsize,
        /// The question number.
        number: NonZeroU32,
    },
    /// Two questions in the file have the same number.
    #[error("line {line}: question {number} is already defined on line {first_line}")]
    DuplicateQuestion {
        /// The line of the duplicate heading.
        line: NonZeroUsize,
        /// The repeated question number.
        number: NonZeroU32,
        /// The line of the first question with this number.
        first_line: NonZeroUsize,
    },
    /// A question has no `**Recommended:**` section, or its recommendation text is empty.
    #[error("line {line}: question {number} has no **Recommended:** text")]
    MissingRecommendation {
        /// The line where the question starts.
        line: NonZeroUsize,
        /// The question number.
        number: NonZeroU32,
    },
    /// A question has more than one `**Recommended:**` label.
    #[error("line {line}: question {number} has **Recommended:** twice")]
    DuplicateRecommendation {
        /// The line of the second `**Recommended:**` label.
        line: NonZeroUsize,
        /// The question number.
        number: NonZeroU32,
    },
    /// A question has no `**Verdict:**` label line.
    #[error("line {line}: question {number} is missing its **Verdict:** line")]
    MissingVerdictLine {
        /// The line where the question starts.
        line: NonZeroUsize,
        /// The question number.
        number: NonZeroU32,
    },
    /// A question has more than one `**Verdict:**` label.
    #[error("line {line}: question {number} has **Verdict:** twice")]
    DuplicateVerdictLine {
        /// The line of the second `**Verdict:**` label.
        line: NonZeroUsize,
        /// The question number.
        number: NonZeroU32,
    },
    /// A question has an empty `**Verdict:**` when complete answers are required.
    #[error("question {number} has no verdict")]
    MissingVerdict {
        /// The unanswered question number.
        number: NonZeroU32,
    },
    /// Complete answers were requested from a file that is not marked `**ANSWERED**`.
    #[error("the questions file is not marked {ANSWERED_MARKER} on its last line")]
    NotMarkedAnswered,
    /// No question in the file has this number.
    #[error("unknown question {number}")]
    UnknownQuestion {
        /// The number that was looked up.
        number: NonZeroU32,
    },
}

/// A parsed or constructed questions file (`questions-<round>.md`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuestionsFile {
    /// The `# ...` heading at the top of the file, without the leading `# `.
    title: Option<String>,
    /// The answering instructions rendered from a prompt template, placed as a blockquote below the title.
    instructions: Option<String>,
    /// The questions, in file order; never empty, with unique numbers.
    questions: Vec<Question>,
    /// Whether the last non-empty line is `**ANSWERED**`.
    status: AnswerStatus,
}

impl QuestionsFile {
    /// Builds a questions file from its parts.
    ///
    /// # Errors
    /// [`QuestionsError::NoQuestions`] when `questions` is empty, or [`QuestionsError::DuplicateQuestion`] when two
    /// questions share a number.
    pub fn new(
        _title: Option<String>,
        _instructions: Option<String>,
        _questions: Vec<Question>,
        _status: AnswerStatus,
    ) -> Result<Self, QuestionsError> {
        Err(QuestionsError::NoQuestions)
    }

    /// Parses the Markdown text of a questions file.
    ///
    /// # Errors
    /// A [`QuestionsError`] when the text exceeds [`MAX_QUESTIONS_BYTES`], has no questions, or a question is
    /// malformed (bad or duplicate number, missing title, missing or duplicate `**Recommended:**` or `**Verdict:**`).
    pub fn parse(_text: &str) -> Result<Self, QuestionsError> {
        Err(QuestionsError::NoQuestions)
    }

    /// The `# ...` title at the top of the file, without `# `.
    #[must_use]
    pub fn title(&self) -> Option<&str> {
        self.title.as_deref()
    }

    /// The answering instructions from the prompt template, without the leading `> ` blockquote prefixes.
    #[must_use]
    pub fn instructions(&self) -> Option<&str> {
        self.instructions.as_deref()
    }

    /// Sets or replaces the answering instructions from a rendered prompt template.
    pub fn set_instructions(&mut self, instructions: Option<String>) {
        self.instructions = instructions;
    }

    /// The questions in the file, in order.
    #[must_use]
    pub fn questions(&self) -> &[Question] {
        &self.questions
    }

    /// The question numbered `number`, if present.
    #[must_use]
    pub fn question(&self, number: NonZeroU32) -> Option<&Question> {
        self.questions.iter().find(|q| q.number == number)
    }

    /// Whether the file is a draft or marked `**ANSWERED**`.
    #[must_use]
    pub const fn status(&self) -> AnswerStatus {
        self.status
    }

    /// Whether the file ends with the `**ANSWERED**` marker.
    #[must_use]
    pub const fn is_answered(&self) -> bool {
        matches!(self.status, AnswerStatus::Answered)
    }

    /// Sets whether the rendered file ends with `**ANSWERED**`.
    pub fn set_status(&mut self, status: AnswerStatus) {
        self.status = status;
    }

    /// Sets or clears the verdict of question `number`.
    ///
    /// # Errors
    /// [`QuestionsError::UnknownQuestion`] when no question in the file has `number`.
    pub fn set_verdict(&mut self, number: NonZeroU32, _verdict: Option<Verdict>) -> Result<(), QuestionsError> {
        Err(QuestionsError::UnknownQuestion { number })
    }

    /// Every question's resolved answer (`ok` expanded to the question's recommendation, or the human's own answer),
    /// keyed by question number.
    ///
    /// # Errors
    /// [`QuestionsError::MissingVerdict`] naming the first question whose `**Verdict:**` is empty.
    pub fn answers(&self) -> Result<BTreeMap<NonZeroU32, &str>, QuestionsError> {
        Err(QuestionsError::NoQuestions)
    }

    /// Checks that the file is marked `**ANSWERED**` and that every question has a verdict, and returns the resolved
    /// answers by question number.
    ///
    /// # Errors
    /// [`QuestionsError::NotMarkedAnswered`] when [`Self::status`] is [`AnswerStatus::Draft`], or
    /// [`QuestionsError::MissingVerdict`] when a question's verdict is empty.
    pub fn require_answered(&self) -> Result<BTreeMap<NonZeroU32, &str>, QuestionsError> {
        Err(QuestionsError::NotMarkedAnswered)
    }

    /// Renders the questions file to Markdown, including the title, the template instructions as a blockquote, every
    /// question with its `**Recommended:**` and `**Verdict:**`, and `**ANSWERED**` on the last line when
    /// [`Self::status`] is [`AnswerStatus::Answered`].
    #[must_use]
    pub fn render(&self) -> String {
        String::new()
    }

    /// Renders the answered questions for the agent's resume prompt: drops the answering instructions and the
    /// `**ANSWERED**` marker, and expands `ok` verdicts to include the accepted recommendation text.
    ///
    /// # Errors
    /// [`QuestionsError::MissingVerdict`] when any question has no verdict.
    pub fn render_for_agent(&self) -> Result<String, QuestionsError> {
        Err(QuestionsError::NoQuestions)
    }
}

impl fmt::Display for QuestionsFile {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.render())
    }
}

impl FromStr for QuestionsFile {
    type Err = QuestionsError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        Self::parse(text)
    }
}

#[cfg(test)]
mod tests {
    use std::error::Error;

    use super::*;
    use crate::kinds::tests::{assert_keywords, assert_refuses};

    const ANSWER_STATUSES: &[(AnswerStatus, &str)] =
        &[(AnswerStatus::Draft, "draft"), (AnswerStatus::Answered, "answered")];

    fn qnum(n: u32) -> NonZeroU32 {
        NonZeroU32::new(n).unwrap()
    }

    fn line(n: usize) -> NonZeroUsize {
        NonZeroUsize::new(n).unwrap()
    }

    const SAMPLE_TEMPLATE_NOTE: &str = "Answer every question after **Verdict:** — `ok` accepts the recommendation.\nWhen done, put **ANSWERED** on the last line and save.";

    #[test]
    fn answer_statuses_have_their_texts_and_round_trip() {
        assert_eq!(
            AnswerStatus::ALL,
            ANSWER_STATUSES.iter().map(|&(s, _)| s).collect::<Vec<_>>()
        );
        assert_keywords(ANSWER_STATUSES);
        assert_refuses("answer status", ANSWER_STATUSES, &["pending", "done", "open"]);
    }

    #[test]
    fn own_answer_trims_and_refuses_empty_or_reserved_ok() {
        let answer: OwnAnswer = "  b) Use Unix domain sockets  ".parse().unwrap();
        assert_eq!(
            answer.as_str(),
            "b) Use a Unix domain socket".replace("a Unix domain socket", "Unix domain sockets")
        );
        assert_eq!(answer.to_string(), "b) Use Unix domain sockets");
        assert_eq!(String::from(answer.clone()), "b) Use Unix domain sockets");

        for empty in ["", " ", "\t\n"] {
            assert_eq!(empty.parse::<OwnAnswer>(), Err(InvalidOwnAnswer::Empty));
        }
        for reserved in ["ok", "OK", "Ok", " oK ", "\tok\n"] {
            assert_eq!(reserved.parse::<OwnAnswer>(), Err(InvalidOwnAnswer::ReservedOk));
        }
    }

    #[test]
    fn verdict_parses_ok_in_any_case_and_own_answers_and_round_trips_in_json() {
        for ok in ["ok", "OK", "Ok", "  ok  "] {
            let v: Verdict = ok.parse().unwrap();
            assert_eq!(v, Verdict::AcceptRecommendation);
            assert_eq!(v.to_string(), "ok");
            let json = serde_json::to_string(&v).unwrap();
            assert_eq!(json, "\"ok\"");
            assert_eq!(
                serde_json::from_str::<Verdict>(&json).unwrap(),
                Verdict::AcceptRecommendation
            );
        }

        let own: Verdict = "a) Keep BTreeMap — deterministic".parse().unwrap();
        assert_eq!(own, Verdict::own("a) Keep BTreeMap — deterministic").unwrap());
        let json = serde_json::to_string(&own).unwrap();
        assert_eq!(serde_json::from_str::<Verdict>(&json).unwrap(), own);

        assert_eq!("".parse::<Verdict>(), Err(EmptyVerdict));
        assert_eq!("   ".parse::<Verdict>(), Err(EmptyVerdict));
        assert_eq!(
            serde_json::from_str::<Verdict>("\"\"").unwrap_err().classify(),
            serde_json::error::Category::Data
        );
    }

    #[test]
    fn non_ascii_multiline_crlf_bom_and_extra_labels_round_trip() {
        let raw = "\u{feff}# Pytania — zadanie\r\n\r\n> Instrukcja z szablonu: wpisz werdykt.\r\n\r\n## 1. Który format wybrać dla żółwia?\r\n- a) JSON — czytelny\r\n* (b) TOML — prosty\r\nc. Binarny — mały\r\n\r\n**Recommended:** a) — łatwy przegląd\r\nw wielu liniach\r\n\r\n**Verdict:** własna odpowiedź:\r\nużyj JSON z polskimi znakami ąęćłńóśźż\r\n\r\n**Note:** dodatkowa uwaga poza werdyktem\r\n\r\n**ANSWERED**\r\n";
        let parsed = QuestionsFile::parse(raw).unwrap();
        assert!(parsed.is_answered());
        assert_eq!(parsed.title(), Some("Pytania — zadanie"));
        assert_eq!(parsed.instructions(), Some("Instrukcja z szablonu: wpisz werdykt."));
        let q = &parsed.questions()[0];
        assert_eq!(q.title(), "Który format wybrać dla żółwia?");
        assert_eq!(q.options().len(), 3);
        assert_eq!(q.options()[0].letter, 'a');
        assert_eq!(q.options()[1].letter, 'b');
        assert_eq!(q.options()[2].letter, 'c');
        assert_eq!(q.recommended(), "a) — łatwy przegląd\nw wielu liniach");
        assert_eq!(
            q.resolved_answer(),
            Some("własna odpowiedź:\nużyj JSON z polskimi znakami ąęćłńóśźż")
        );
    }

    #[test]
    fn constructors_and_set_verdict_refuse_invalid_inputs() {
        assert_eq!(
            Question::new(qnum(1), "   ", "body", "rec", None),
            Err(QuestionsError::MissingTitle {
                line: line(1),
                number: qnum(1),
            })
        );
        assert_eq!(
            Question::new(qnum(1), "Title", "body", "   ", None),
            Err(QuestionsError::MissingRecommendation {
                line: line(1),
                number: qnum(1),
            })
        );
        assert_eq!(
            QuestionsFile::new(None, None, Vec::new(), AnswerStatus::Draft),
            Err(QuestionsError::NoQuestions)
        );
        let q1 = Question::new(qnum(1), "First", "", "a", None).unwrap();
        let q1_dup = Question::new(qnum(1), "Second", "", "b", None).unwrap();
        assert_eq!(
            QuestionsFile::new(None, None, vec![q1.clone(), q1_dup], AnswerStatus::Draft),
            Err(QuestionsError::DuplicateQuestion {
                line: line(2),
                number: qnum(1),
                first_line: line(1),
            })
        );
        let mut file = QuestionsFile::new(None, None, vec![q1], AnswerStatus::Draft).unwrap();
        assert_eq!(
            file.set_verdict(qnum(99), Some(Verdict::AcceptRecommendation)),
            Err(QuestionsError::UnknownQuestion { number: qnum(99) })
        );
    }

    #[test]
    fn rendered_questions_file_answered_with_ok_and_own_text_and_marked_answered_parses_back() {
        let q1 = Question::new(
            qnum(1),
            "Which state format should we persist?",
            "a) JSON — human-readable\nb) Binary — compact",
            "a) — easy to inspect and debug (investigator)",
            None,
        )
        .unwrap();
        let q2 = Question::new(
            qnum(2),
            "How should the socket frame messages?",
            "a) JSON lines\nb) Length-prefixed frames",
            "a) — matches the plan and is simple to trace (investigator)",
            None,
        )
        .unwrap();

        let mut file = QuestionsFile::new(
            Some("Questions — task-20261111-191554-k3x9".to_owned()),
            Some(SAMPLE_TEMPLATE_NOTE.to_owned()),
            vec![q1, q2],
            AnswerStatus::Draft,
        )
        .unwrap();

        let draft_text = file.render();
        let parsed_draft = QuestionsFile::parse(&draft_text).unwrap();
        assert!(!parsed_draft.is_answered());
        assert_eq!(parsed_draft.status(), AnswerStatus::Draft);
        assert_eq!(parsed_draft.title(), Some("Questions — task-20261111-191554-k3x9"));
        assert_eq!(parsed_draft.instructions(), Some(SAMPLE_TEMPLATE_NOTE));
        assert_eq!(parsed_draft.questions().len(), 2);
        assert_eq!(parsed_draft.questions()[0].verdict(), None);
        assert_eq!(parsed_draft.questions()[1].verdict(), None);
        assert_eq!(
            parsed_draft.questions()[0].options(),
            &[
                QuestionOption {
                    letter: 'a',
                    text: "JSON — human-readable".to_owned(),
                },
                QuestionOption {
                    letter: 'b',
                    text: "Binary — compact".to_owned(),
                },
            ]
        );

        // Answer question 1 with `ok` and question 2 with own text, and mark ANSWERED.
        file.set_verdict(qnum(1), Some(Verdict::AcceptRecommendation)).unwrap();
        file.set_verdict(
            qnum(2),
            Some(Verdict::own("a) JSON lines with a 4 MiB line bound and UTF-8").unwrap()),
        )
        .unwrap();
        file.set_status(AnswerStatus::Answered);

        let answered_text = file.render();
        assert!(answered_text.trim_end().ends_with(ANSWERED_MARKER));

        let parsed = QuestionsFile::parse(&answered_text).unwrap();
        assert!(parsed.is_answered());
        assert_eq!(parsed, file);
        assert_eq!(
            parsed.question(qnum(1)).unwrap().verdict(),
            Some(&Verdict::AcceptRecommendation)
        );
        assert_eq!(
            parsed.question(qnum(1)).unwrap().resolved_answer(),
            Some("a) — easy to inspect and debug (investigator)")
        );
        assert_eq!(
            parsed.question(qnum(2)).unwrap().verdict(),
            Some(&Verdict::own("a) JSON lines with a 4 MiB line bound and UTF-8").unwrap())
        );
        assert_eq!(
            parsed.question(qnum(2)).unwrap().resolved_answer(),
            Some("a) JSON lines with a 4 MiB line bound and UTF-8")
        );

        let answers = parsed.require_answered().unwrap();
        assert_eq!(answers.len(), 2);
        assert_eq!(answers[&qnum(1)], "a) — easy to inspect and debug (investigator)");
        assert_eq!(answers[&qnum(2)], "a) JSON lines with a 4 MiB line bound and UTF-8");
    }

    #[test]
    fn missing_verdict_is_refused_when_answers_are_required_and_when_verdict_line_is_absent() {
        let raw_with_empty_verdict = "\
# Questions — task-20261111-191554-k3x9

## 1. First question?
a) Option A
b) Option B

**Recommended:** a) — simpler

**Verdict:** ok

## 2. Second question?
a) Option A
b) Option B

**Recommended:** b) — faster

**Verdict:**

**ANSWERED**
";
        let parsed = QuestionsFile::parse(raw_with_empty_verdict).unwrap();
        assert!(parsed.is_answered());
        assert_eq!(
            parsed.answers(),
            Err(QuestionsError::MissingVerdict { number: qnum(2) })
        );
        assert_eq!(
            parsed.require_answered(),
            Err(QuestionsError::MissingVerdict { number: qnum(2) })
        );
        assert_eq!(
            parsed.render_for_agent(),
            Err(QuestionsError::MissingVerdict { number: qnum(2) })
        );

        let raw_without_verdict_line = "\
# Questions — task-20261111-191554-k3x9

## 1. Question without verdict label
a) Option A

**Recommended:** a) — only choice
";
        assert_eq!(
            QuestionsFile::parse(raw_without_verdict_line),
            Err(QuestionsError::MissingVerdictLine {
                line: line(3),
                number: qnum(1),
            })
        );
    }

    #[test]
    fn draft_file_without_answered_marker_is_refused_by_require_answered() {
        let draft = "\
# Questions — task-20261111-191554-k3x9

## 1. Ready?
a) Yes

**Recommended:** a) — ready

**Verdict:** ok
";
        let parsed = QuestionsFile::parse(draft).unwrap();
        assert!(!parsed.is_answered());
        assert_eq!(parsed.answers().unwrap()[&qnum(1)], "a) — ready");
        assert_eq!(parsed.require_answered(), Err(QuestionsError::NotMarkedAnswered));
    }

    #[test]
    fn render_for_agent_drops_instructions_and_answered_marker_and_expands_ok() {
        let text = "\
# Questions — task-20261111-191554-k3x9

> Answer every question after **Verdict:** — `ok` accepts the recommendation.

## 1. Storage?
a) JSON

**Recommended:** a) — readable

**Verdict:** ok

## 2. Compression?
a) None
b) Zstd

**Recommended:** a) — small files

**Verdict:** b) Use zstd level 3

**ANSWERED**
";
        let parsed = QuestionsFile::parse(text).unwrap();
        let for_agent = parsed.render_for_agent().unwrap();
        assert!(!for_agent.contains("Answer every question after"));
        assert!(!for_agent.contains(ANSWERED_MARKER));
        assert!(for_agent.contains("**Verdict:** ok — the human accepts the recommendation: a) — readable"));
        assert!(for_agent.contains("**Verdict:** b) Use zstd level 3"));
    }

    #[test]
    fn malformed_questions_files_report_their_exact_error_and_line() {
        assert_eq!(QuestionsFile::parse(""), Err(QuestionsError::NoQuestions));
        assert_eq!(
            QuestionsFile::parse("# Questions\n\n> Only instructions\n"),
            Err(QuestionsError::NoQuestions)
        );

        let oversized = "x".repeat(MAX_QUESTIONS_BYTES + 1);
        assert_eq!(
            QuestionsFile::parse(&oversized),
            Err(QuestionsError::TooLarge {
                len: MAX_QUESTIONS_BYTES + 1,
                max: MAX_QUESTIONS_BYTES,
            })
        );

        let bad_shape = "## 01. Leading zero\n**Recommended:** a\n**Verdict:**\n";
        let err = QuestionsFile::parse(bad_shape).unwrap_err();
        assert_eq!(
            err,
            QuestionsError::InvalidQuestionNumber {
                line: line(1),
                text: "01".to_owned(),
                problem: QuestionNumberProblem::Shape,
            }
        );
        assert_eq!(
            err.source().and_then(|s| s.downcast_ref::<QuestionNumberProblem>()),
            Some(&QuestionNumberProblem::Shape)
        );

        let bad_zero = "## 0. Zero number\n**Recommended:** a\n**Verdict:**\n";
        let err = QuestionsFile::parse(bad_zero).unwrap_err();
        assert!(matches!(
            err,
            QuestionsError::InvalidQuestionNumber {
                line: l,
                ref text,
                problem: QuestionNumberProblem::Range(_),
            } if l == line(1) && text == "0"
        ));
        assert!(
            err.source()
                .and_then(Error::source)
                .and_then(|s| s.downcast_ref::<ParseIntError>())
                .is_some()
        );

        let missing_title = "## 1.   \n**Recommended:** a\n**Verdict:**\n";
        assert_eq!(
            QuestionsFile::parse(missing_title),
            Err(QuestionsError::MissingTitle {
                line: line(1),
                number: qnum(1),
            })
        );

        let duplicate_q = "\
## 1. First
**Recommended:** a
**Verdict:**

## 1. Duplicate
**Recommended:** b
**Verdict:**
";
        assert_eq!(
            QuestionsFile::parse(duplicate_q),
            Err(QuestionsError::DuplicateQuestion {
                line: line(5),
                number: qnum(1),
                first_line: line(1),
            })
        );

        let missing_rec = "## 1. Title\nOnly body\n**Verdict:** ok\n";
        assert_eq!(
            QuestionsFile::parse(missing_rec),
            Err(QuestionsError::MissingRecommendation {
                line: line(1),
                number: qnum(1),
            })
        );

        let dup_rec = "## 1. Title\n**Recommended:** a\n**Recommended:** b\n**Verdict:**\n";
        assert_eq!(
            QuestionsFile::parse(dup_rec),
            Err(QuestionsError::DuplicateRecommendation {
                line: line(3),
                number: qnum(1),
            })
        );

        let dup_verdict = "## 1. Title\n**Recommended:** a\n**Verdict:** ok\n**Verdict:** no\n";
        assert_eq!(
            QuestionsFile::parse(dup_verdict),
            Err(QuestionsError::DuplicateVerdictLine {
                line: line(4),
                number: qnum(1),
            })
        );
    }
}
