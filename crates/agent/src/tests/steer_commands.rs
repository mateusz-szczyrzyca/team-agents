//! The commands that steer — `agent answer`, `stop`, `pause`, `unpause`, `resume` and `say` — on the command line: how
//! they are parsed, what they ask of the daemon, what they print and how they fail.

use std::ffi::OsString;
use std::num::NonZeroU32;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::atomic::{AtomicUsize, Ordering};

use agent_core::config::AbsolutePath;
use agent_core::protocol::{
    Command as Request, ControlRequest, Failure, FailureKind, Frame, Reply, Response, SayRequest, Target,
};
use agent_core::questions::Verdict;
use agent_core::state::{Question, RunEnd, State};
use agent_core::status::{RunOutcome, TaskStatus};

use super::*;
use crate::fixtures::{at, run as run_id, solo_state, task, team_id};
use crate::testing::{Answer, FakeDaemon};

/// A questions file with one question, as an agent writes it.
const ASKED: &str = "\
# Questions — fake

## 1. Which store?
a) memory
b) disk

**Recommended:** a) — simpler (fake)

**Verdict:**
";

/// The fake editor, a committed script.
fn fake_editor() -> String {
    format!("{}/tests/fixtures/fake-editor.sh", env!("CARGO_MANIFEST_DIR"))
}

fn environment(dir: &tempfile::TempDir) -> Environment {
    Environment::new(Some(OsString::from(dir.path())))
}

/// Runs `agent` with `args` in `environment` and returns its status, standard output and standard error; for the
/// command lines that fail before a daemon is asked.
fn agent(environment: &Environment, args: &[&str]) -> (ExitCode, String, String) {
    let mut out = Vec::new();
    let mut err = Vec::new();
    let code = run_in(
        environment,
        std::iter::once("agent").chain(args.iter().copied()),
        &mut out,
        &mut err,
    )
    .unwrap();
    (code, String::from_utf8(out).unwrap(), String::from_utf8(err).unwrap())
}

/// Carries out the command `args` says, which must be one that steers, and returns what it printed.
async fn steer(environment: &Environment, args: &[&str]) -> (Result<(), CommandError>, String) {
    let Invocation::Steer(steer) = parse_invocation(std::iter::once("agent").chain(args.iter().copied())).unwrap()
    else {
        panic!("a command that steers was expected: {args:?}");
    };
    let mut out = Vec::new();
    let result = execute_steer(environment, steer, &mut out).await;
    (result, String::from_utf8(out).unwrap())
}

fn line(request: &agent_core::protocol::Request, reply: Reply) -> Answer {
    Answer::Lines(vec![
        Frame::Response(Response::ok(request.id, reply)).to_line().unwrap(),
    ])
}

fn refusal(request: &agent_core::protocol::Request, kind: FailureKind, message: &str) -> Answer {
    let failure = Failure {
        kind,
        message: message.to_owned(),
    };
    Answer::Lines(vec![
        Frame::Response(Response::error(request.id, failure)).to_line().unwrap(),
    ])
}

fn nz(number: u32) -> NonZeroU32 {
    NonZeroU32::new(number).unwrap()
}

/// A state in which task 1 asked questions number 1, whose file is `file`.
fn asking(file: &Path) -> State {
    let mut state = solo_state();
    let (_, mut member) = state.find_task_mut(&task(1)).unwrap();
    member.transition(TaskStatus::Running).unwrap();
    member
        .begin_run(run_id(1), at(1), Some("ses-1".parse().unwrap()))
        .unwrap();
    member
        .end_run(
            at(2),
            RunEnd::Reported {
                outcome: RunOutcome::Questions,
            },
        )
        .unwrap();
    let file = AbsolutePath::try_from(file.to_str().unwrap().to_owned()).unwrap();
    member.ask_question(Question::new(nz(1), file, at(3))).unwrap();
    member.transition(TaskStatus::Questions).unwrap();
    state
}

/// The answer to `show` of task 1 in `state`.
fn shown(state: &State) -> Reply {
    let found = state.find_task(&task(1)).unwrap();
    Reply::Task {
        team: found.team.id().clone(),
        role: found.role,
        member: Box::new(found.member.clone()),
    }
}

/// A home with a daemon that shows task 1 asking for the file `file` and takes the answers.
fn answering(dir: &tempfile::TempDir, file: &Path) -> FakeDaemon {
    let state = asking(file);
    FakeDaemon::serve(&dir.path().join("agent.sock"), move |request| match &request.command {
        Request::Show(_) => line(request, shown(&state)),
        Request::Answer(_) => line(request, Reply::Applied { tasks: vec![task(1)] }),
        other => panic!("{other:?}"),
    })
}

/// The answers the daemon was sent, as (question, verdict text).
fn sent_answers(daemon: &FakeDaemon) -> Vec<Vec<(u32, String)>> {
    daemon
        .requests()
        .into_iter()
        .filter_map(|request| match request.command {
            Request::Answer(answer) => Some(
                answer
                    .answers
                    .as_slice()
                    .iter()
                    .map(|input| {
                        let text = match &input.verdict {
                            Verdict::Accept => "ok".to_owned(),
                            Verdict::Own(own) => own.as_str().to_owned(),
                        };
                        (input.question.get(), text)
                    })
                    .collect(),
            ),
            _ => None,
        })
        .collect()
}

fn write(dir: &tempfile::TempDir, name: &str, text: &str) -> PathBuf {
    let path = dir.path().join(name);
    std::fs::write(&path, text).unwrap();
    path
}

// ---- stop, pause, unpause, resume ---------------------------------------------------------------------------------

#[tokio::test]
async fn each_control_asks_the_daemon_for_its_own_action_and_prints_the_tasks_it_reached() {
    for (word, verb) in [
        ("stop", "Stopped"),
        ("pause", "Paused"),
        ("unpause", "Unpaused"),
        ("resume", "Resumed"),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let daemon = FakeDaemon::serve(&dir.path().join("agent.sock"), |request| {
            line(
                request,
                Reply::Applied {
                    tasks: vec![task(1), task(2)],
                },
            )
        });

        let (result, out) = steer(&environment(&dir), &[word, task(1).as_str()]).await;

        result.unwrap();
        assert_eq!(out, format!("{verb} {}\n{verb} {}\n", task(1), task(2)), "{word}");
        let request = daemon.requests().remove(0);
        let expected = ControlRequest {
            target: Target::Task(task(1)),
        };
        let expected = match word {
            "stop" => Request::Stop(expected),
            "pause" => Request::Pause(expected),
            "unpause" => Request::Unpause(expected),
            _ => Request::Resume(expected),
        };
        assert_eq!(request.command, expected, "{word}");
        assert_eq!(request.client, agent_core::protocol::Client::Cli, "{word}");
    }
}

#[tokio::test]
async fn a_team_is_a_target_too() {
    let dir = tempfile::tempdir().unwrap();
    let daemon = FakeDaemon::serve(&dir.path().join("agent.sock"), |request| {
        line(request, Reply::Applied { tasks: vec![task(1)] })
    });

    let (result, _) = steer(&environment(&dir), &["pause", team_id(0).as_str()]).await;

    result.unwrap();
    assert_eq!(
        daemon.requests()[0].command,
        Request::Pause(ControlRequest {
            target: Target::Team(team_id(0))
        })
    );
}

#[tokio::test]
async fn a_refusal_of_the_daemon_fails_the_command_with_its_reason() {
    let dir = tempfile::tempdir().unwrap();
    let _daemon = FakeDaemon::serve(&dir.path().join("agent.sock"), |request| {
        refusal(
            request,
            FailureKind::Conflict,
            "task x is done: nothing is left to do for it",
        )
    });

    let (result, out) = steer(&environment(&dir), &["stop", task(1).as_str()]).await;

    let error = result.unwrap_err();
    assert!(
        error_chain(&error).contains("the daemon refused the request: task x is done: nothing is left to do for it"),
        "{}",
        error_chain(&error)
    );
    assert_eq!(out, "");
}

#[test]
fn a_control_without_a_target_or_with_a_word_that_is_no_id_is_a_usage_error() {
    let dir = tempfile::tempdir().unwrap();
    for (args, said) in [
        (&["stop"][..], "<TARGET>"),
        (&["pause", "nonsense"], "neither a team ID"),
        (&["resume", "task-1"], "\"task-1\" is neither a team ID"),
        (&["answer", "team-20261111-153000-t5m8"], "is not a task ID"),
        (&["say", "task-20261111-153001-k3x9"], "<WORDS>"),
    ] {
        let (code, out, err) = agent(&environment(&dir), args);

        assert_eq!(code, ExitCode::from(2), "{args:?}: {err}");
        assert_eq!(out, "");
        assert!(err.contains(said), "{args:?}: {err}");
    }
}

#[test]
fn the_help_lists_the_commands_that_steer() {
    let dir = tempfile::tempdir().unwrap();

    let (code, out, _) = agent(&environment(&dir), &["--help"]);

    assert_eq!(code, ExitCode::SUCCESS);
    for word in ["answer", "stop", "pause", "unpause", "resume", "say"] {
        assert!(
            out.lines().any(|line| line.trim_start().starts_with(word)),
            "{word}: {out}"
        );
    }
}

// ---- say --------------------------------------------------------------------------------------------------------------

#[tokio::test]
async fn say_joins_the_words_with_spaces_and_prints_who_heard_it() {
    let dir = tempfile::tempdir().unwrap();
    let daemon = FakeDaemon::serve(&dir.path().join("agent.sock"), |request| {
        line(request, Reply::Applied { tasks: vec![task(1)] })
    });

    let (result, out) = steer(&environment(&dir), &["say", task(1).as_str(), "Prefer", "the disk."]).await;

    result.unwrap();
    assert_eq!(out, format!("Said it to {}\n", task(1)));
    let Request::Say(SayRequest { target, text }) = daemon.requests().remove(0).command else {
        panic!("a say was expected");
    };
    assert_eq!((target, text.as_str()), (Target::Task(task(1)), "Prefer the disk."));
}

#[tokio::test]
async fn say_with_blank_words_is_refused_before_the_daemon_is_asked() {
    let dir = tempfile::tempdir().unwrap();

    // No daemon listens: the words are checked first.
    let (result, _) = steer(&environment(&dir), &["say", task(1).as_str(), "  "]).await;

    assert!(matches!(result, Err(CommandError::Words(_))), "{result:?}");
}

// ---- answer -----------------------------------------------------------------------------------------------------------

#[tokio::test]
async fn answer_with_a_file_sends_the_verdicts_the_file_has_for_the_round_that_is_open() {
    let dir = tempfile::tempdir().unwrap();
    let questions = write(&dir, "questions-1.md", ASKED);
    let daemon = answering(&dir, &questions);
    let answers = write(
        &dir,
        "answers.md",
        &ASKED.replace("**Verdict:**\n", "**Verdict:** ok\n"),
    );

    let (result, out) = steer(
        &environment(&dir),
        &["answer", task(1).as_str(), "--file", answers.to_str().unwrap()],
    )
    .await;

    result.unwrap();
    assert_eq!(sent_answers(&daemon), [[(1, "ok".to_owned())]]);
    assert!(
        out.contains("questions number 1") && out.contains(task(1).as_str()),
        "{out}"
    );
    let Request::Answer(sent) = daemon.requests().pop().unwrap().command else {
        panic!("the answer was sent last");
    };
    assert_eq!((sent.task, sent.number), (task(1), nz(1)));
}

#[tokio::test]
async fn answer_with_a_file_of_own_text_sends_that_text() {
    let dir = tempfile::tempdir().unwrap();
    let questions = write(&dir, "questions-1.md", ASKED);
    let daemon = answering(&dir, &questions);
    let answers = write(
        &dir,
        "answers.md",
        &ASKED.replace("**Verdict:**\n", "**Verdict:** Use the disk,\nand name it store.db\n"),
    );

    let (result, _) = steer(
        &environment(&dir),
        &["answer", task(1).as_str(), "-f", answers.to_str().unwrap()],
    )
    .await;

    result.unwrap();
    assert_eq!(
        sent_answers(&daemon),
        [[(1, "Use the disk,\nand name it store.db".to_owned())]]
    );
}

#[tokio::test]
async fn answer_with_a_file_without_a_verdict_says_so_and_sends_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let questions = write(&dir, "questions-1.md", ASKED);
    let daemon = answering(&dir, &questions);
    let answers = write(&dir, "answers.md", ASKED);

    let (result, _) = steer(
        &environment(&dir),
        &["answer", task(1).as_str(), "--file", answers.to_str().unwrap()],
    )
    .await;

    let error = error_chain(&result.unwrap_err());
    assert!(
        error.contains("no answers") && error.contains("**Verdict:**"),
        "{error}"
    );
    assert_eq!(sent_answers(&daemon), Vec::<Vec<(u32, String)>>::new());
}

#[tokio::test]
async fn answer_with_a_file_that_is_not_there_names_it() {
    let dir = tempfile::tempdir().unwrap();
    let questions = write(&dir, "questions-1.md", ASKED);
    let _daemon = answering(&dir, &questions);
    let missing = dir.path().join("nothing.md");

    let (result, _) = steer(
        &environment(&dir),
        &["answer", task(1).as_str(), "--file", missing.to_str().unwrap()],
    )
    .await;

    let error = error_chain(&result.unwrap_err());
    assert!(
        error.contains("nothing.md") && error.contains("does not exist"),
        "{error}"
    );
}

#[tokio::test]
async fn answer_with_a_file_that_is_not_a_questions_file_says_what_is_wrong_with_it() {
    let dir = tempfile::tempdir().unwrap();
    let questions = write(&dir, "questions-1.md", ASKED);
    let _daemon = answering(&dir, &questions);
    let answers = write(&dir, "answers.md", "just some words\n");

    let (result, _) = steer(
        &environment(&dir),
        &["answer", task(1).as_str(), "--file", answers.to_str().unwrap()],
    )
    .await;

    let error = error_chain(&result.unwrap_err());
    assert!(
        error.contains("answers.md") && error.contains("not in the format of a questions file"),
        "{error}"
    );
}

#[tokio::test]
async fn answer_for_a_task_that_waits_for_nothing_says_so_and_sends_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let state = solo_state();
    let daemon = FakeDaemon::serve(&dir.path().join("agent.sock"), move |request| match &request.command {
        Request::Show(_) => line(request, shown(&state)),
        other => panic!("{other:?}"),
    });
    let answers = write(
        &dir,
        "answers.md",
        &ASKED.replace("**Verdict:**\n", "**Verdict:** ok\n"),
    );

    let (result, _) = steer(
        &environment(&dir),
        &["answer", task(1).as_str(), "--file", answers.to_str().unwrap()],
    )
    .await;

    let error = error_chain(&result.unwrap_err());
    assert!(
        error.contains(task(1).as_str()) && error.contains("not waiting for answers"),
        "{error}"
    );
    assert_eq!(daemon.requests().len(), 1, "only the question about the task was asked");
}

#[tokio::test]
async fn answers_the_daemon_finds_incomplete_are_shown_with_the_way_to_go_on() {
    let dir = tempfile::tempdir().unwrap();
    let questions = write(&dir, "questions-1.md", ASKED);
    let state = asking(&questions);
    let _daemon = FakeDaemon::serve(&dir.path().join("agent.sock"), move |request| match &request.command {
        Request::Show(_) => line(request, shown(&state)),
        Request::Answer(_) => refusal(
            request,
            FailureKind::Invalid,
            "the answers are not complete:\n- question 2 (Which name?): no verdict",
        ),
        other => panic!("{other:?}"),
    });
    let answers = write(
        &dir,
        "answers.md",
        &ASKED.replace("**Verdict:**\n", "**Verdict:** ok\n"),
    );

    let (result, _) = steer(
        &environment(&dir),
        &["answer", task(1).as_str(), "--file", answers.to_str().unwrap()],
    )
    .await;

    let error = error_chain(&result.unwrap_err());
    assert!(error.contains("question 2 (Which name?): no verdict"), "{error}");
    assert!(error.contains(&format!("agent answer {}", task(1))), "{error}");
}

/// A home whose configuration names the fake editor with `mode`.
fn editing(mode: &str) -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let questions = write(&dir, "questions-1.md", ASKED);
    write(
        &dir,
        "agents.toml",
        &format!("[questions]\neditor = [\"{}\", \"{mode}\"]\n", fake_editor()),
    );
    (dir, questions)
}

#[tokio::test]
async fn answer_in_the_editor_starts_it_on_the_questions_file_and_sends_what_was_saved() {
    let (dir, questions) = editing("accept");
    let daemon = answering(&dir, &questions);

    let (result, out) = steer(&environment(&dir), &["answer", task(1).as_str()]).await;

    result.unwrap();
    assert_eq!(sent_answers(&daemon), [[(1, "ok".to_owned())]]);
    assert!(out.contains("questions number 1"), "{out}");
}

#[tokio::test]
async fn answer_in_an_editor_that_changed_nothing_says_how_to_go_on_and_sends_nothing() {
    let (dir, questions) = editing("nothing");
    let daemon = answering(&dir, &questions);

    let (result, _) = steer(&environment(&dir), &["answer", task(1).as_str()]).await;

    let error = error_chain(&result.unwrap_err());
    assert!(
        error.contains("no answers") && error.contains(&format!("agent answer {}", task(1))),
        "{error}"
    );
    assert_eq!(sent_answers(&daemon), Vec::<Vec<(u32, String)>>::new());
}

#[tokio::test]
async fn answer_in_an_editor_that_fails_sends_nothing() {
    let (dir, questions) = editing("fail");
    let daemon = answering(&dir, &questions);

    let (result, _) = steer(&environment(&dir), &["answer", task(1).as_str()]).await;

    let error = error_chain(&result.unwrap_err());
    assert!(error.contains("the editor") && error.contains("failed"), "{error}");
    assert_eq!(sent_answers(&daemon), Vec::<Vec<(u32, String)>>::new());
}

#[tokio::test]
async fn answers_the_daemon_took_from_the_saved_file_while_the_editor_was_open_are_not_sent_again() {
    let (dir, questions) = editing("accept");
    let state = asking(&questions);
    let answered = {
        let mut next = state.clone();
        let (_, mut member) = next.find_task_mut(&task(1)).unwrap();
        member
            .answer_question(nz(1), at(10), agent_core::state::AnsweredVia::SavedFile)
            .unwrap();
        member.transition(TaskStatus::Queued).unwrap();
        next
    };
    let shows = AtomicUsize::new(0);
    let daemon = FakeDaemon::serve(&dir.path().join("agent.sock"), move |request| match &request.command {
        // The first look finds the questions open; the look after the editor finds them answered.
        Request::Show(_) if shows.fetch_add(1, Ordering::SeqCst) == 0 => line(request, shown(&state)),
        Request::Show(_) => line(request, shown(&answered)),
        other => panic!("{other:?}"),
    });

    let (result, out) = steer(&environment(&dir), &["answer", task(1).as_str()]).await;

    result.unwrap();
    assert!(out.contains("when you saved the file"), "{out}");
    assert_eq!(sent_answers(&daemon), Vec::<Vec<(u32, String)>>::new());
}

#[tokio::test]
async fn answers_the_daemon_took_from_the_saved_file_just_before_ours_are_not_an_error() {
    let (dir, questions) = editing("accept");
    let state = asking(&questions);
    let answered = {
        let mut next = state.clone();
        let (_, mut member) = next.find_task_mut(&task(1)).unwrap();
        member
            .answer_question(nz(1), at(10), agent_core::state::AnsweredVia::SavedFile)
            .unwrap();
        member.transition(TaskStatus::Queued).unwrap();
        next
    };
    let shows = AtomicUsize::new(0);
    let _daemon = FakeDaemon::serve(&dir.path().join("agent.sock"), move |request| match &request.command {
        // The questions are open at the look before and after the editor, and answered when the answer is refused.
        Request::Show(_) if shows.fetch_add(1, Ordering::SeqCst) < 2 => line(request, shown(&state)),
        Request::Show(_) => line(request, shown(&answered)),
        Request::Answer(_) => refusal(request, FailureKind::Conflict, "task is not waiting for answers"),
        other => panic!("{other:?}"),
    });

    let (result, out) = steer(&environment(&dir), &["answer", task(1).as_str()]).await;

    result.unwrap();
    assert!(out.contains("when you saved the file"), "{out}");
}

#[tokio::test]
async fn without_an_editor_in_the_configuration_or_the_environment_the_command_says_what_to_set() {
    let dir = tempfile::tempdir().unwrap();
    write(&dir, "agents.toml", "");

    let (result, _) = steer(&environment(&dir), &["answer", task(1).as_str()]).await;

    let error = error_chain(&result.unwrap_err());
    assert!(
        error.contains("$EDITOR") && error.contains("[questions]") && error.contains("--file"),
        "{error}"
    );
}

#[tokio::test]
async fn the_editor_of_the_environment_is_used_when_the_configuration_names_none() {
    let dir = tempfile::tempdir().unwrap();
    write(&dir, "agents.toml", "");
    let questions = write(&dir, "questions-1.md", ASKED);
    let daemon = answering(&dir, &questions);
    let editor = format!("{} accept", fake_editor());

    let (result, _) = steer(&environment(&dir).with_editor(&editor), &["answer", task(1).as_str()]).await;

    result.unwrap();
    assert_eq!(sent_answers(&daemon), [[(1, "ok".to_owned())]]);
}

#[tokio::test]
async fn the_editor_of_the_configuration_wins_over_the_environment() {
    let (dir, questions) = editing("own");
    let daemon = answering(&dir, &questions);

    let (result, _) = steer(
        &environment(&dir).with_editor("/nonexistent/editor"),
        &["answer", task(1).as_str()],
    )
    .await;

    result.unwrap();
    assert_eq!(sent_answers(&daemon), [[(1, "Use the disk".to_owned())]]);
}
