//! The check of a request to a worker against the requester's plan: whether the four pieces a request names — a step
//! of the plan, the files, the check and what to do — make a request that leaves the worker nothing to guess.
//!
//! The bus does not read a worktree and has no plan reader; the daemon's `agent` command reads the plan file and the
//! configuration (`[plan] check_refuse`) at the moment of the request and asks [`decide`] here. Every outcome is a
//! variant of [`Rejection`], so a request the bus refuses says in its own words what to add.
//!
//! ```text
//! step 2.1 of a plan, its files, its check and what to do     ACCEPTED
//! a step the plan does not have                               NoSuchStep
//! a step that is done, or given up                            StepClosed
//! the check `reviewer`, or `agent gate`                       CheckRefused
//! a check followed by words: `grep x f.md shows the line`     SentenceCheck
//! ```

use crate::ids::{InvalidItemId, ItemId};
use crate::plan::judge::refuse_command;
use crate::plan::{Check, Plan, PlanError, Step, StepState};
use crate::repo_path::{InvalidRepoPath, RepoPath};

/// The pieces of a request to a worker that tie it to a step of the requester's plan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Named {
    /// The ID of the step, as the requester wrote it: `2.3`.
    pub step: String,
    /// The paths the worker may read and, when it produces something, write; empty when none were given.
    pub files: Vec<String>,
    /// The `Check:` line of the request, as the requester wrote it; empty when none was given.
    pub check: String,
    /// What to do, in the requester's words; empty when none was given.
    pub doing: String,
}

/// What a request names of its plan: the pieces the worker needs, and where they can be refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    /// What the requester gave.
    pub named: Named,
    /// The kinds of check the configuration refuses as a step's check (`[plan] check_refuse`).
    pub refused: Vec<String>,
}

/// A request to a worker that leaves it what to guess, and what to add.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Rejection {
    /// The request names no step of the plan: `for` is empty.
    #[error("the request names no step of your plan: add for, the ID of a step (2.3)")]
    MissingStep,
    /// `for` is not a step ID.
    #[error("the request's for {step:?} is no step ID (a step is named 2.3)")]
    BadStep {
        /// What the requester wrote.
        step: String,
        /// Why it is not one.
        #[source]
        source: InvalidItemId,
    },
    /// `for` names a step the plan does not have.
    #[error("the request's for {step} names no step of your plan: name a step it has")]
    NoSuchStep {
        /// The step named.
        step: ItemId,
        /// The steps the plan has.
        steps: String,
    },
    /// `for` names a step that is done or given up.
    #[error("the request's for {step} is {state}: send it only for a step still to do or in hand")]
    StepClosed {
        /// The step named.
        step: ItemId,
        /// Its state, as the plan writes it: done or given up.
        state: &'static str,
    },
    /// The requirement carries no files.
    #[error("the request names no files: add files, the paths the worker may read and write, comma-separated")]
    MissingFiles,
    /// A file is not a path inside the repository.
    #[error("the request's files have {path:?}, which is no path of the repository")]
    BadFiles {
        /// The path as the requester wrote it.
        path: String,
        /// Why it is not one.
        #[source]
        source: InvalidRepoPath,
    },
    /// The requirement carries no check.
    #[error("the request names no check: add check, a command or `exists PATH` that shows the result is right")]
    MissingCheck,
    /// The check is one a step of a plan must not carry: `reviewer`, or one the configuration refuses.
    #[error(
        "the request's check {check:?} cannot show a worker's result: a check is a command or `exists PATH`, never reviewer and never the repository's gate"
    )]
    CheckRefused {
        /// The check as the requester wrote it.
        check: String,
    },
    /// The check is not a command and not `exists PATH`.
    #[error("the request's check {check:?} is neither a command nor `exists PATH`")]
    BadCheck {
        /// The check as the requester wrote it.
        check: String,
        /// Why a plan would not read it as a step's `Check:` line.
        #[source]
        source: PlanError,
    },
    /// The check is a command followed by words that say what it shows: run as a command, it fails in every worktree.
    #[error(
        "the request's check {check:?} goes on as a sentence at {word:?}: a check is one command and nothing after it; say what it shows in do, and quote a word that belongs to the command"
    )]
    SentenceCheck {
        /// The check as the requester wrote it.
        check: String,
        /// The first word that reads as prose rather than part of the command.
        word: String,
    },
    /// The requirement says nothing to do.
    #[error("the request says nothing to do: add do, what the worker is to do")]
    MissingDo,
}

/// A request that leaves the worker nothing to guess: the step it is made for, the paths and the check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Accepted {
    /// The step of the requester's plan, as the plan has it.
    pub step: Step,
    /// The paths the worker may read and write, inside the repository.
    pub files: Vec<RepoPath>,
    /// The check of the request, read as a step's `Check:` line.
    pub check: Check,
    /// What to do, in the requester's words, trimmed.
    pub doing: String,
}

/// Decides a request of an agent with a plan: the step it names and the pieces it gives, or what to add.
///
/// # Errors
/// The [`Rejection`] of the first thing missing or wrong, in the order a requester writes them: the step, the files,
/// the check, what to do.
pub fn decide(plan: &Plan, request: &Request) -> Result<Accepted, Rejection> {
    let named = &request.named;
    let step = name_step(plan, named.step.trim())?;
    let files = name_files(&named.files)?;
    let check = name_check(named.check.trim(), &request.refused)?;
    let doing = named.doing.trim();
    if doing.is_empty() {
        return Err(Rejection::MissingDo);
    }
    Ok(Accepted {
        step: step.clone(),
        files,
        check,
        doing: doing.to_owned(),
    })
}

/// The step of `plan` that `text` names, which must still be open to a request: to do or in hand.
fn name_step<'a>(plan: &'a Plan, text: &str) -> Result<&'a Step, Rejection> {
    if text.is_empty() {
        return Err(Rejection::MissingStep);
    }
    let id: ItemId = text.parse().map_err(|source| Rejection::BadStep {
        step: text.to_owned(),
        source,
    })?;
    let step = plan.step(&id).ok_or_else(|| Rejection::NoSuchStep {
        step: id,
        steps: listed(plan),
    })?;
    match step.state() {
        StepState::Todo | StepState::InHand => Ok(step),
        StepState::Done => Err(Rejection::StepClosed {
            step: id,
            state: "done",
        }),
        StepState::GivenUp => Err(Rejection::StepClosed {
            step: id,
            state: "given up",
        }),
    }
}

/// The paths a request gives, each one a path of the repository.
fn name_files(given: &[String]) -> Result<Vec<RepoPath>, Rejection> {
    if given.is_empty() {
        return Err(Rejection::MissingFiles);
    }
    given
        .iter()
        .map(|path| {
            let path = path.trim();
            path.parse().map_err(|source| Rejection::BadFiles {
                path: path.to_owned(),
                source,
            })
        })
        .collect()
}

/// The check a request gives, which a worker's result must be shown by.
fn name_check(text: &str, refused: &[String]) -> Result<Check, Rejection> {
    if text.is_empty() {
        return Err(Rejection::MissingCheck);
    }
    let check: Check = text.parse().map_err(|source| Rejection::BadCheck {
        check: text.to_owned(),
        source,
    })?;
    if let Check::Command(command) = &check
        && let Some(word) = sentence_word(command)
    {
        return Err(Rejection::SentenceCheck {
            check: text.to_owned(),
            word,
        });
    }
    let refused_kind = match &check {
        Check::Command(command) => refuse_command(command, refused),
        Check::Exists(_) | Check::Reviewer => false,
    };
    if !check.shows_a_result() || refused_kind {
        return Err(Rejection::CheckRefused { check: text.to_owned() });
    }
    Ok(check)
}

/// Words that end a command and start a sentence about what it shows. A word of a command that is one of them is quoted.
const PROSE_WORDS: [&str; 14] = [
    "the", "and", "shows", "gives", "returns", "prints", "exits", "should", "must", "contains", "succeeds", "passes",
    "fails", "outputs",
];

/// The first word of `command`, after the command's name, that reads as prose: one of [`PROSE_WORDS`], a word that ends
/// with a comma, or an ellipsis. Quoted or escaped words and options are part of the command and never prose; `None`
/// when every word can be the command's.
fn sentence_word(command: &str) -> Option<String> {
    shell_words(command)
        .into_iter()
        .skip(1)
        .filter(|word| !word.quoted && !word.text.starts_with('-'))
        .map(|word| word.text)
        .find(|word| {
            let bare = word.trim_end_matches([',', '.']);
            word.contains('…') || (word.len() > 1 && word.ends_with(',')) || PROSE_WORDS.contains(&bare)
        })
}

/// A word of a command line, as a shell splits it.
struct ShellWord {
    /// The word with its quotes and escapes as written.
    text: String,
    /// Whether any part of it is quoted or escaped.
    quoted: bool,
}

/// The words of `command`, split at whitespace outside quotes; a quote that is not closed runs to the end.
fn shell_words(command: &str) -> Vec<ShellWord> {
    let mut words = Vec::new();
    let mut text = String::new();
    let mut quoted = false;
    let mut quote: Option<char> = None;
    let mut chars = command.chars();
    while let Some(c) = chars.next() {
        match (quote, c) {
            (Some(open), _) if c == open => quote = None,
            (None, '\\') => {
                quoted = true;
                text.push(c);
                if let Some(next) = chars.next() {
                    text.push(next);
                }
                continue;
            }
            (None, '\'' | '"') => {
                quote = Some(c);
                quoted = true;
            }
            (None, _) if c.is_whitespace() => {
                if !text.is_empty() {
                    words.push(ShellWord {
                        text: std::mem::take(&mut text),
                        quoted,
                    });
                }
                quoted = false;
                continue;
            }
            _ => {}
        }
        text.push(c);
    }
    if !text.is_empty() {
        words.push(ShellWord { text, quoted });
    }
    words
}

/// The steps of `plan` as a request may name them.
fn listed(plan: &Plan) -> String {
    let ids: Vec<String> = plan.steps().map(|step| step.id().to_string()).collect();
    if ids.is_empty() {
        "none".to_owned()
    } else {
        ids.join(", ")
    }
}

#[cfg(test)]
mod tests;
