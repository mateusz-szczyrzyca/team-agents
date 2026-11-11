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

/// The byte order mark some editors place at the start of a file; it is stripped before parsing.
const BYTE_ORDER_MARK: char = '\u{feff}';

/// Prefix of a numbered question heading at column 0.
const QUESTION_HEADING_PREFIX: &str = "## ";

/// Prefix of a document title heading at column 0.
const TITLE_HEADING_PREFIX: &str = "# ";

/// Canonical label for a question's recommendation.
const RECOMMENDED_LABEL: &str = "**Recommended:**";

/// Canonical label for a question's verdict.
const VERDICT_LABEL: &str = "**Verdict:**";

/// Prefix written when expanding an `ok` verdict for the agent's resume prompt.
const EXPANDED_OK_PREFIX: &str = "**Verdict:** ok — the human accepts the recommendation: ";

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
/// ```
/// # fn main() -> Result<(), agent_core::questions::InvalidOwnAnswer> {
/// let answer: agent_core::questions::OwnAnswer = "b) Use a BTreeMap for deterministic order".parse()?;
/// assert_eq!(answer.as_str(), "b) Use a BTreeMap for deterministic order");
/// # Ok(())
/// # }
/// ```
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

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let trimmed = text.trim();
        if trimmed.is_empty() {
            return Err(InvalidOwnAnswer::Empty);
        }
        if trimmed.eq_ignore_ascii_case(OK_KEYWORD) {
            return Err(InvalidOwnAnswer::ReservedOk);
        }
        Ok(Self(trimmed.to_owned()))
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
    pub fn parse_optional(text: &str) -> Option<Self> {
        let trimmed = text.trim();
        if trimmed.is_empty() {
            return None;
        }
        if trimmed.eq_ignore_ascii_case(OK_KEYWORD) {
            return Some(Self::AcceptRecommendation);
        }
        Some(Self::Own(OwnAnswer(trimmed.to_owned())))
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

/// Extracts lettered options (`a) ...`, `- a) ...`, `* (a) ...`, `a. ...`) from the lines of `body`.
fn extract_options(body: &str) -> Vec<QuestionOption> {
    body.lines().filter_map(parse_option_line).collect()
}

/// Parses one line as a [`QuestionOption`], if it has the shape `[optional - or *] [optional (]<a-z>[) or .] <text>`.
fn parse_option_line(line: &str) -> Option<QuestionOption> {
    let trimmed = line.trim();
    let after_bullet = match trimmed.strip_prefix(['-', '*']) {
        Some(rest) if rest.starts_with(char::is_whitespace) => rest.trim_start(),
        Some(_) => return None,
        None => trimmed,
    };
    let after_open = after_bullet
        .strip_prefix('(')
        .unwrap_or(after_open_fallback(after_bullet));
    let mut chars = after_open.chars();
    let letter = chars.next().filter(char::is_ascii_lowercase)?;
    let delim = chars.next().filter(|&c| matches!(c, ')' | '.'))?;
    let _ = delim;
    let rest = chars.as_str();
    if !rest.starts_with(char::is_whitespace) {
        return None;
    }
    let text = rest.trim();
    (!text.is_empty()).then(|| QuestionOption {
        letter,
        text: text.to_owned(),
    })
}

/// Identity helper for `parse_option_line` when there is no leading `(`.
const fn after_open_fallback(text: &str) -> &str {
    text
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
        title: &str,
        body: &str,
        recommended: &str,
        verdict: Option<Verdict>,
    ) -> Result<Self, QuestionsError> {
        Self::new_at_line(NonZeroUsize::MIN, number, title, body, recommended, verdict)
    }

    /// Creates a question whose heading starts on `line`.
    fn new_at_line(
        line: NonZeroUsize,
        number: NonZeroU32,
        title: &str,
        body: &str,
        recommended: &str,
        verdict: Option<Verdict>,
    ) -> Result<Self, QuestionsError> {
        let title = title.trim();
        if title.is_empty() {
            return Err(QuestionsError::MissingTitle { line, number });
        }
        let recommended = recommended.trim();
        if recommended.is_empty() {
            return Err(QuestionsError::MissingRecommendation { line, number });
        }
        let body = body.trim().to_owned();
        let options = extract_options(&body);
        Ok(Self {
            number,
            title: title.to_owned(),
            body,
            options,
            recommended: recommended.to_owned(),
            verdict,
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
        title: Option<String>,
        instructions: Option<String>,
        questions: Vec<Question>,
        status: AnswerStatus,
    ) -> Result<Self, QuestionsError> {
        if questions.is_empty() {
            return Err(QuestionsError::NoQuestions);
        }
        let mut seen: BTreeMap<NonZeroU32, NonZeroUsize> = BTreeMap::new();
        for (index, question) in questions.iter().enumerate() {
            let line = NonZeroUsize::MIN.saturating_add(index);
            if let Some(&first_line) = seen.get(&question.number) {
                return Err(QuestionsError::DuplicateQuestion {
                    line,
                    number: question.number,
                    first_line,
                });
            }
            seen.insert(question.number, line);
        }
        let title = title.map(|t| t.trim().to_owned()).filter(|t| !t.is_empty());
        let instructions = instructions.map(|i| i.trim().to_owned()).filter(|i| !i.is_empty());
        Ok(Self {
            title,
            instructions,
            questions,
            status,
        })
    }

    /// Parses the Markdown text of a questions file.
    ///
    /// # Errors
    /// A [`QuestionsError`] when the text exceeds [`MAX_QUESTIONS_BYTES`], has no questions, or a question is
    /// malformed (bad or duplicate number, missing title, missing or duplicate `**Recommended:**` or `**Verdict:**`).
    pub fn parse(text: &str) -> Result<Self, QuestionsError> {
        if text.len() > MAX_QUESTIONS_BYTES {
            return Err(QuestionsError::TooLarge {
                len: text.len(),
                max: MAX_QUESTIONS_BYTES,
            });
        }
        let text = text.strip_prefix(BYTE_ORDER_MARK).unwrap_or(text);
        let mut lines: Vec<(NonZeroUsize, &str)> = text
            .lines()
            .enumerate()
            .map(|(index, line)| (NonZeroUsize::MIN.saturating_add(index), line))
            .collect();

        let status = match lines.iter().rposition(|(_, line)| !line.trim().is_empty()) {
            Some(last_idx) if lines.get(last_idx).is_some_and(|(_, line)| is_answered_marker(line)) => {
                lines.truncate(last_idx);
                AnswerStatus::Answered
            }
            _ => AnswerStatus::Draft,
        };

        let mut parser = FileParser::default();
        for (line_no, line) in lines {
            parser.accept(line_no, line)?;
        }
        parser.finish(status)
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
        self.instructions = instructions.map(|i| i.trim().to_owned()).filter(|i| !i.is_empty());
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
    pub fn set_verdict(&mut self, number: NonZeroU32, verdict: Option<Verdict>) -> Result<(), QuestionsError> {
        let Some(question) = self.questions.iter_mut().find(|q| q.number == number) else {
            return Err(QuestionsError::UnknownQuestion { number });
        };
        question.set_verdict(verdict);
        Ok(())
    }

    /// Every question's resolved answer (`ok` expanded to the question's recommendation, or the human's own answer),
    /// keyed by question number.
    ///
    /// # Errors
    /// [`QuestionsError::MissingVerdict`] naming the first question whose `**Verdict:**` is empty.
    pub fn answers(&self) -> Result<BTreeMap<NonZeroU32, &str>, QuestionsError> {
        let mut out = BTreeMap::new();
        for question in &self.questions {
            let answer = question.resolved_answer().ok_or(QuestionsError::MissingVerdict {
                number: question.number,
            })?;
            out.insert(question.number, answer);
        }
        Ok(out)
    }

    /// Checks that the file is marked `**ANSWERED**` and that every question has a verdict, and returns the resolved
    /// answers by question number.
    ///
    /// # Errors
    /// [`QuestionsError::NotMarkedAnswered`] when [`Self::status`] is [`AnswerStatus::Draft`], or
    /// [`QuestionsError::MissingVerdict`] when a question's verdict is empty.
    pub fn require_answered(&self) -> Result<BTreeMap<NonZeroU32, &str>, QuestionsError> {
        if !self.is_answered() {
            return Err(QuestionsError::NotMarkedAnswered);
        }
        self.answers()
    }

    /// Renders the questions file to Markdown, including the title, the template instructions as a blockquote, every
    /// question with its `**Recommended:**` and `**Verdict:**`, and `**ANSWERED**` on the last line when
    /// [`Self::status`] is [`AnswerStatus::Answered`].
    #[must_use]
    pub fn render(&self) -> String {
        let mut blocks: Vec<String> = Vec::new();
        if let Some(title) = &self.title {
            blocks.push(format!("{TITLE_HEADING_PREFIX}{title}"));
        }
        if let Some(instructions) = &self.instructions {
            let quoted: Vec<String> = instructions
                .lines()
                .map(|line| {
                    let trimmed = line.trim_end();
                    if trimmed.is_empty() {
                        ">".to_owned()
                    } else if trimmed.starts_with('>') {
                        trimmed.to_owned()
                    } else {
                        format!("> {trimmed}")
                    }
                })
                .collect();
            blocks.push(quoted.join("\n"));
        }
        for question in &self.questions {
            blocks.push(render_question(question, VerdictFormat::Editable));
        }
        if self.is_answered() {
            blocks.push(ANSWERED_MARKER.to_owned());
        }
        let mut rendered = blocks.join("\n\n");
        rendered.push('\n');
        rendered
    }

    /// Renders the answered questions for the agent's resume prompt: drops the answering instructions and the
    /// `**ANSWERED**` marker, and expands `ok` verdicts to include the accepted recommendation text.
    ///
    /// # Errors
    /// [`QuestionsError::MissingVerdict`] when any question has no verdict.
    pub fn render_for_agent(&self) -> Result<String, QuestionsError> {
        for question in &self.questions {
            if question.verdict.is_none() {
                return Err(QuestionsError::MissingVerdict {
                    number: question.number,
                });
            }
        }
        let mut blocks: Vec<String> = Vec::new();
        if let Some(title) = &self.title {
            blocks.push(format!("{TITLE_HEADING_PREFIX}{title}"));
        }
        for question in &self.questions {
            blocks.push(render_question(question, VerdictFormat::ExpandedForAgent));
        }
        let mut rendered = blocks.join("\n\n");
        rendered.push('\n');
        Ok(rendered)
    }
}

/// How a question's verdict is rendered.
#[derive(Clone, Copy)]
enum VerdictFormat {
    /// Editable file for the human: `**Verdict:** ok` or `**Verdict:**`.
    Editable,
    /// Resume prompt for the agent: `ok` expands to include the accepted recommendation.
    ExpandedForAgent,
}

/// Renders one [`Question`] to Markdown.
fn render_question(question: &Question, format: VerdictFormat) -> String {
    let mut parts: Vec<String> = Vec::new();
    let heading = format!("## {}. {}", question.number, question.title);
    if question.body.is_empty() {
        parts.push(heading);
    } else {
        parts.push(format!("{heading}\n{}", question.body));
    }
    parts.push(format!("{RECOMMENDED_LABEL} {}", question.recommended));
    let verdict_line = match (&question.verdict, format) {
        (None, _) => VERDICT_LABEL.to_owned(),
        (Some(Verdict::AcceptRecommendation), VerdictFormat::Editable) => {
            format!("{VERDICT_LABEL} {OK_KEYWORD}")
        }
        (Some(Verdict::AcceptRecommendation), VerdictFormat::ExpandedForAgent) => {
            let one_line_rec = question
                .recommended
                .lines()
                .map(str::trim)
                .filter(|line| !line.is_empty())
                .collect::<Vec<_>>()
                .join(" ");
            format!("{EXPANDED_OK_PREFIX}{one_line_rec}")
        }
        (Some(Verdict::Own(own)), _) => format!("{VERDICT_LABEL} {own}"),
    };
    parts.push(verdict_line);
    parts.join("\n\n")
}

/// Whether `line` is the `**ANSWERED**` marker (allowing spaces inside or around the `**` delimiters).
fn is_answered_marker(line: &str) -> bool {
    let trimmed = line.trim();
    let Some(inner) = trimmed.strip_prefix("**").and_then(|rest| rest.strip_suffix("**")) else {
        return false;
    };
    inner.trim() == "ANSWERED"
}

/// What a bold label at the start of a line is (`**Recommended:**`, `**Verdict:**`, or another `**<Label>:**`).
enum BoldLabel<'a> {
    /// `**Recommended:**` with the rest of the line after the closing `**`.
    Recommended(&'a str),
    /// `**Verdict:**` with the rest of the line after the closing `**`.
    Verdict(&'a str),
    /// Any other bold label `**<Word>:**`.
    Other,
}

/// Classifies `line` if it starts with a bold label `**<Name>:**`.
fn parse_bold_label(line: &str) -> Option<BoldLabel<'_>> {
    let after_open = line.strip_prefix("**")?;
    let (inside, after_close) = after_open.split_once("**")?;
    let label = inside.trim().strip_suffix(':')?.trim();
    if label.is_empty() || !label.chars().all(|c| c.is_alphabetic() || c == ' ') {
        return None;
    }
    match label {
        "Recommended" => Some(BoldLabel::Recommended(after_close.trim())),
        "Verdict" | "Answer" | "Odpowiedź" => Some(BoldLabel::Verdict(after_close.trim())),
        _ => Some(BoldLabel::Other),
    }
}

/// Parses a question number (`1`..=`u32::MAX`, ASCII digits without a leading zero).
fn parse_question_number(digits: &str) -> Result<NonZeroU32, QuestionNumberProblem> {
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) || (digits.len() > 1 && digits.starts_with('0'))
    {
        return Err(QuestionNumberProblem::Shape);
    }
    digits.parse::<NonZeroU32>().map_err(QuestionNumberProblem::Range)
}

/// Which section of an open question subsequent lines belong to.
#[derive(Clone, Copy, Default)]
enum QuestionSection {
    /// Between `## N. Title` and the first bold label.
    #[default]
    Body,
    /// Inside `**Recommended:**`.
    Recommended,
    /// Inside `**Verdict:**`.
    Verdict,
    /// Inside another bold label (`**Note:**`, etc.), ignored when building the verdict.
    Other,
}

/// A question while its lines are being collected by [`FileParser`].
struct OpenQuestion<'a> {
    /// The line of the `## N. Title` heading.
    start_line: NonZeroUsize,
    /// The question number.
    number: NonZeroU32,
    /// The question title.
    title: &'a str,
    /// Which section is currently open.
    section: QuestionSection,
    /// The lines of the body.
    body: Vec<&'a str>,
    /// Whether `**Recommended:**` has been seen.
    has_recommended: bool,
    /// The lines of the recommendation.
    recommended: Vec<&'a str>,
    /// Whether `**Verdict:**` has been seen.
    has_verdict: bool,
    /// The lines of the verdict.
    verdict: Vec<&'a str>,
}

impl OpenQuestion<'_> {
    /// Validates and closes the question into a [`Question`].
    fn close(self) -> Result<Question, QuestionsError> {
        if !self.has_recommended {
            return Err(QuestionsError::MissingRecommendation {
                line: self.start_line,
                number: self.number,
            });
        }
        let recommended = self.recommended.join("\n");
        if recommended.trim().is_empty() {
            return Err(QuestionsError::MissingRecommendation {
                line: self.start_line,
                number: self.number,
            });
        }
        if !self.has_verdict {
            return Err(QuestionsError::MissingVerdictLine {
                line: self.start_line,
                number: self.number,
            });
        }
        let body = self.body.join("\n");
        let verdict = Verdict::parse_optional(&self.verdict.join("\n"));
        Question::new_at_line(self.start_line, self.number, self.title, &body, &recommended, verdict)
    }
}

/// Line-by-line parser for [`QuestionsFile::parse`].
#[derive(Default)]
struct FileParser<'a> {
    /// The `# ...` title, if seen before the first question.
    title: Option<String>,
    /// Blockquote instruction lines seen before the first question.
    instructions: Vec<&'a str>,
    /// Closed questions in file order.
    questions: Vec<Question>,
    /// First line of each question number, to report duplicates.
    seen_numbers: BTreeMap<NonZeroU32, NonZeroUsize>,
    /// The question currently being read.
    current: Option<OpenQuestion<'a>>,
}

impl<'a> FileParser<'a> {
    /// Processes `line` at 1-based line number `line_no`.
    fn accept(&mut self, line_no: NonZeroUsize, line: &'a str) -> Result<(), QuestionsError> {
        if let Some(after_hashes) = line.strip_prefix(QUESTION_HEADING_PREFIX)
            && let Some((digits, rest)) = after_hashes.split_once('.')
            && !digits.is_empty()
            && !digits.contains(char::is_whitespace)
        {
            self.close_current()?;
            let number = parse_question_number(digits).map_err(|problem| QuestionsError::InvalidQuestionNumber {
                line: line_no,
                text: digits.to_owned(),
                problem,
            })?;
            let title = rest.trim();
            if title.is_empty() {
                return Err(QuestionsError::MissingTitle { line: line_no, number });
            }
            if let Some(&first_line) = self.seen_numbers.get(&number) {
                return Err(QuestionsError::DuplicateQuestion {
                    line: line_no,
                    number,
                    first_line,
                });
            }
            self.seen_numbers.insert(number, line_no);
            self.current = Some(OpenQuestion {
                start_line: line_no,
                number,
                title,
                section: QuestionSection::Body,
                body: Vec::new(),
                has_recommended: false,
                recommended: Vec::new(),
                has_verdict: false,
                verdict: Vec::new(),
            });
            return Ok(());
        }

        let Some(open) = &mut self.current else {
            self.accept_header(line);
            return Ok(());
        };

        if let Some(label) = parse_bold_label(line) {
            match label {
                BoldLabel::Recommended(rest) => {
                    if open.has_recommended {
                        return Err(QuestionsError::DuplicateRecommendation {
                            line: line_no,
                            number: open.number,
                        });
                    }
                    open.has_recommended = true;
                    open.section = QuestionSection::Recommended;
                    if !rest.is_empty() {
                        open.recommended.push(rest);
                    }
                }
                BoldLabel::Verdict(rest) => {
                    if open.has_verdict {
                        return Err(QuestionsError::DuplicateVerdictLine {
                            line: line_no,
                            number: open.number,
                        });
                    }
                    open.has_verdict = true;
                    open.section = QuestionSection::Verdict;
                    if !rest.is_empty() {
                        open.verdict.push(rest);
                    }
                }
                BoldLabel::Other => {
                    open.section = QuestionSection::Other;
                }
            }
            return Ok(());
        }

        match open.section {
            QuestionSection::Body => open.body.push(line),
            QuestionSection::Recommended => open.recommended.push(line),
            QuestionSection::Verdict => open.verdict.push(line),
            QuestionSection::Other => {}
        }
        Ok(())
    }

    /// Collects the `# ...` title and `> ...` instruction lines before the first question.
    fn accept_header(&mut self, line: &'a str) {
        let trimmed = line.trim();
        if self.title.is_none()
            && self.instructions.is_empty()
            && let Some(heading) = trimmed.strip_prefix(TITLE_HEADING_PREFIX)
        {
            let heading = heading.trim();
            if !heading.is_empty() {
                self.title = Some(heading.to_owned());
            }
            return;
        }
        if let Some(quoted) = trimmed.strip_prefix('>') {
            let unquoted = quoted.strip_prefix(' ').unwrap_or(quoted);
            self.instructions.push(unquoted);
        }
    }

    /// Closes the current open question, if any.
    fn close_current(&mut self) -> Result<(), QuestionsError> {
        if let Some(open) = self.current.take() {
            self.questions.push(open.close()?);
        }
        Ok(())
    }

    /// Finishes parsing and builds the [`QuestionsFile`].
    fn finish(mut self, status: AnswerStatus) -> Result<QuestionsFile, QuestionsError> {
        self.close_current()?;
        if self.questions.is_empty() {
            return Err(QuestionsError::NoQuestions);
        }
        let instructions = (!self.instructions.is_empty())
            .then(|| self.instructions.join("\n").trim().to_owned())
            .filter(|text| !text.is_empty());
        Ok(QuestionsFile {
            title: self.title,
            instructions,
            questions: self.questions,
            status,
        })
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
