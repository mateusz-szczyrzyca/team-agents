//! `agent _delegation-check`: the check the bus asks of a request to a worker, against the requester's plan file.
//!
//! The bus reads no worktree and has no plan reader; this hidden command reads the plan at the moment of the request and
//! the `[plan] check_refuse` of the home's configuration, and tells the bus its verdict on standard output:
//!
//! ```text
//! ACCEPTED                                     the request may go to a worker
//! The step of the requester's plan:            the step, quoted as the plan has it
//! Step 1.2 Write the walkthrough               its ID, title and every line
//! These words are part of the step.
//! Files: docs/guide.md
//! Check: exists docs/guide.md
//! Request files: docs/guide.md, README.md      then the request's own pieces
//! Request check: exists docs/guide.md
//! Do: Write the walkthrough.
//! REFUSED <KEYWORD>                 the request is refused; the line below says what to add
//! NO-PLAN                           there is no plan file: the old form (a look-up) is allowed
//! ```
//!
//! The keyword of a refusal is one of [`MISSING_STEP`], [`MISSING_FILES`], [`MISSING_CHECK`], [`MISSING_DO`],
//! [`NO_SUCH_STEP`], [`STEP_CLOSED`], [`PATH_OUTSIDE`], [`CHECK_REFUSED`], [`PLAN_UNREADABLE`]. The command's status is
//! success whenever it answered; a write failure or a configuration that does not load is its error.

use std::io::{self, Write};
use std::path::{Path, PathBuf};

use agent_core::delegation::{self, Named, Rejection, Request};
use agent_core::plan::{self, Check, Plan, Step};
use agent_core::repo_path::RepoPath;
use agent_runtime::process::error_chain;
use clap::Args;

/// The verdict of an agent without a plan file: it may send a request in the old form (a look-up).
pub(crate) const NO_PLAN: &str = "NO-PLAN";

/// The verdict of a request that may go to a worker.
const ACCEPTED: &str = "ACCEPTED";

/// The word that starts the line of a refusal.
const REFUSED: &str = "REFUSED";

/// The keyword of a request that names no step; the bus maps it to its own cause.
pub(crate) const MISSING_STEP: &str = "MISSING-STEP";
/// The request names no files.
pub(crate) const MISSING_FILES: &str = "MISSING-FILES";
/// The request names no check.
pub(crate) const MISSING_CHECK: &str = "MISSING-CHECK";
/// The request says nothing to do.
pub(crate) const MISSING_DO: &str = "MISSING-DO";
/// The step named is not in the plan.
pub(crate) const NO_SUCH_STEP: &str = "NO-SUCH-STEP";
/// The step named is done or given up.
pub(crate) const STEP_CLOSED: &str = "STEP-CLOSED";
/// A file is not a path inside the repository.
pub(crate) const PATH_OUTSIDE: &str = "PATH-OUTSIDE";
/// The check is one a worker's result cannot be shown by.
pub(crate) const CHECK_REFUSED: &str = "CHECK-REFUSED";
/// The plan file is there but does not read.
pub(crate) const PLAN_UNREADABLE: &str = "PLAN-UNREADABLE";

/// The arguments of `agent _delegation-check`.
#[derive(Debug, Args)]
pub(crate) struct DelegationCheckArgs {
    /// The requester's plan file, `.agent/plan.md` of its worktree.
    #[arg(long, value_name = "PATH")]
    pub(crate) plan: PathBuf,
    /// The step of the plan the request names (`--for 2.3`).
    #[arg(long = "for", value_name = "STEP", default_value = "", allow_hyphen_values = true)]
    pub(crate) step: String,
    /// The paths the worker may read and write, comma-separated.
    #[arg(long, value_name = "PATHS", default_value = "", allow_hyphen_values = true)]
    pub(crate) files: String,
    /// The check that shows the result is right.
    #[arg(long, value_name = "CHECK", default_value = "", allow_hyphen_values = true)]
    pub(crate) check: String,
    /// What the worker is to do.
    #[arg(long = "do", value_name = "WHAT", default_value = "", allow_hyphen_values = true)]
    pub(crate) doing: String,
}

/// Why a plan file cannot be used.
#[derive(Debug, thiserror::Error)]
pub(crate) enum PlanProblem {
    /// The file cannot be read.
    #[error("cannot read the plan file {path:?}")]
    Read {
        /// The plan file.
        path: PathBuf,
        /// The failure of the file system.
        #[source]
        source: io::Error,
    },
    /// The file is not text.
    #[error("the plan file {path:?} is not UTF-8")]
    NotText {
        /// The plan file.
        path: PathBuf,
        /// What is wrong with its bytes.
        #[source]
        source: std::string::FromUtf8Error,
    },
    /// The text is not a plan.
    #[error("the plan file {path:?} does not read")]
    Parse {
        /// The plan file.
        path: PathBuf,
        /// The plan reader's reason, with its line.
        #[source]
        source: plan::PlanError,
    },
}

/// Reads the plan file at `path`: `None` for a file that is not there (an agent without a plan).
///
/// # Errors
/// [`PlanProblem`] when the file cannot be read, is not text, or is not a plan.
pub(crate) fn read_plan(path: &Path) -> Result<Option<Plan>, PlanProblem> {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(source) if source.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(source) => {
            return Err(PlanProblem::Read {
                path: path.to_path_buf(),
                source,
            });
        }
    };
    let text = String::from_utf8(bytes).map_err(|source| PlanProblem::NotText {
        path: path.to_path_buf(),
        source,
    })?;
    plan::parse_plan(&text).map(Some).map_err(|source| PlanProblem::Parse {
        path: path.to_path_buf(),
        source,
    })
}

/// The keyword of `rejection`, which the bus maps to its own cause.
pub(crate) fn keyword(rejection: &Rejection) -> &'static str {
    match rejection {
        Rejection::MissingStep => MISSING_STEP,
        Rejection::BadStep { .. } | Rejection::NoSuchStep { .. } => NO_SUCH_STEP,
        Rejection::StepClosed { .. } => STEP_CLOSED,
        Rejection::MissingFiles => MISSING_FILES,
        Rejection::BadFiles { .. } => PATH_OUTSIDE,
        Rejection::MissingCheck => MISSING_CHECK,
        Rejection::CheckRefused { .. } | Rejection::BadCheck { .. } | Rejection::SentenceCheck { .. } => CHECK_REFUSED,
        Rejection::MissingDo => MISSING_DO,
    }
}

/// The lines of the step `step` in full: its line, then its text as the plan has it.
fn step_lines(step: &Step) -> Vec<String> {
    let mut lines = vec![format!("Step {} {}", step.id(), step.title())];
    lines.extend(step.note().lines().map(str::to_owned));
    if !step.files().is_empty() {
        let paths: Vec<&str> = step.files().iter().map(RepoPath::as_str).collect();
        lines.push(format!("Files: {}", paths.join(", ")));
    }
    if let Some(check) = step.check() {
        lines.push(format!("Check: {check}"));
    }
    lines
}

/// The line before the step quoted as the plan has it.
const STEP_INTRO: &str = "The step of the requester's plan:";

/// The label of the request's own files, apart from the step's `Files:`.
const REQUEST_FILES: &str = "Request files:";

/// The label of the request's own check, apart from the step's `Check:`.
const REQUEST_CHECK: &str = "Request check:";

/// The lines of the answer to a complete request: the step in full, then the request's own files, check and what to
/// do, each under a label of its own so that nothing of the step is taken for something the requester gave.
fn answer_lines(step: &Step, files: &[RepoPath], check: &Check, doing: &str) -> Vec<String> {
    let mut lines = vec![ACCEPTED.to_owned(), STEP_INTRO.to_owned()];
    lines.extend(step_lines(step));
    let paths: Vec<&str> = files.iter().map(RepoPath::as_str).collect();
    lines.push(format!("{REQUEST_FILES} {}", paths.join(", ")));
    lines.push(format!("{REQUEST_CHECK} {check}"));
    lines.push(format!("Do: {doing}"));
    lines
}

/// Writes `lines`, each on a line of its own, and flushes them.
fn write_lines(out: &mut impl Write, lines: &[String]) -> io::Result<()> {
    for line in lines {
        writeln!(out, "{line}")?;
    }
    out.flush()
}

/// Writes a refusal with its keyword and its advice, which is one line.
fn refuse(out: &mut impl Write, keyword: &str, advice: &str) -> Result<(), AnswerError> {
    let advice = advice.replace('\n', " ");
    write_lines(out, &[format!("{REFUSED} {keyword}"), advice.trim().to_owned()]).map_err(AnswerError::Write)
}

/// Writes the verdict of `args` to `out`, with the checks `refused` names (`[plan] check_refuse`).
///
/// # Errors
/// [`AnswerError::Write`] when `out` refuses the answer.
pub(crate) fn answer(args: &DelegationCheckArgs, refused: &[String], out: &mut impl Write) -> Result<(), AnswerError> {
    let plan = match read_plan(&args.plan) {
        Ok(None) => return write_lines(out, &[NO_PLAN.to_owned()]).map_err(AnswerError::Write),
        Ok(Some(plan)) => plan,
        Err(problem) => return refuse(out, PLAN_UNREADABLE, &error_chain(&problem)),
    };
    let named = Named {
        step: args.step.clone(),
        files: files_of(&args.files),
        check: args.check.clone(),
        doing: args.doing.clone(),
    };
    let request = Request {
        named,
        refused: refused.to_vec(),
    };
    match delegation::decide(&plan, &request) {
        Ok(accepted) => {
            let lines = answer_lines(&accepted.step, &accepted.files, &accepted.check, &accepted.doing);
            write_lines(out, &lines).map_err(AnswerError::Write)
        }
        Err(rejection) => refuse(out, keyword(&rejection), &error_chain(&rejection)),
    }
}

/// The paths of a request's `--files`, comma-separated: none for a text with nothing in it, and otherwise every entry,
/// an empty one too, so that the plan's path rules refuse it instead of it being dropped.
fn files_of(text: &str) -> Vec<String> {
    if text.trim().is_empty() {
        return Vec::new();
    }
    text.split(',').map(|path| path.trim().to_owned()).collect()
}

/// Why `agent _delegation-check` could not answer.
#[derive(Debug, thiserror::Error)]
pub(crate) enum AnswerError {
    /// The answer cannot be written.
    #[error("cannot write the answer of the delegation check")]
    Write(#[source] io::Error),
}
