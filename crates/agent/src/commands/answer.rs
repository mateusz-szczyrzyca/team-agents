//! `agent answer`: answers the questions of a task, in an editor or from a file.
//!
//! The daemon keeps the questions file of the open round; this command finds out which it is (`show`), takes your
//! answers from the file you give (`--file`) or from that file after your editor is done, and sends the verdicts it
//! finds. The daemon checks them against the questions, puts them into the file and queues the task. A file saved with
//! the `**ANSWERED**` marker is taken by the daemon by itself, so the answers may be taken already when the editor ends.

use std::io::{self, Write};
use std::num::NonZeroU32;
use std::path::{Path, PathBuf};
use std::process::ExitStatus;

use agent_core::config::CommandLine;
use agent_core::ids::TaskId;
use agent_core::protocol::{
    AnswerInput, AnswerList, AnswerListError, AnswerRequest, Client, Command, FailureKind, Reply, ShowRequest, Target,
};
use agent_core::questions::{MAX_QUESTIONS_BYTES, ParseError, QuestionsFile};
use agent_core::state::Member;
use agent_core::status::TaskStatus;
use agent_runtime::text_file::{TextFileError, read_text_file};

use super::status::kind_of;
use crate::client::{ClientError, Connection};

/// Where the answers come from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Source {
    /// A questions file the human filled in, given with `--file`.
    File(PathBuf),
    /// The questions file of the task, which this editor is started on.
    Editor(CommandLine),
}

/// Why `agent answer` failed.
#[derive(Debug, thiserror::Error)]
pub(crate) enum AnswerError {
    /// The daemon could not be asked, or refused.
    #[error(transparent)]
    Client(#[from] ClientError),
    /// The daemon answered with something else than what was asked.
    #[error("the daemon answered with {kind}, not with {wanted}")]
    UnexpectedReply {
        /// What it answered.
        kind: &'static str,
        /// What it was asked for.
        wanted: &'static str,
    },
    /// The task asks nothing of the human now.
    #[error("task {task} is {status}, not waiting for answers")]
    NotWaiting {
        /// The task.
        task: TaskId,
        /// Its status.
        status: TaskStatus,
    },
    /// The file with the answers is not there.
    #[error("the file {path:?} does not exist")]
    NoFile {
        /// The file.
        path: PathBuf,
    },
    /// The file with the answers cannot be read.
    #[error("cannot read the file {path:?}")]
    ReadFile {
        /// The file.
        path: PathBuf,
        /// What is wrong with it.
        source: TextFileError,
    },
    /// The file is not in the format of a questions file.
    #[error("the file {path:?} is not in the format of a questions file")]
    Format {
        /// The file.
        path: PathBuf,
        /// What is wrong with it, with its line.
        source: ParseError,
    },
    /// The file has no answer to any question.
    #[error(
        "the file {path:?} has no answers yet: write yours after **Verdict:** (`ok` accepts the recommendation), then run `agent answer {task}` again"
    )]
    NoAnswers {
        /// The file.
        path: PathBuf,
        /// The task.
        task: TaskId,
    },
    /// The file has more answers than a request takes.
    #[error("the file {path:?} has answers that cannot be sent")]
    Answers {
        /// The file.
        path: PathBuf,
        /// What is wrong with them.
        source: AnswerListError,
    },
    /// The editor cannot be started.
    #[error("cannot start the editor {program:?}")]
    EditorStart {
        /// The editor's program.
        program: PathBuf,
        /// What the operating system said.
        source: io::Error,
    },
    /// The editor ended with a failure.
    #[error("the editor {program:?} failed ({status}): the answers are not sent")]
    EditorFailed {
        /// The editor's program.
        program: PathBuf,
        /// How it ended.
        status: ExitStatus,
    },
    /// The daemon found the answers not complete.
    #[error("{message}\nComplete them in the file and run: agent answer {task}")]
    Incomplete {
        /// What the daemon said is missing.
        message: String,
        /// The task.
        task: TaskId,
    },
    /// The output cannot be written.
    #[error("cannot write the result")]
    Output(#[source] io::Error),
}

/// The round of questions a task waits for answers to.
struct OpenRound {
    /// The round.
    round: NonZeroU32,
    /// The questions file of the round.
    file: PathBuf,
}

/// Where the answers that are sent come from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Origin {
    /// A file the human named: nothing else can have taken the answers.
    File,
    /// The questions file of the round, edited now: the daemon may have taken it when the human saved it.
    Editor,
}

/// The answers of one round on their way to the daemon.
struct Delivery<'a> {
    /// The task.
    task: TaskId,
    /// The round the answers are for.
    open: &'a OpenRound,
    /// Where they come from.
    origin: Origin,
}

/// Answers the questions of `task` from `source` and says what happened.
///
/// # Errors
/// [`AnswerError`].
pub(crate) async fn answer(
    connection: &mut Connection,
    task: TaskId,
    source: Source,
    out: &mut impl Write,
) -> Result<(), AnswerError> {
    let open = open_round(connection, &task)
        .await?
        .map_err(|status| AnswerError::NotWaiting {
            task: task.clone(),
            status,
        })?;
    let (path, origin) = match source {
        Source::File(path) => (path, Origin::File),
        Source::Editor(editor) => {
            run_editor(&editor, &open.file).await?;
            // The daemon takes a file saved with the marker by itself: the answers may be taken already.
            let still_open = open_round(connection, &task)
                .await?
                .is_ok_and(|now| now.round == open.round);
            if !still_open {
                return taken(&task, out);
            }
            (open.file.clone(), Origin::Editor)
        }
    };
    let list = answers_in(&path, &task).await?;
    let delivery = Delivery {
        task,
        open: &open,
        origin,
    };
    send(connection, &delivery, list, out).await
}

/// Sends the answers `list` of `delivery`. When the daemon refuses them because the question is no longer open and the
/// file was edited, the daemon took them from the saved file first, which is no error.
async fn send(
    connection: &mut Connection,
    delivery: &Delivery<'_>,
    list: AnswerList,
    out: &mut impl Write,
) -> Result<(), AnswerError> {
    let Delivery { task, open, origin } = delivery;
    let task = task.clone();
    let request = Command::Answer(AnswerRequest {
        task: task.clone(),
        round: open.round,
        answers: list,
    });
    match connection.request(Client::Cli, request).await {
        Ok(Reply::Applied { .. }) => say(
            out,
            &format!(
                "Answered round {} of {task}: it goes on with your answers.\n",
                open.round
            ),
        ),
        Ok(other) => Err(AnswerError::UnexpectedReply {
            kind: kind_of(&other),
            wanted: "the task that took the answers",
        }),
        Err(ClientError::Refused(failure)) if failure.kind == FailureKind::Invalid => Err(AnswerError::Incomplete {
            message: failure.message,
            task,
        }),
        // The save that took the answers can land between the look after the editor and the answer.
        Err(ClientError::Refused(failure)) if *origin == Origin::Editor && failure.kind == FailureKind::Conflict => {
            let still_open = open_round(connection, &task)
                .await?
                .is_ok_and(|now| now.round == open.round);
            if still_open {
                return Err(ClientError::Refused(failure).into());
            }
            taken(&task, out)
        }
        Err(error) => Err(error.into()),
    }
}

/// Says that the answers were taken from the file the human saved.
fn taken(task: &TaskId, out: &mut impl Write) -> Result<(), AnswerError> {
    say(
        out,
        &format!("The answers were sent when you saved the file: {task} goes on.\n"),
    )
}

/// Writes `text` to `out`.
fn say(out: &mut impl Write, text: &str) -> Result<(), AnswerError> {
    out.write_all(text.as_bytes()).map_err(AnswerError::Output)?;
    out.flush().map_err(AnswerError::Output)
}

/// The round `task` waits for answers to, or the status it has instead.
async fn open_round(connection: &mut Connection, task: &TaskId) -> Result<Result<OpenRound, TaskStatus>, AnswerError> {
    let reply = connection
        .request(
            Client::Cli,
            Command::Show(ShowRequest {
                target: Target::Task(task.clone()),
            }),
        )
        .await?;
    let Reply::Task { member, .. } = reply else {
        return Err(AnswerError::UnexpectedReply {
            kind: kind_of(&reply),
            wanted: "the task",
        });
    };
    Ok(asked(&member))
}

/// The open round of `member`, or its status when it asks nothing.
fn asked(member: &Member) -> Result<OpenRound, TaskStatus> {
    match member.open_question() {
        Some(question) if member.status() == TaskStatus::Questions => Ok(OpenRound {
            round: question.round(),
            file: question.file().as_path().to_path_buf(),
        }),
        Some(_) | None => Err(member.status()),
    }
}

/// The verdicts of the questions file at `path`: one for each question it has an answer to.
async fn answers_in(path: &Path, task: &TaskId) -> Result<AnswerList, AnswerError> {
    let file = read_text_file(path, MAX_QUESTIONS_BYTES)
        .await
        .map_err(|source| AnswerError::ReadFile {
            path: path.to_path_buf(),
            source,
        })?
        .ok_or_else(|| AnswerError::NoFile {
            path: path.to_path_buf(),
        })?;
    let parsed = QuestionsFile::parse(&file.text).map_err(|source| AnswerError::Format {
        path: path.to_path_buf(),
        source,
    })?;
    let given: Vec<AnswerInput> = parsed
        .entries()
        .iter()
        .filter_map(|entry| {
            entry.verdict().map(|verdict| AnswerInput {
                question: entry.number(),
                verdict: verdict.clone(),
            })
        })
        .collect();
    AnswerList::try_from(given).map_err(|source| match source {
        AnswerListError::Empty => AnswerError::NoAnswers {
            path: path.to_path_buf(),
            task: task.clone(),
        },
        AnswerListError::TooMany { .. } | AnswerListError::Duplicate { .. } => AnswerError::Answers {
            path: path.to_path_buf(),
            source,
        },
    })
}

/// Starts `editor` on `file` and waits for it to end, with the terminal of this command.
async fn run_editor(editor: &CommandLine, file: &Path) -> Result<(), AnswerError> {
    let program = editor.program().as_path().to_path_buf();
    let status = tokio::process::Command::new(&program)
        .args(editor.args().as_slice())
        .arg(file)
        .status()
        .await
        .map_err(|source| AnswerError::EditorStart {
            program: program.clone(),
            source,
        })?;
    if status.success() {
        Ok(())
    } else {
        Err(AnswerError::EditorFailed { program, status })
    }
}
