//! The checks before a review, in the daemon: the reasons the team is given, the questions the human is asked and what
//! their answers mean.
//!
//! What the checks find is `agent_core::decide::checks`, what becomes of it is `agent_core::decide::round::Checking`; the
//! owner carries it out (`owner/rounds/checks.rs`). Nothing here reads a file or runs a process, but the texts, which come
//! from `prompts/harness/checks/`.

use std::num::{NonZeroU32, TryFromIntError};
use std::path::Path;

use agent_core::decide::checks::markers::Marker;
use agent_core::decide::checks::{Issue, Subject, TestChange};
use agent_core::decide::round::AskReason;
use agent_core::ids::{TaskId, TeamId};
use agent_core::prompt::{ChecksPrompt, Variables, VariablesError};
use agent_core::questions::{ComposeError, Entry, EntryError, QuestionsFile, Verdict};
use agent_core::state::{CommitId, PrNumber};
use agent_runtime::checks_prompts::{ChecksPrompts, RenderChecksTextError};

/// The word with which the human allows what a question names; any case.
pub(super) const ALLOW_WORD: &str = "allow";

/// The word with which the human refuses what a question names.
pub(super) const REFUSE_WORD: &str = "refuse";

/// How many times the lead is woken to put the question to the human before the human is told directly that it did not.
pub(super) const MAX_RELAYS: u32 = 2;

/// Why a text of the checks cannot be made.
#[derive(Debug, thiserror::Error)]
pub(super) enum ChecksTextError {
    /// A text cannot be rendered.
    #[error("cannot render a text of the checks")]
    Render(#[from] RenderChecksTextError),
    /// A variable name of the program is not valid.
    #[error("a variable of a text is not valid")]
    Variable(#[from] VariablesError),
    /// A question cannot be made of the texts.
    #[error("a question of the checks cannot be made")]
    Entry(#[from] EntryError),
    /// The questions file cannot be made of the questions.
    #[error("the questions file of the checks cannot be made")]
    Compose(#[from] ComposeError),
    /// There are more questions than a file can number.
    #[error("the checks have more questions than a file can number")]
    Count(#[from] TryFromIntError),
}

/// How many digits of a commit ID name it in a text.
const SHORT_ID_LEN: usize = 7;

/// A commit ID as it is written in a text: its first digits.
fn short(id: &CommitId) -> String {
    id.as_str().chars().take(SHORT_ID_LEN).collect()
}

/// The reason of `issue`, as the team and the human are told it.
///
/// # Errors
/// [`ChecksTextError`].
pub(super) fn reason(texts: &ChecksPrompts, issue: &Issue) -> Result<String, ChecksTextError> {
    let (prompt, values) = match issue {
        Issue::TestChanged { path, how } => match how {
            TestChange::Deleted => (
                ChecksPrompt::IssueTestDeleted,
                Variables::new().with("path", path.as_str())?,
            ),
            TestChange::Renamed { to } => (
                ChecksPrompt::IssueTestRenamed,
                Variables::new().with("path", path.as_str())?.with("to", to.as_str())?,
            ),
            TestChange::Edited { removed } => (
                ChecksPrompt::IssueTestEdited,
                Variables::new()
                    .with("path", path.as_str())?
                    .with("removed", removed.to_string())?,
            ),
            TestChange::EditedBinary => (
                ChecksPrompt::IssueTestBinary,
                Variables::new().with("path", path.as_str())?,
            ),
        },
        Issue::ProtectedChanged { path, change } => (
            ChecksPrompt::IssueProtected,
            Variables::new()
                .with("path", path.as_str())?
                .with("change", change.to_string())?,
        ),
        Issue::RedTestEdited { path, red, edit } => (
            ChecksPrompt::IssueRedEdited,
            Variables::new()
                .with("path", path.as_str())?
                .with("red", short(red))?
                .with("edit", short(edit))?,
        ),
        Issue::NoTests => (ChecksPrompt::IssueNoTests, Variables::new()),
        Issue::Marker {
            path,
            marker,
            line,
            count,
        } => (
            match marker {
                Marker::SkipsTest => ChecksPrompt::IssueSkipsTest,
                Marker::SuppressesLint => ChecksPrompt::IssueSuppressesLint,
                Marker::ChecksUnderTest => ChecksPrompt::IssueChecksUnderTest,
            },
            Variables::new()
                .with("path", path.as_str())?
                .with("line", line.to_string())?
                .with("count", count.to_string())?,
        ),
    };
    Ok(texts.render(prompt, &values)?.trim().to_owned())
}

/// The reasons of `issues`, one on each line, each after a dash.
fn bullets<'a>(texts: &ChecksPrompts, issues: impl IntoIterator<Item = &'a Issue>) -> Result<String, ChecksTextError> {
    let lines: Vec<String> = issues
        .into_iter()
        .map(|issue| reason(texts, issue).map(|text| format!("- {text}")))
        .collect::<Result<_, _>>()?;
    Ok(lines.join("\n"))
}

/// What a subject is called in a list: the file, or the lack of tests.
fn named(texts: &ChecksPrompts, subject: &Subject) -> Result<String, ChecksTextError> {
    Ok(match subject {
        Subject::Path(path) => format!("`{path}`"),
        Subject::NoTests => texts
            .render(ChecksPrompt::QuestionTitleNoTests, &Variables::new())?
            .trim()
            .to_owned(),
    })
}

/// What the lead is told when the checks send the round back as the `pass`-th of `passes`: every reason, and what the
/// human `refused`.
///
/// # Errors
/// [`ChecksTextError`].
pub(super) fn team_message(
    texts: &ChecksPrompts,
    pr: PrNumber,
    pass: NonZeroU32,
    passes: u32,
    issues: &[Issue],
    refused: &[Subject],
) -> Result<String, ChecksTextError> {
    let refused = if refused.is_empty() {
        String::new()
    } else {
        let subjects: Vec<String> = refused
            .iter()
            .map(|subject| named(texts, subject).map(|name| format!("- {name}")))
            .collect::<Result<_, _>>()?;
        let told = texts.render(
            ChecksPrompt::Refused,
            &Variables::new().with("subjects", subjects.join("\n"))?,
        )?;
        format!("\n{}\n", told.trim())
    };
    let values = Variables::new()
        .with("pr", pr.to_string())?
        .with("pass", pass.to_string())?
        .with("passes", passes.to_string())?
        .with("reasons", bullets(texts, issues)?)?
        .with("refused", refused)?;
    Ok(texts.render(ChecksPrompt::TeamPass, &values)?.trim_end().to_owned())
}

/// The questions the human is asked about the issues of a round, and what each is about.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Questions {
    /// The questions file: one question for each subject, in the order of the subjects.
    pub(super) file: QuestionsFile,
    /// What each question is about, in the order of the questions.
    pub(super) subjects: Vec<Subject>,
}

/// The questions for the `issues` of the round of the team `team`, whose lead is `task`: one for each subject. `reason` says
/// why the human is asked, and `passes` is what the team used.
///
/// # Errors
/// [`ChecksTextError`].
pub(super) fn questions(
    texts: &ChecksPrompts,
    team: &TeamId,
    task: &TaskId,
    issues: &[Issue],
    reason: AskReason,
    passes: u32,
) -> Result<Questions, ChecksTextError> {
    let mut grouped: Vec<(Subject, Vec<&Issue>)> = Vec::new();
    for issue in issues {
        let subject = issue.subject();
        match grouped.iter_mut().find(|(known, _)| *known == subject) {
            Some((_, group)) => group.push(issue),
            None => grouped.push((subject, vec![issue])),
        }
    }
    let why = match reason {
        AskReason::OnlyTheHumanMayAllow => texts.render(ChecksPrompt::WhyOnlyHuman, &Variables::new())?,
        AskReason::PassesUsed => texts.render(
            ChecksPrompt::WhyPassesUsed,
            &Variables::new().with("passes", passes.to_string())?,
        )?,
    };
    let recommendation = texts.render(
        ChecksPrompt::QuestionRecommend,
        &Variables::new().with("refuse", REFUSE_WORD)?,
    )?;
    let mut entries = Vec::new();
    for (position, (subject, group)) in grouped.iter().enumerate() {
        let number = NonZeroU32::MIN.saturating_add(u32::try_from(position)?);
        let title = match subject {
            Subject::Path(path) => texts.render(
                ChecksPrompt::QuestionTitlePath,
                &Variables::new().with("path", path.as_str())?,
            )?,
            Subject::NoTests => texts.render(ChecksPrompt::QuestionTitleNoTests, &Variables::new())?,
        };
        let body = texts.render(
            ChecksPrompt::QuestionBody,
            &Variables::new()
                .with("why", why.trim())?
                .with("issues", bullets(texts, group.iter().copied())?)?
                .with("allow", ALLOW_WORD)?
                .with("refuse", REFUSE_WORD)?,
        )?;
        entries.push(
            Entry::new(number, title.trim())?
                .with_body(body.trim())?
                .recommend(recommendation.trim())?,
        );
    }
    let title = texts.render(
        ChecksPrompt::QuestionsTitle,
        &Variables::new()
            .with("team", team.as_str())?
            .with("task", task.as_str())?,
    )?;
    Ok(Questions {
        file: QuestionsFile::new(title.trim(), entries)?,
        subjects: grouped.into_iter().map(|(subject, _)| subject).collect(),
    })
}

/// What the human decided about the subjects of a questions file.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(super) struct Decisions {
    /// The subjects the human allowed.
    pub(super) allowed: Vec<Subject>,
    /// The subjects the human refused, or did not answer: only an explicit word allows.
    pub(super) refused: Vec<Subject>,
}

/// Whether the answer `verdict` allows: its first word is [`ALLOW_WORD`], in any case.
fn allows(verdict: &Verdict) -> bool {
    match verdict {
        Verdict::Accept => false,
        Verdict::Own(answer) => answer.as_str().split_whitespace().next().is_some_and(|word| {
            word.trim_matches(|c: char| !c.is_alphanumeric())
                .eq_ignore_ascii_case(ALLOW_WORD)
        }),
    }
}

/// What the human wrote in `file` about `subjects`, which the questions were made for, in their order. Only the word
/// [`ALLOW_WORD`] allows: `ok` accepts the recommendation, which is to refuse, and a question that has no verdict, or is
/// not in the file, is refused. A question of the file that names no subject is not looked at.
#[must_use]
pub(super) fn decisions(subjects: &[Subject], file: &QuestionsFile) -> Decisions {
    let mut decided = Decisions::default();
    for (position, subject) in subjects.iter().enumerate() {
        let verdict = u32::try_from(position)
            .ok()
            .and_then(|index| file.entry(NonZeroU32::MIN.saturating_add(index)))
            .and_then(Entry::verdict);
        if verdict.is_some_and(allows) {
            decided.allowed.push(subject.clone());
        } else {
            decided.refused.push(subject.clone());
        }
    }
    decided
}

/// What the lead is told to put the questions of `round` in `file` to the human.
///
/// # Errors
/// [`ChecksTextError`].
pub(super) fn relay(
    texts: &ChecksPrompts,
    task: &TaskId,
    round: NonZeroU32,
    file: &Path,
) -> Result<String, ChecksTextError> {
    let values = Variables::new()
        .with("questions_file", file.display().to_string())?
        .with("round", round.to_string())?
        .with("task", task.as_str())?;
    Ok(texts.render(ChecksPrompt::AskRelay, &values)?.trim_end().to_owned())
}

/// What the lead is told when the human answered the questions of the checks.
///
/// # Errors
/// [`ChecksTextError`].
pub(super) fn answered(texts: &ChecksPrompts) -> Result<String, ChecksTextError> {
    Ok(texts
        .render(ChecksPrompt::Answered, &Variables::new())?
        .trim_end()
        .to_owned())
}

#[cfg(test)]
mod tests;
