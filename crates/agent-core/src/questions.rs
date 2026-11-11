//! Questions files: the Markdown file in which an agent (or the harness) asks the human questions and the human
//! answers them (see `docs/state.md`).
//!
//! ```text
//! # Questions — task-20261111-153601-k3x9
//!
//! > Answer every question after **Verdict:** …          (the instructions, from a template)
//!
//! ## 1. Which store?
//! a) in memory
//! b) on disk
//!
//! **Recommended:** a) — simpler (inv-opus)
//!
//! **Verdict:** ok
//!
//! **ANSWERED**
//! ```
//!
//! A [`QuestionsFile`] is the parsed file: a title, the instructions, free text, and [`Entry`]s — each `## N. title`,
//! its text, a `**Recommended:**` part, a `**Verdict:**` part and any other `**Label:**` part, which are kept as they
//! are. The human's answer to a question is its [`Verdict`]: `ok` accepts the recommendation, any other text is an
//! own answer. `**ANSWERED**` on the last line says the human is done. The labels are fixed English words whatever
//! language the questions are in; spaces inside the asterisks are tolerated.
//!
//! Nothing here reads a file or knows a wording: the text of the instructions and the words that tell the agent a
//! recommendation was accepted come from templates under `prompts/harness/questions/` and are passed in.

use std::collections::HashSet;

use std::fmt;
use std::num::{NonZeroU32, NonZeroUsize, ParseIntError};

/// Largest questions file accepted, in bytes: a bound on untrusted input, far above any real file.
pub const MAX_QUESTIONS_BYTES: usize = 1 << 20;

/// The label of the part that holds the human's answer.
const VERDICT_LABEL: &str = "Verdict";

/// The label of the part that holds the recommended answer.
const RECOMMENDED_LABEL: &str = "Recommended";

/// The verdict that accepts the recommendation, in any case.
const ACCEPT_WORD: &str = "ok";

/// The word of the marker line that says the answers are ready.
const ANSWERED_WORD: &str = "ANSWERED";

/// The marker line as it is written.
const MARKER_LINE: &str = "**ANSWERED**";

/// The start of a bold label or of the marker line: `**`.
const BOLD: &str = "**";

/// The start of the title line.
const TITLE_PREFIX: &str = "# ";

/// The start of a question's line, before its number.
const QUESTION_PREFIX: &str = "## ";

/// A parsed or composed questions file.
///
/// Made only by [`QuestionsFile::new`] or [`QuestionsFile::parse`], so it has at least one question and no two
/// questions have the same number. Rendering it and parsing the result gives an equal file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuestionsFile {
    /// The text of the `# ` line at the top, if the file has one.
    title: Option<String>,
    /// The instructions for the human: the block quote under the title, without its `> `; empty when there is none.
    instructions: String,
    /// The text between the instructions and the first question; empty when there is none.
    intro: String,
    /// The questions, in the order written.
    entries: Vec<Entry>,
    /// Whether the last non-empty line was the `**ANSWERED**` marker.
    answered: bool,
}

/// One question of a questions file: `## N. title`, its text and its labelled parts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// The number in `## N.`: unique in its file.
    number: NonZeroU32,
    /// The rest of the `## N.` line.
    title: String,
    /// The text between the title and the first labelled part (the options, the context); empty when there is none.
    body: String,
    /// The labelled parts, in the order written; at most one `Recommended` and one `Verdict`.
    parts: Vec<Part>,
}

/// A labelled part of a question: the label's line and the lines that follow it up to the next label.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Part {
    /// `**Recommended:**` and its text, which is empty when the agent wrote none.
    Recommended(String),
    /// `**Verdict:**` and the human's answer; `None` while the part is empty.
    Verdict(Option<Verdict>),
    /// Any other `**Label:**`, kept as it is.
    Other {
        /// The label's words, without the asterisks and the colon.
        label: String,
        /// The text of the part.
        text: String,
    },
}

/// The human's answer to one question.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    /// `ok`: the human accepts the recommendation.
    Accept,
    /// Any other text: the human's own answer.
    Own(OwnAnswer),
}

/// A verdict is written as its text — `ok`, or the own answer — so that a client can send what the human typed.
impl serde::Serialize for Verdict {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(match self {
            Self::Accept => ACCEPT_WORD,
            Self::Own(answer) => answer.as_str(),
        })
    }
}

impl<'de> serde::Deserialize<'de> for Verdict {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        Self::from_text(&text)
            .map_err(serde::de::Error::custom)?
            .ok_or_else(|| serde::de::Error::custom(OwnAnswerError::Blank))
    }
}

/// The text of an own answer: not blank, not `ok`, and no line that the file format would read as structure.
///
/// Made by [`Verdict::from_text`] or [`TryFrom<String>`]; the checks keep an answer inside its `**Verdict:**` part
/// when the file is written and read again.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OwnAnswer(String);

impl OwnAnswer {
    /// The answer's text, trimmed.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A line of an answer that the file format would read as the start of a question, a label or the `ANSWERED` marker.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("line {line} of the answer would be read as part of the file's structure")]
pub struct StructuralLine {
    /// The line, counted from 1.
    pub line: NonZeroUsize,
}

/// Why a text is not an own answer.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum OwnAnswerError {
    /// The text is empty or only whitespace: that is no answer.
    #[error("an answer must not be blank")]
    Blank,
    /// The text is `ok`, which accepts the recommendation and is a [`Verdict::Accept`].
    #[error("`ok` accepts the recommendation: it is not an own answer")]
    IsOk,
    /// This line (of the trimmed text) would be read as the start of a question, a label or the `ANSWERED` marker.
    #[error("line {line} of the answer would be read as part of the file's structure")]
    Structural {
        /// The line, counted from 1.
        line: NonZeroUsize,
    },
}

impl TryFrom<String> for OwnAnswer {
    type Error = OwnAnswerError;

    fn try_from(text: String) -> Result<Self, Self::Error> {
        let text = text.trim();
        if text.is_empty() {
            return Err(OwnAnswerError::Blank);
        }
        if text.eq_ignore_ascii_case(ACCEPT_WORD) {
            return Err(OwnAnswerError::IsOk);
        }
        if let Some(line) = first_structural_line(text) {
            return Err(OwnAnswerError::Structural { line });
        }
        Ok(Self(text.to_owned()))
    }
}

impl Verdict {
    /// Reads the text of a verdict: `None` for a blank text (no answer yet), [`Verdict::Accept`] for `ok` in any case,
    /// otherwise the trimmed text as an own answer.
    ///
    /// # Errors
    /// [`StructuralLine`] when a line of the text would be read as the start of a question, a label or the
    /// `ANSWERED` marker.
    pub fn from_text(text: &str) -> Result<Option<Self>, StructuralLine> {
        let text = text.trim();
        match first_structural_line(text) {
            Some(line) => Err(StructuralLine { line }),
            None => Ok(Self::from_section(text)),
        }
    }

    /// Reads the trimmed text of a verdict part, which the parser has already split at every label.
    fn from_section(text: &str) -> Option<Self> {
        if text.is_empty() {
            None
        } else if text.eq_ignore_ascii_case(ACCEPT_WORD) {
            Some(Self::Accept)
        } else {
            Some(Self::Own(OwnAnswer(text.to_owned())))
        }
    }
}

/// Why [`QuestionsFile::parse`] refused a text.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ParseError {
    /// The text is longer than [`MAX_QUESTIONS_BYTES`].
    #[error("the questions file has {len} bytes; the limit is {max}")]
    TooLarge {
        /// The text's length, in bytes.
        len: usize,
        /// The limit.
        max: usize,
    },
    /// The text has no line of the form `## N. title`.
    #[error("the file has no questions in the \"## N. title\" format")]
    NoQuestions,
    /// The number of a `## N.` line is 0 or does not fit in 32 bits.
    #[error("line {line}: {digits:?} is not a question number (1 to 4294967295)")]
    InvalidNumber {
        /// The line, counted from 1.
        line: NonZeroUsize,
        /// The digits that were read.
        digits: String,
        /// Why the digits are not a number in that range.
        source: ParseIntError,
    },
    /// Two questions have the same number.
    #[error("line {line}: question {number} appears twice")]
    DuplicateNumber {
        /// The line of the second one, counted from 1.
        line: NonZeroUsize,
        /// The repeated number.
        number: NonZeroU32,
    },
    /// A question has two `**Recommended:**` or two `**Verdict:**` parts.
    #[error("line {line}: question {number} has a second **{label}:** part")]
    DuplicateLabel {
        /// The line of the second one, counted from 1.
        line: NonZeroUsize,
        /// The number of the question.
        number: NonZeroU32,
        /// The repeated label: `Recommended` or `Verdict`.
        label: &'static str,
    },
}

/// Why [`QuestionsFile::new`] refused its arguments.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ComposeError {
    /// The title is blank or has more than one line.
    #[error("the title must be one non-blank line")]
    InvalidTitle,
    /// There are no questions.
    #[error("a questions file needs at least one question")]
    NoQuestions,
    /// Two questions have the same number.
    #[error("question {number} appears twice")]
    DuplicateNumber {
        /// The repeated number.
        number: NonZeroU32,
    },
}

/// The part of an [`Entry`] that a text goes into.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntryField {
    /// The text between the title and the first label.
    Body,
    /// The text of the `**Recommended:**` part.
    Recommendation,
}

/// Why [`Entry::new`] or [`Entry::recommend`] refused a text.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum EntryError {
    /// The title is blank or has more than one line.
    #[error("the title must be one non-blank line")]
    InvalidTitle,
    /// The recommendation is blank.
    #[error("the recommendation must not be blank")]
    BlankRecommendation,
    /// This line of `field` would be read as the start of a question, a label or the `ANSWERED` marker.
    #[error("line {line} of the {field:?} would be read as part of the file's structure")]
    Structural {
        /// Which text it is in.
        field: EntryField,
        /// The line, counted from 1.
        line: NonZeroUsize,
    },
}

/// The question that [`QuestionsFile::set_verdict`] was asked to answer is not in the file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("the file has no question {number}")]
pub struct UnknownQuestion {
    /// The number asked for.
    pub number: NonZeroU32,
}

/// Why a question cannot be answered from its verdict.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnswerProblem {
    /// The question has no `**Verdict:**` part, or it is empty.
    MissingVerdict,
    /// The verdict is `ok` but the question has no recommendation to accept.
    AcceptWithoutRecommendation,
}

/// The questions of a file that cannot be answered from their verdicts, each with its problem, in file order.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{} question(s) are not answered properly", .problems.len())]
pub struct AnswersError {
    /// The questions and what is wrong with each; never empty.
    pub problems: Vec<(NonZeroU32, AnswerProblem)>,
}

/// What the human answered to one question.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Answer {
    /// The question's number.
    pub number: NonZeroU32,
    /// The question's title.
    pub title: String,
    /// The answer.
    pub choice: Choice,
}

/// An answer: the recommendation, or the human's own text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Choice {
    /// The human wrote `ok`: this is the recommendation they accepted.
    Recommendation(String),
    /// The human's own answer.
    Own(String),
}

impl Entry {
    /// A question with a title and a text (the options, the context), and an empty `**Verdict:**` part for the human
    /// to fill.
    ///
    /// # Errors
    /// [`EntryError::InvalidTitle`] for a blank title or one with more than one line;
    /// [`EntryError::Structural`] when a line of `body` would be read as the start of a question, a label or the
    /// `ANSWERED` marker.
    pub fn new(number: NonZeroU32, title: &str, body: &str) -> Result<Self, EntryError> {
        let title = title.trim();
        if title.is_empty() || title.contains(['\n', '\r']) {
            return Err(EntryError::InvalidTitle);
        }
        if let Some(line) = first_structural_line(body) {
            return Err(EntryError::Structural {
                field: EntryField::Body,
                line,
            });
        }
        Ok(Self {
            number,
            title: title.to_owned(),
            body: body.trim().to_owned(),
            parts: vec![Part::Verdict(None)],
        })
    }

    /// The question with a `**Recommended:**` part, in place of any it had.
    ///
    /// # Errors
    /// [`EntryError::BlankRecommendation`] for a blank text; [`EntryError::Structural`] when a line of it would be
    /// read as the start of a question, a label or the `ANSWERED` marker.
    pub fn recommend(mut self, text: &str) -> Result<Self, EntryError> {
        if text.trim().is_empty() {
            return Err(EntryError::BlankRecommendation);
        }
        if let Some(line) = first_structural_line(text) {
            return Err(EntryError::Structural {
                field: EntryField::Recommendation,
                line,
            });
        }
        self.parts.retain(|part| !matches!(part, Part::Recommended(_)));
        self.parts.insert(0, Part::Recommended(text.trim().to_owned()));
        Ok(self)
    }

    /// The question's number.
    #[must_use]
    pub const fn number(&self) -> NonZeroU32 {
        self.number
    }

    /// The rest of the `## N.` line.
    #[must_use]
    pub fn title(&self) -> &str {
        &self.title
    }

    /// The text between the title and the first labelled part; empty when there is none.
    #[must_use]
    pub fn body(&self) -> &str {
        &self.body
    }

    /// The text of the `**Recommended:**` part; `None` when the question has none or it is empty.
    #[must_use]
    pub fn recommendation(&self) -> Option<&str> {
        self.parts.iter().find_map(|part| match part {
            Part::Recommended(text) if !text.is_empty() => Some(text.as_str()),
            Part::Recommended(_) | Part::Verdict(_) | Part::Other { .. } => None,
        })
    }

    /// The human's answer; `None` when the question has no `**Verdict:**` part or it is empty.
    #[must_use]
    pub fn verdict(&self) -> Option<&Verdict> {
        self.parts.iter().find_map(|part| match part {
            Part::Verdict(verdict) => verdict.as_ref(),
            Part::Recommended(_) | Part::Other { .. } => None,
        })
    }
}

impl Entry {
    /// Replaces the verdict part's text, or adds the part when there is something to put in it.
    fn set_verdict(&mut self, verdict: Option<Verdict>) {
        match self.parts.iter_mut().find(|part| matches!(part, Part::Verdict(_))) {
            Some(part) => *part = Part::Verdict(verdict),
            None if verdict.is_some() => self.parts.push(Part::Verdict(verdict)),
            None => {}
        }
    }

    /// The answer the verdict gives, or why it gives none.
    fn answer(&self) -> Result<Choice, AnswerProblem> {
        match (self.verdict(), self.recommendation()) {
            (None, _) => Err(AnswerProblem::MissingVerdict),
            (Some(Verdict::Accept), None) => Err(AnswerProblem::AcceptWithoutRecommendation),
            (Some(Verdict::Accept), Some(recommendation)) => Ok(Choice::Recommendation(recommendation.to_owned())),
            (Some(Verdict::Own(own)), _) => Ok(Choice::Own(own.as_str().to_owned())),
        }
    }

    /// The question as text for `rendering`: its line, its text and its parts, each part after a blank line.
    fn render_as(&self, rendering: &Rendering<'_>) -> String {
        let mut out = if self.title.is_empty() {
            format!("{QUESTION_PREFIX}{}.\n", self.number)
        } else {
            format!("{QUESTION_PREFIX}{}. {}\n", self.number, self.title)
        };
        if !self.body.is_empty() {
            out.push_str(&self.body);
            out.push('\n');
        }
        for part in &self.parts {
            out.push('\n');
            out.push_str(&match part {
                Part::Recommended(text) => labelled(RECOMMENDED_LABEL, text),
                Part::Verdict(verdict) => labelled(VERDICT_LABEL, &self.verdict_text(verdict.as_ref(), rendering)),
                Part::Other { label, text } => labelled(label, text),
            });
            out.push('\n');
        }
        out
    }

    /// The text of a verdict part for `rendering`: for the agent, `ok` becomes the words that say the human accepted
    /// the recommendation.
    fn verdict_text(&self, verdict: Option<&Verdict>, rendering: &Rendering<'_>) -> String {
        match (verdict, rendering) {
            (None, _) => String::new(),
            (Some(Verdict::Own(own)), _) => own.as_str().to_owned(),
            (Some(Verdict::Accept), Rendering::Human) => ACCEPT_WORD.to_owned(),
            (Some(Verdict::Accept), Rendering::Agent { accepted }) => {
                accepted(self.recommendation().unwrap_or_default())
            }
        }
    }
}

impl QuestionsFile {
    /// A questions file with a title and questions, not yet answered and without instructions.
    ///
    /// # Errors
    /// [`ComposeError`]: the title is blank or has more than one line, there are no questions, or two have the same
    /// number.
    pub fn new(title: &str, entries: Vec<Entry>) -> Result<Self, ComposeError> {
        let title = title.trim();
        if title.is_empty() || title.contains(['\n', '\r']) {
            return Err(ComposeError::InvalidTitle);
        }
        if entries.is_empty() {
            return Err(ComposeError::NoQuestions);
        }
        let mut seen = HashSet::new();
        if let Some(entry) = entries.iter().find(|entry| !seen.insert(entry.number)) {
            return Err(ComposeError::DuplicateNumber { number: entry.number });
        }
        Ok(Self {
            title: Some(title.to_owned()),
            instructions: String::new(),
            intro: String::new(),
            entries,
            answered: false,
        })
    }

    /// Parses the text of a questions file.
    ///
    /// # Errors
    /// [`ParseError`]: the text is over [`MAX_QUESTIONS_BYTES`], has no questions, a question number that is 0 or too
    /// large, two questions with one number, or a question with two `**Recommended:**` or two `**Verdict:**` parts.
    pub fn parse(text: &str) -> Result<Self, ParseError> {
        if text.len() > MAX_QUESTIONS_BYTES {
            return Err(ParseError::TooLarge {
                len: text.len(),
                max: MAX_QUESTIONS_BYTES,
            });
        }
        let text = text.strip_prefix('\u{feff}').unwrap_or(text);
        let mut preamble: Vec<&str> = Vec::new();
        let mut drafts: Vec<Draft<'_>> = Vec::new();
        let mut seen = HashSet::new();
        for (index, line) in text.lines().enumerate() {
            let at = NonZeroUsize::MIN.saturating_add(index);
            if let Some(header) = read_header(line) {
                let number = header.number(at)?;
                if !seen.insert(number) {
                    return Err(ParseError::DuplicateNumber { line: at, number });
                }
                drafts.push(Draft::new(number, header.title));
            } else if is_marker(line) {
                // Wherever it stands, the marker is not part of a question's text.
            } else if let Some(draft) = drafts.last_mut() {
                draft.push_line(line, at)?;
            } else {
                preamble.push(line);
            }
        }
        if drafts.is_empty() {
            return Err(ParseError::NoQuestions);
        }
        let Preamble {
            title,
            instructions,
            intro,
        } = read_preamble(&preamble);
        Ok(Self {
            title,
            instructions,
            intro,
            entries: drafts.into_iter().map(Draft::finish).collect(),
            answered: text
                .lines()
                .rev()
                .find(|line| !line.trim().is_empty())
                .is_some_and(is_marker),
        })
    }

    /// The file as text. Parsing it gives an equal file.
    #[must_use]
    pub fn render(&self) -> String {
        self.render_as(&Rendering::Human)
    }

    /// The file as text for `rendering`.
    fn render_as(&self, rendering: &Rendering<'_>) -> String {
        let mut blocks: Vec<String> = Vec::new();
        if let Some(title) = &self.title {
            blocks.push(format!("{TITLE_PREFIX}{title}"));
        }
        if matches!(rendering, Rendering::Human) && !self.instructions.is_empty() {
            blocks.push(quote(&self.instructions));
        }
        if !self.intro.is_empty() {
            blocks.push(self.intro.clone());
        }
        let mut out = blocks.join("\n\n");
        if !out.is_empty() {
            out.push_str("\n\n");
        }
        let entries: Vec<String> = self.entries.iter().map(|entry| entry.render_as(rendering)).collect();
        out.push_str(&entries.join("\n"));
        if matches!(rendering, Rendering::Human) && self.answered {
            out.push('\n');
            out.push_str(MARKER_LINE);
            out.push('\n');
        }
        out
    }

    /// The title, if the file has one.
    #[must_use]
    pub fn title(&self) -> Option<&str> {
        self.title.as_deref()
    }

    /// The instructions for the human; empty when there are none.
    #[must_use]
    pub fn instructions(&self) -> &str {
        &self.instructions
    }

    /// The free text between the instructions and the first question; empty when there is none.
    #[must_use]
    pub fn intro(&self) -> &str {
        &self.intro
    }

    /// The questions, in the order written.
    #[must_use]
    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    /// The question numbered `number`, if the file has it.
    #[must_use]
    pub fn entry(&self, number: NonZeroU32) -> Option<&Entry> {
        self.entries.iter().find(|entry| entry.number == number)
    }

    /// Whether the last non-empty line was the `**ANSWERED**` marker: the human says they are done.
    #[must_use]
    pub const fn is_answered(&self) -> bool {
        self.answered
    }

    /// Puts `text` under the title as the instructions, in place of any there are; a blank text removes them. Doing
    /// it twice with the same text changes nothing.
    pub fn set_instructions(&mut self, text: &str) {
        text.trim().clone_into(&mut self.instructions);
    }

    /// Sets the verdict of a question; `None` empties it.
    ///
    /// # Errors
    /// [`UnknownQuestion`] when the file has no question `number`.
    pub fn set_verdict(&mut self, number: NonZeroU32, verdict: Option<Verdict>) -> Result<(), UnknownQuestion> {
        let entry = self
            .entries
            .iter_mut()
            .find(|entry| entry.number == number)
            .ok_or(UnknownQuestion { number })?;
        entry.set_verdict(verdict);
        Ok(())
    }

    /// Puts the `**ANSWERED**` marker on the last line (`true`) or takes it away (`false`).
    pub const fn mark_answered(&mut self, answered: bool) {
        self.answered = answered;
    }

    /// The numbers of the questions without a recommendation, in file order.
    #[must_use]
    pub fn missing_recommendations(&self) -> Vec<NonZeroU32> {
        self.entries
            .iter()
            .filter(|entry| entry.recommendation().is_none())
            .map(Entry::number)
            .collect()
    }

    /// What the human answered, question by question.
    ///
    /// # Errors
    /// [`AnswersError`] listing every question without a verdict and every `ok` without a recommendation.
    pub fn answers(&self) -> Result<Vec<Answer>, AnswersError> {
        let mut answers = Vec::with_capacity(self.entries.len());
        let mut problems = Vec::new();
        for entry in &self.entries {
            match entry.answer() {
                Ok(choice) => answers.push(Answer {
                    number: entry.number,
                    title: entry.title.clone(),
                    choice,
                }),
                Err(problem) => problems.push((entry.number, problem)),
            }
        }
        if problems.is_empty() {
            Ok(answers)
        } else {
            Err(AnswersError { problems })
        }
    }

    /// The answered file as the agent gets it: without the instructions and the marker, and with the verdict `ok`
    /// replaced by `accepted(recommendation)` — the words that tell the agent the human accepted it.
    ///
    /// # Errors
    /// [`AnswersError`] as for [`QuestionsFile::answers`]: an unanswered file is not handed to the agent.
    pub fn expand(&self, accepted: impl Fn(&str) -> String) -> Result<String, AnswersError> {
        self.answers()?;
        Ok(self.render_as(&Rendering::Agent { accepted: &accepted }))
    }
}

/// Who a file is rendered for.
enum Rendering<'a> {
    /// The human: the instructions, the verdicts as written and the marker.
    Human,
    /// The agent that asked: no instructions and no marker, and `accepted` turns the recommendation of a question
    /// whose verdict is `ok` into the words that tell the agent it was accepted.
    Agent {
        /// Makes the words of an accepted recommendation.
        accepted: &'a dyn Fn(&str) -> String,
    },
}

/// A question being read: its lines, not yet told apart.
struct Draft<'a> {
    /// The question's number.
    number: NonZeroU32,
    /// The rest of the `## N.` line.
    title: &'a str,
    /// The lines before the first label.
    body: Vec<&'a str>,
    /// The labelled parts, in the order written.
    sections: Vec<Section<'a>>,
}

/// A labelled part being read.
struct Section<'a> {
    /// Which label starts it.
    kind: SectionKind<'a>,
    /// The text on the label's line, then the lines up to the next label.
    lines: Vec<&'a str>,
}

/// The label of a part.
#[derive(Clone, Copy, PartialEq, Eq)]
enum SectionKind<'a> {
    /// `**Recommended:**`.
    Recommended,
    /// `**Verdict:**`.
    Verdict,
    /// Any other label: its words.
    Other(&'a str),
}

impl<'a> Draft<'a> {
    /// A question that has only its `## N.` line so far.
    const fn new(number: NonZeroU32, title: &'a str) -> Self {
        Self {
            number,
            title,
            body: Vec::new(),
            sections: Vec::new(),
        }
    }

    /// Takes a line of the question: a label starts a part, any other line belongs to the part before it.
    fn push_line(&mut self, line: &'a str, at: NonZeroUsize) -> Result<(), ParseError> {
        let Some((label, rest)) = split_label(line) else {
            match self.sections.last_mut() {
                Some(section) => section.lines.push(line),
                None => self.body.push(line),
            }
            return Ok(());
        };
        let (kind, repeated) = match label {
            RECOMMENDED_LABEL => (SectionKind::Recommended, Some(RECOMMENDED_LABEL)),
            VERDICT_LABEL => (SectionKind::Verdict, Some(VERDICT_LABEL)),
            other => (SectionKind::Other(other), None),
        };
        if let Some(label) = repeated.filter(|_| self.sections.iter().any(|section| section.kind == kind)) {
            return Err(ParseError::DuplicateLabel {
                line: at,
                number: self.number,
                label,
            });
        }
        self.sections.push(Section {
            kind,
            lines: vec![rest],
        });
        Ok(())
    }

    /// The question, its lines told apart.
    fn finish(self) -> Entry {
        let parts = self
            .sections
            .into_iter()
            .map(|section| {
                let text = trimmed(&section.lines);
                match section.kind {
                    SectionKind::Recommended => Part::Recommended(text),
                    SectionKind::Verdict => Part::Verdict(Verdict::from_section(&text)),
                    SectionKind::Other(label) => Part::Other {
                        label: label.to_owned(),
                        text,
                    },
                }
            })
            .collect();
        Entry {
            number: self.number,
            title: self.title.to_owned(),
            body: trimmed(&self.body),
            parts,
        }
    }
}

/// A `## N. title` line, read but not yet checked.
struct Header<'a> {
    /// The digits of the number.
    digits: &'a str,
    /// The text after the dot, without the spaces around it.
    title: &'a str,
}

impl Header<'_> {
    /// The number, which must be 1 or more and fit in 32 bits.
    fn number(&self, at: NonZeroUsize) -> Result<NonZeroU32, ParseError> {
        self.digits.parse().map_err(|source| ParseError::InvalidNumber {
            line: at,
            digits: self.digits.to_owned(),
            source,
        })
    }
}

/// What comes before the first question.
struct Preamble {
    /// The `# ` line.
    title: Option<String>,
    /// The block quote under it, unquoted.
    instructions: String,
    /// Everything else.
    intro: String,
}

/// Reads the lines before the first question: an optional title, an optional block quote, free text.
fn read_preamble(lines: &[&str]) -> Preamble {
    let mut lines = lines
        .iter()
        .copied()
        .skip_while(|line| line.trim().is_empty())
        .peekable();
    let title = lines
        .next_if(|line| line.starts_with(TITLE_PREFIX))
        .and_then(|line| line.strip_prefix(TITLE_PREFIX))
        .map(|title| title.trim().to_owned());
    while lines.next_if(|line| line.trim().is_empty()).is_some() {}
    let mut quoted: Vec<&str> = Vec::new();
    while let Some(line) = lines.next_if(|line| line.starts_with('>')) {
        let unquoted = line.strip_prefix('>').unwrap_or(line);
        quoted.push(unquoted.strip_prefix(' ').unwrap_or(unquoted));
    }
    let intro: Vec<&str> = lines.collect();
    Preamble {
        title,
        instructions: trimmed(&quoted),
        intro: trimmed(&intro),
    }
}

/// `## N. title` read as a question's line; `None` for any other line.
fn read_header(line: &str) -> Option<Header<'_>> {
    let (digits, title) = line.strip_prefix(QUESTION_PREFIX)?.split_once('.')?;
    (!digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit())).then_some(Header {
        digits,
        title: title.trim(),
    })
}

/// `**Label:** text` read as a label and its text; `None` for any other line. The label is words and spaces, and
/// spaces inside the asterisks are tolerated.
fn split_label(line: &str) -> Option<(&str, &str)> {
    let rest = line.strip_prefix(BOLD)?.trim_start();
    if !rest.chars().next()?.is_alphabetic() {
        return None;
    }
    let end = rest.find(|c: char| !(c.is_alphabetic() || c == ' '))?;
    let (label, rest) = rest.split_at_checked(end)?;
    let text = rest.strip_prefix(':')?.trim_start().strip_prefix(BOLD)?;
    Some((label.trim_end(), text.trim_start()))
}

/// Whether the line is the `**ANSWERED**` marker alone.
fn is_marker(line: &str) -> bool {
    line.trim()
        .strip_prefix(BOLD)
        .and_then(|rest| rest.trim_start().strip_prefix(ANSWERED_WORD))
        .and_then(|rest| rest.trim_start().strip_prefix(BOLD))
        .is_some_and(str::is_empty)
}

/// Whether the parser would read the line as the start of a question, a label or the marker.
fn is_structural(line: &str) -> bool {
    read_header(line).is_some() || split_label(line).is_some() || is_marker(line)
}

/// The first line of `text` that [`is_structural`], counted from 1.
fn first_structural_line(text: &str) -> Option<NonZeroUsize> {
    text.lines()
        .position(is_structural)
        .map(|index| NonZeroUsize::MIN.saturating_add(index))
}

/// The lines joined, without the empty lines and spaces at both ends.
fn trimmed(lines: &[&str]) -> String {
    lines.join("\n").trim().to_owned()
}

/// A labelled part: the label and, when there is any, its text on the same line.
fn labelled(label: &str, text: &str) -> String {
    if text.is_empty() {
        format!("{BOLD}{label}:{BOLD}")
    } else {
        format!("{BOLD}{label}:{BOLD} {text}")
    }
}

/// The text as a block quote: each line after `> `, an empty line as `>`.
fn quote(text: &str) -> String {
    text.lines()
        .map(|line| {
            if line.is_empty() {
                String::from(">")
            } else {
                format!("> {line}")
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

impl fmt::Display for AnswerProblem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::MissingVerdict => "no verdict",
            Self::AcceptWithoutRecommendation => "`ok` but no recommendation to accept",
        })
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    /// A question number from a literal.
    fn n(number: u32) -> NonZeroU32 {
        NonZeroU32::new(number).unwrap()
    }

    /// A line number from a literal.
    fn line(number: usize) -> NonZeroUsize {
        NonZeroUsize::new(number).unwrap()
    }

    /// An own answer from a literal.
    fn own(text: &str) -> Verdict {
        Verdict::from_text(text).unwrap().unwrap()
    }

    /// The questions file the agents are told to write: numbered questions, options, a recommendation and an empty
    /// verdict.
    const AGENT_FILE: &str = "\
# Questions — task-20261111-153601-k3x9

## 1. Which store?
a) in memory
b) on disk

**Recommended:** a) — simpler (inv-opus)

**Verdict:**

## 2. Żółć or bile?
Free text about the second question.

**Recommended:** bile — the \"żółć\" key is\ntaken

**Verdict:**
";

    // ---- reading the verdict -------------------------------------------------------------------------------------

    #[test]
    fn a_blank_verdict_is_no_answer() {
        for text in ["", " ", "\n", " \t\n "] {
            assert_eq!(Verdict::from_text(text), Ok(None), "{text:?}");
        }
    }

    #[test]
    fn ok_in_any_case_and_padded_accepts_the_recommendation() {
        for text in ["ok", "OK", "Ok", "oK", "  ok  ", "\nok\n"] {
            assert_eq!(Verdict::from_text(text), Ok(Some(Verdict::Accept)), "{text:?}");
        }
    }

    #[test]
    fn any_other_text_is_an_own_answer_trimmed() {
        let verdict = Verdict::from_text("  b) on disk, but only for the cache \n")
            .unwrap()
            .unwrap();

        let Verdict::Own(answer) = verdict else {
            panic!("not an own answer");
        };
        assert_eq!(answer.as_str(), "b) on disk, but only for the cache");
    }

    #[test]
    fn text_that_only_starts_with_ok_is_an_own_answer() {
        for text in ["ok, but with a cache", "okay", "oko", "not ok"] {
            assert!(
                matches!(Verdict::from_text(text), Ok(Some(Verdict::Own(_)))),
                "{text:?}"
            );
        }
    }

    #[test]
    fn an_own_answer_keeps_its_lines_and_non_ascii_text() {
        let verdict = own("Żółć\n\n  and then bile\n");

        let Verdict::Own(answer) = verdict else {
            panic!("not an own answer");
        };
        assert_eq!(answer.as_str(), "Żółć\n\n  and then bile");
    }

    #[test]
    fn an_answer_with_a_structural_line_is_refused_with_its_line() {
        let cases: [(&str, usize); 6] = [
            ("fine\n## 3. A new question", 2),
            ("**Verdict:** again", 1),
            ("one\ntwo\n**Note:**", 3),
            ("**ANSWERED**", 1),
            ("  ** ANSWERED ** ", 1),
            ("x\n**Zażółć gęślą:** x", 2),
        ];
        for (text, at) in cases {
            assert_eq!(
                Verdict::from_text(text),
                Err(StructuralLine { line: line(at) }),
                "{text:?}"
            );
        }
    }

    #[test]
    fn an_own_answer_is_made_only_from_a_good_text() {
        assert_eq!(OwnAnswer::try_from(String::from(" yes ")).unwrap().as_str(), "yes");
        assert_eq!(OwnAnswer::try_from(String::from("  ")), Err(OwnAnswerError::Blank));
        assert_eq!(OwnAnswer::try_from(String::from("OK")), Err(OwnAnswerError::IsOk));
        assert_eq!(
            OwnAnswer::try_from(String::from("a\n## 1. b")),
            Err(OwnAnswerError::Structural { line: line(2) })
        );
    }

    // ---- verdicts as JSON ---------------------------------------------------------------------------------------

    #[test]
    fn a_verdict_is_written_as_its_text() {
        assert_eq!(serde_json::to_value(Verdict::Accept).unwrap(), json!("ok"));
        assert_eq!(
            serde_json::to_value(own("b) on disk\nbecause")).unwrap(),
            json!("b) on disk\nbecause")
        );
    }

    #[test]
    fn a_verdict_is_read_from_its_text_with_ok_in_any_case_accepting() {
        for (text, expected) in [
            ("ok", Verdict::Accept),
            ("  OK ", Verdict::Accept),
            (" Żółć ", own("Żółć")),
        ] {
            let read: Verdict = serde_json::from_value(json!(text)).unwrap();

            assert_eq!(read, expected, "{text:?}");
            let again: Verdict = serde_json::from_value(serde_json::to_value(&read).unwrap()).unwrap();
            assert_eq!(again, read);
        }
    }

    #[test]
    fn a_blank_or_structural_or_non_text_verdict_is_refused_when_read() {
        for bad in [
            json!(""),
            json!("  "),
            json!("x\n## 2. y"),
            json!("**Verdict:** z"),
            json!(null),
            json!(5),
            json!(["ok"]),
        ] {
            let err = serde_json::from_value::<Verdict>(bad.clone()).unwrap_err();

            assert_eq!(err.classify(), serde_json::error::Category::Data, "{bad}");
        }
    }

    // ---- composing ----------------------------------------------------------------------------------------------

    #[test]
    fn an_entry_has_its_number_title_text_and_an_empty_verdict() {
        let entry = Entry::new(n(3), "  Which store? ", "a) memory\nb) disk\n").unwrap();

        assert_eq!(entry.number(), n(3));
        assert_eq!(entry.title(), "Which store?");
        assert_eq!(entry.body(), "a) memory\nb) disk");
        assert_eq!(entry.recommendation(), None);
        assert_eq!(entry.verdict(), None);
    }

    #[test]
    fn an_entry_refuses_a_blank_or_multiline_title() {
        for title in ["", "  ", "\n", "one\ntwo", "one\r\ntwo"] {
            assert_eq!(Entry::new(n(1), title, ""), Err(EntryError::InvalidTitle), "{title:?}");
        }
    }

    #[test]
    fn an_entry_refuses_a_body_line_that_is_structure() {
        for (body, at) in [
            ("text\n## 2. not a new question", 2),
            ("**Recommended:** x", 1),
            ("a\nb\n**Verdict:**", 3),
            ("**ANSWERED**", 1),
        ] {
            assert_eq!(
                Entry::new(n(1), "Q", body),
                Err(EntryError::Structural {
                    field: EntryField::Body,
                    line: line(at)
                }),
                "{body:?}"
            );
        }
    }

    #[test]
    fn a_recommendation_is_trimmed_and_may_have_several_lines() {
        let entry = Entry::new(n(1), "Q", "")
            .unwrap()
            .recommend("  a) why\nand more \n")
            .unwrap();

        assert_eq!(entry.recommendation(), Some("a) why\nand more"));
    }

    #[test]
    fn a_recommendation_refuses_blank_text_and_structure() {
        let entry = Entry::new(n(1), "Q", "").unwrap();

        assert_eq!(entry.clone().recommend(" \n"), Err(EntryError::BlankRecommendation));
        assert_eq!(
            entry.recommend("fine\n**Verdict:** x"),
            Err(EntryError::Structural {
                field: EntryField::Recommendation,
                line: line(2)
            })
        );
    }

    #[test]
    fn a_file_needs_a_one_line_title_questions_and_unique_numbers() {
        let q = |number| Entry::new(n(number), "Q", "").unwrap();

        assert_eq!(QuestionsFile::new("", vec![q(1)]), Err(ComposeError::InvalidTitle));
        assert_eq!(QuestionsFile::new("a\nb", vec![q(1)]), Err(ComposeError::InvalidTitle));
        assert_eq!(QuestionsFile::new("T", vec![]), Err(ComposeError::NoQuestions));
        assert_eq!(
            QuestionsFile::new("T", vec![q(1), q(2), q(1)]),
            Err(ComposeError::DuplicateNumber { number: n(1) })
        );
        let file = QuestionsFile::new(" T ", vec![q(2), q(1)]).unwrap();
        assert_eq!(file.title(), Some("T"));
        let numbers: Vec<u32> = file.entries().iter().map(|e| e.number().get()).collect();
        assert_eq!(numbers, [2, 1]);
        assert!(!file.is_answered());
        assert_eq!(file.instructions(), "");
    }

    // ---- rendering ----------------------------------------------------------------------------------------------

    #[test]
    fn a_composed_file_renders_in_the_format_the_agents_write() {
        let first = Entry::new(n(1), "Which store?", "a) in memory\nb) on disk")
            .unwrap()
            .recommend("a) — simpler (inv-opus)")
            .unwrap();
        let second = Entry::new(n(2), "Sure?", "").unwrap();
        let file = QuestionsFile::new("Questions — task-1", vec![first, second]).unwrap();

        assert_eq!(
            file.render(),
            "\
# Questions — task-1

## 1. Which store?
a) in memory
b) on disk

**Recommended:** a) — simpler (inv-opus)

**Verdict:**

## 2. Sure?

**Verdict:**
"
        );
    }

    #[test]
    fn instructions_are_a_block_quote_under_the_title() {
        let mut file = QuestionsFile::parse(AGENT_FILE).unwrap();

        file.set_instructions("Answer every question.\n\nThen save.");

        assert_eq!(file.instructions(), "Answer every question.\n\nThen save.");
        assert!(file.render().starts_with(
            "# Questions — task-20261111-153601-k3x9\n\n> Answer every question.\n>\n> Then save.\n\n## 1."
        ));
    }

    #[test]
    fn instructions_set_twice_or_replaced_stay_one_block() {
        let mut file = QuestionsFile::parse(AGENT_FILE).unwrap();
        file.set_instructions("First.");
        let once = file.render();

        file.set_instructions("First.");
        assert_eq!(file.render(), once);
        file.set_instructions("Second.");

        assert_eq!(file.instructions(), "Second.");
        assert_eq!(file.render().matches("> ").count(), 1);
        assert!(!file.render().contains("First."));
    }

    #[test]
    fn blank_instructions_remove_them() {
        let mut file = QuestionsFile::parse(AGENT_FILE).unwrap();
        file.set_instructions("Answer.");

        file.set_instructions("  \n ");

        assert_eq!(file.instructions(), "");
        assert_eq!(file.render(), QuestionsFile::parse(AGENT_FILE).unwrap().render());
    }

    #[test]
    fn instructions_go_first_in_a_file_without_a_title() {
        let mut file = QuestionsFile::parse("## 1. Q\n\n**Verdict:**\n").unwrap();

        file.set_instructions("Answer.");

        assert_eq!(file.render(), "> Answer.\n\n## 1. Q\n\n**Verdict:**\n");
        assert_eq!(QuestionsFile::parse(&file.render()).unwrap(), file);
    }

    #[test]
    fn the_marker_is_the_last_line_and_can_be_taken_away() {
        let mut file = QuestionsFile::parse(AGENT_FILE).unwrap();

        file.mark_answered(true);
        assert!(file.is_answered());
        assert!(file.render().ends_with("**Verdict:**\n\n**ANSWERED**\n"));
        file.mark_answered(true);
        assert_eq!(file.render().matches("**ANSWERED**").count(), 1);
        file.mark_answered(false);

        assert!(!file.is_answered());
        assert!(!file.render().contains("ANSWERED"));
    }

    // ---- parsing ------------------------------------------------------------------------------------------------

    #[test]
    fn the_file_agents_write_parses_into_its_questions() {
        let file = QuestionsFile::parse(AGENT_FILE).unwrap();

        assert_eq!(file.title(), Some("Questions — task-20261111-153601-k3x9"));
        assert_eq!(file.instructions(), "");
        assert_eq!(file.intro(), "");
        assert!(!file.is_answered());
        let [first, second] = file.entries() else {
            panic!("expected two questions");
        };
        assert_eq!((first.number(), first.title()), (n(1), "Which store?"));
        assert_eq!(first.body(), "a) in memory\nb) on disk");
        assert_eq!(first.recommendation(), Some("a) — simpler (inv-opus)"));
        assert_eq!(first.verdict(), None);
        assert_eq!((second.number(), second.title()), (n(2), "Żółć or bile?"));
        assert_eq!(second.body(), "Free text about the second question.");
        assert_eq!(second.recommendation(), Some("bile — the \"żółć\" key is\ntaken"));
        assert_eq!(file.entry(n(2)), Some(second));
        assert_eq!(file.entry(n(3)), None);
    }

    #[test]
    fn a_parsed_file_renders_and_parses_back_equal() {
        for text in [
            AGENT_FILE,
            "## 1. Q\n",
            "# T\n\n> note\n> more\n\nIntro text.\n\n## 4. Q\nbody\n\n**Context:** why\nit matters\n\n**Recommended:** x\n\n**Verdict:** ok\n\n**ANSWERED**\n",
            "## 1. Q\n**Verdict:** Żółć\nsecond line\n\n## 2. R\n\n**Verdict:** ok\n",
        ] {
            let file = QuestionsFile::parse(text).unwrap();

            assert_eq!(QuestionsFile::parse(&file.render()).unwrap(), file, "{text:?}");
        }
    }

    #[test]
    fn a_file_may_start_without_a_title_and_have_free_text_before_the_questions() {
        let file = QuestionsFile::parse("Some words first.\n\nMore.\n\n## 1. Q\n").unwrap();

        assert_eq!(file.title(), None);
        assert_eq!(file.intro(), "Some words first.\n\nMore.");
    }

    #[test]
    fn the_title_instructions_and_intro_are_told_apart() {
        let file = QuestionsFile::parse("\n# T\n\n>  First line\n>\n> Third\n\nIntro\n\n## 1. Q\n").unwrap();

        assert_eq!(file.title(), Some("T"));
        assert_eq!(file.instructions(), "First line\n\nThird");
        assert_eq!(file.intro(), "Intro");
    }

    #[test]
    fn labels_are_read_with_spaces_inside_the_asterisks() {
        let file = QuestionsFile::parse("## 1. Q\n\n**  Recommended :**  x\n\n** Verdict:** ok\n").unwrap();

        let entry = file.entry(n(1)).unwrap();
        assert_eq!(entry.recommendation(), Some("x"));
        assert_eq!(entry.verdict(), Some(&Verdict::Accept));
    }

    #[test]
    fn the_labels_are_case_sensitive_and_only_the_english_words_count() {
        let file =
            QuestionsFile::parse("## 1. Q\n\n**recommended:** x\n\n**Verdict:**\n\n**Odpowiedź:** tak\n").unwrap();

        let entry = file.entry(n(1)).unwrap();
        assert_eq!(entry.recommendation(), None);
        assert_eq!(entry.verdict(), None);
        // The two foreign labels are other parts: kept, not read.
        assert!(file.render().contains("**recommended:** x"));
        assert!(file.render().contains("**Odpowiedź:** tak"));
    }

    #[test]
    fn a_verdict_is_the_text_after_its_label_up_to_the_next_label() {
        let file = QuestionsFile::parse(
            "## 1. Q\n\n**Verdict:**\nown answer\nover two lines\n\n**Note:** not part of it\n\n## 2. R\n**Verdict:** ok\n",
        )
        .unwrap();

        assert_eq!(
            file.entry(n(1)).unwrap().verdict(),
            Some(&own("own answer\nover two lines"))
        );
        assert_eq!(file.entry(n(2)).unwrap().verdict(), Some(&Verdict::Accept));
    }

    #[test]
    fn a_verdict_label_may_come_before_the_recommended_one() {
        let file = QuestionsFile::parse("## 1. Q\n**Verdict:** ok\n**Recommended:** x\n").unwrap();

        let entry = file.entry(n(1)).unwrap();
        assert_eq!(entry.verdict(), Some(&Verdict::Accept));
        assert_eq!(entry.recommendation(), Some("x"));
    }

    #[test]
    fn the_marker_counts_only_as_the_last_non_empty_line() {
        let last = QuestionsFile::parse("## 1. Q\n**Verdict:** ok\n\n  **ANSWERED**  \n\n\n").unwrap();
        let middle = QuestionsFile::parse("## 1. Q\n**ANSWERED**\n**Verdict:** ok\n").unwrap();
        let none = QuestionsFile::parse("## 1. Q\n**Verdict:** ok\n").unwrap();
        let spaced = QuestionsFile::parse("## 1. Q\n** ANSWERED **").unwrap();
        let word = QuestionsFile::parse("## 1. Q\nANSWERED\n").unwrap();

        assert!(last.is_answered());
        assert!(!middle.is_answered());
        assert!(!none.is_answered());
        assert!(spaced.is_answered());
        assert!(!word.is_answered());
        // Wherever it was, the marker is not part of the question's text.
        assert_eq!(middle.entry(n(1)).unwrap().body(), "");
        assert_eq!(middle.entry(n(1)).unwrap().verdict(), Some(&Verdict::Accept));
    }

    #[test]
    fn windows_line_endings_are_read_like_unix_ones() {
        let file =
            QuestionsFile::parse("# T\r\n\r\n## 1. Q\r\nbody\r\n\r\n**Verdict:** ok\r\n\r\n**ANSWERED**\r\n").unwrap();

        assert_eq!(file.entry(n(1)).unwrap().body(), "body");
        assert_eq!(file.entry(n(1)).unwrap().verdict(), Some(&Verdict::Accept));
        assert!(file.is_answered());
    }

    #[test]
    fn a_question_header_is_two_hashes_a_number_and_a_dot() {
        let file =
            QuestionsFile::parse("## 1. A\n## 2 not a header\n##3. neither\n### 3. nor this\n## 04. Zero first\n")
                .unwrap();

        let numbers: Vec<u32> = file.entries().iter().map(|e| e.number().get()).collect();
        assert_eq!(numbers, [1, 4]);
        assert_eq!(
            file.entry(n(1)).unwrap().body(),
            "## 2 not a header\n##3. neither\n### 3. nor this"
        );
        assert_eq!(file.entry(n(4)).unwrap().title(), "Zero first");
    }

    #[test]
    fn a_header_without_a_title_is_a_question() {
        let file = QuestionsFile::parse("## 7.\n").unwrap();

        assert_eq!(file.entry(n(7)).unwrap().title(), "");
    }

    #[test]
    fn a_file_without_questions_is_refused() {
        for text in ["", "# Title only\n", "text\n\n**Verdict:** ok\n", "## no number\n"] {
            assert_eq!(QuestionsFile::parse(text), Err(ParseError::NoQuestions), "{text:?}");
        }
    }

    #[test]
    fn a_zero_or_huge_number_is_refused_with_its_line() {
        assert_eq!(
            QuestionsFile::parse("## 1. A\n\n## 0. B\n"),
            Err(ParseError::InvalidNumber {
                line: line(3),
                digits: String::from("0"),
                source: "0".parse::<NonZeroU32>().unwrap_err()
            })
        );
        assert_eq!(
            QuestionsFile::parse("## 4294967296. A\n"),
            Err(ParseError::InvalidNumber {
                line: line(1),
                digits: String::from("4294967296"),
                source: "4294967296".parse::<NonZeroU32>().unwrap_err()
            })
        );
        QuestionsFile::parse("## 4294967295. A\n").unwrap();
    }

    #[test]
    fn two_questions_with_one_number_are_refused_with_the_second_line() {
        assert_eq!(
            QuestionsFile::parse("## 1. A\n\n## 2. B\n\n## 1. C\n"),
            Err(ParseError::DuplicateNumber {
                line: line(5),
                number: n(1)
            })
        );
    }

    #[test]
    fn a_second_recommended_or_verdict_part_is_refused_with_its_line() {
        assert_eq!(
            QuestionsFile::parse("## 1. A\n**Recommended:** x\n**Verdict:**\n**Recommended:** y\n"),
            Err(ParseError::DuplicateLabel {
                line: line(4),
                number: n(1),
                label: "Recommended"
            })
        );
        assert_eq!(
            QuestionsFile::parse("## 1. A\n**Verdict:** ok\n\n## 2. B\n**Verdict:**\n\n**Verdict:** ok\n"),
            Err(ParseError::DuplicateLabel {
                line: line(7),
                number: n(2),
                label: "Verdict"
            })
        );
    }

    #[test]
    fn a_file_over_the_limit_is_refused() {
        let text = format!("## 1. A\n{}", "x".repeat(MAX_QUESTIONS_BYTES));

        assert_eq!(
            QuestionsFile::parse(&text),
            Err(ParseError::TooLarge {
                len: text.len(),
                max: MAX_QUESTIONS_BYTES
            })
        );
        let at_the_limit = format!("## 1. A\n{}", "x".repeat(MAX_QUESTIONS_BYTES - "## 1. A\n".len()));
        QuestionsFile::parse(&at_the_limit).unwrap();
    }

    // ---- answering ----------------------------------------------------------------------------------------------

    #[test]
    fn a_verdict_is_set_replaced_and_emptied() {
        let mut file = QuestionsFile::parse(AGENT_FILE).unwrap();

        file.set_verdict(n(1), Some(Verdict::Accept)).unwrap();
        file.set_verdict(n(2), Some(own("bile, please"))).unwrap();
        assert_eq!(file.entry(n(1)).unwrap().verdict(), Some(&Verdict::Accept));
        assert_eq!(file.entry(n(2)).unwrap().verdict(), Some(&own("bile, please")));
        file.set_verdict(n(1), Some(own("b) on disk"))).unwrap();
        file.set_verdict(n(2), None).unwrap();

        assert_eq!(file.entry(n(1)).unwrap().verdict(), Some(&own("b) on disk")));
        assert_eq!(file.entry(n(2)).unwrap().verdict(), None);
        assert_eq!(QuestionsFile::parse(&file.render()).unwrap(), file);
    }

    #[test]
    fn a_verdict_is_added_to_a_question_that_has_no_verdict_part() {
        let mut file = QuestionsFile::parse("## 1. Q\n\n**Recommended:** x\n").unwrap();

        file.set_verdict(n(1), Some(Verdict::Accept)).unwrap();

        assert_eq!(file.render(), "## 1. Q\n\n**Recommended:** x\n\n**Verdict:** ok\n");
    }

    #[test]
    fn emptying_a_missing_verdict_changes_nothing() {
        let mut file = QuestionsFile::parse("## 1. Q\n").unwrap();
        let before = file.clone();

        file.set_verdict(n(1), None).unwrap();

        assert_eq!(file, before);
    }

    #[test]
    fn setting_the_verdict_of_an_unknown_question_is_refused() {
        let mut file = QuestionsFile::parse(AGENT_FILE).unwrap();
        let before = file.clone();

        assert_eq!(
            file.set_verdict(n(9), Some(Verdict::Accept)),
            Err(UnknownQuestion { number: n(9) })
        );
        assert_eq!(file, before);
    }

    #[test]
    fn setting_a_verdict_leaves_every_other_part_as_it_was() {
        let mut file = QuestionsFile::parse(
            "## 1. Q\nbody\n\n**Context:** c\n\n**Recommended:** r\n\n**Verdict:**\n\n**Note:** n\n",
        )
        .unwrap();

        file.set_verdict(n(1), Some(own("mine"))).unwrap();

        assert_eq!(
            file.render(),
            "## 1. Q\nbody\n\n**Context:** c\n\n**Recommended:** r\n\n**Verdict:** mine\n\n**Note:** n\n"
        );
    }

    #[test]
    fn the_answers_are_the_recommendation_or_the_own_text_in_file_order() {
        let mut file = QuestionsFile::parse(AGENT_FILE).unwrap();
        file.set_verdict(n(2), Some(Verdict::Accept)).unwrap();
        file.set_verdict(n(1), Some(own("b) on disk"))).unwrap();

        let answers = file.answers().unwrap();

        assert_eq!(
            answers,
            [
                Answer {
                    number: n(1),
                    title: String::from("Which store?"),
                    choice: Choice::Own(String::from("b) on disk"))
                },
                Answer {
                    number: n(2),
                    title: String::from("Żółć or bile?"),
                    choice: Choice::Recommendation(String::from("bile — the \"żółć\" key is\ntaken"))
                },
            ]
        );
    }

    #[test]
    fn every_unanswered_question_and_every_ok_without_a_recommendation_is_reported() {
        let file = QuestionsFile::parse("## 1. A\n**Verdict:**\n## 2. B\n**Recommended:** r\n## 3. C\n**Verdict:** ok\n## 4. D\n**Recommended:**\n**Verdict:** ok\n## 5. E\n**Verdict:** fine\n").unwrap();

        assert_eq!(
            file.answers(),
            Err(AnswersError {
                problems: vec![
                    (n(1), AnswerProblem::MissingVerdict),
                    (n(2), AnswerProblem::MissingVerdict),
                    (n(3), AnswerProblem::AcceptWithoutRecommendation),
                    (n(4), AnswerProblem::AcceptWithoutRecommendation),
                ]
            })
        );
    }

    #[test]
    fn the_questions_without_a_recommendation_are_listed() {
        let file = QuestionsFile::parse("## 1. A\n**Recommended:** r\n## 2. B\n## 3. C\n**Recommended:**\n").unwrap();

        assert_eq!(file.missing_recommendations(), [n(2), n(3)]);
        assert_eq!(QuestionsFile::parse(AGENT_FILE).unwrap().missing_recommendations(), []);
    }

    #[test]
    fn the_expanded_file_has_the_recommendations_in_place_of_ok_and_no_instructions_or_marker() {
        let mut file = QuestionsFile::parse(AGENT_FILE).unwrap();
        file.set_instructions("Answer every question.");
        file.set_verdict(n(1), Some(Verdict::Accept)).unwrap();
        file.set_verdict(n(2), Some(own("something else"))).unwrap();
        file.mark_answered(true);

        let expanded = file
            .expand(|recommendation| format!("accepted: {recommendation}"))
            .unwrap();

        assert_eq!(
            expanded,
            "\
# Questions — task-20261111-153601-k3x9

## 1. Which store?
a) in memory
b) on disk

**Recommended:** a) — simpler (inv-opus)

**Verdict:** accepted: a) — simpler (inv-opus)

## 2. Żółć or bile?
Free text about the second question.

**Recommended:** bile — the \"żółć\" key is
taken

**Verdict:** something else
"
        );
    }

    #[test]
    fn an_unanswered_file_cannot_be_expanded() {
        let file = QuestionsFile::parse(AGENT_FILE).unwrap();

        let err = file.expand(str::to_owned).unwrap_err();

        assert_eq!(
            err.problems,
            [
                (n(1), AnswerProblem::MissingVerdict),
                (n(2), AnswerProblem::MissingVerdict)
            ]
        );
    }
}
