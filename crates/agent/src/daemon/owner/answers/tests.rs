//! The questions of a task through the daemon: the file a run's questions leave, the notification, and the three ways
//! the answers arrive — the protocol, a saved file and the dashboard — each resuming the task with them.

use agent_core::protocol::{Client, Command, ControlRequest, FailureKind, Reply, Target};
use agent_core::questions::QuestionsFile;
use agent_core::state::{AnsweredVia, QuestionState, State};
use agent_core::status::TaskStatus;

use super::super::fixture::{Daemon, answer, n};
use crate::client::ClientError;
use crate::daemon::questions::watch::Report;
use crate::fixtures::{solo_state, task};

/// A daemon whose task 1 asked its question and waits for the answers; its next run ends with `DONE`.
async fn waiting_for_answers() -> Daemon {
    let mut daemon = Daemon::new(&solo_state(), None);
    let file = daemon.questions_file(1, 1);
    daemon.scenario(1, &format!("questions {}", file.display()));
    daemon.start().await;
    daemon.status_is(1, TaskStatus::Questions).await;
    daemon.scenario(1, "done");
    daemon
}

/// The refusal an error of the client is.
fn refusal(result: Result<Reply, ClientError>) -> (FailureKind, String) {
    match result {
        Err(ClientError::Refused(failure)) => (failure.kind, failure.message),
        other => panic!("a refusal was expected: {other:?}"),
    }
}

fn answered_via(state: &State) -> AnsweredVia {
    let member = state.find_task(&task(1)).unwrap().member;
    match member.questions()[0].state() {
        QuestionState::Answered { via, .. } => *via,
        QuestionState::Open => panic!("the questions are still open"),
    }
}

#[tokio::test]
async fn a_run_ending_with_questions_leaves_a_file_with_the_instructions_and_tells_the_human() {
    let mut daemon = waiting_for_answers().await;

    let text = std::fs::read_to_string(daemon.questions_file(1, 1)).unwrap();
    let file = QuestionsFile::parse(&text).unwrap();
    assert!(
        file.instructions().contains(&format!("agent answer {}", task(1))),
        "{}",
        file.instructions()
    );
    assert!(!file.is_answered());
    let state = daemon.state();
    let member = state.find_task(&task(1)).unwrap().member;
    assert_eq!(member.questions().len(), 1);
    assert!(member.open_question().is_some());
    let note = daemon.notification_with("questions-1.md").await;
    assert!(note.contains(&format!("agent answer {}", task(1))), "{note}");
    daemon.stop().await.unwrap();
}

#[tokio::test]
async fn answers_from_the_protocol_resume_the_task_with_the_answers_in_its_next_prompt() {
    let mut daemon = waiting_for_answers().await;

    let reply = daemon.ask(answer(1, 1, &[(1, "ok")])).await.unwrap();

    assert_eq!(reply, Reply::Applied { tasks: vec![task(1)] });
    let state = daemon.status_is(1, TaskStatus::Done).await;
    assert_eq!(answered_via(&state), AnsweredVia::Command);
    let prompt = daemon.recorded(1, ".fake-args");
    assert!(
        prompt.contains("the human accepts the recommendation: a) — simpler (fake)"),
        "{prompt}"
    );
    assert!(prompt.contains("round 1"), "{prompt}");
    let on_disk = QuestionsFile::parse(&std::fs::read_to_string(daemon.questions_file(1, 1)).unwrap()).unwrap();
    assert!(on_disk.is_answered(), "the file keeps the answers that were given");
    daemon.stop().await.unwrap();
}

#[tokio::test]
async fn an_own_answer_reaches_the_task_as_it_was_written() {
    let mut daemon = waiting_for_answers().await;

    daemon
        .ask(answer(1, 1, &[(1, "Use the disk, and call the file store.db")]))
        .await
        .unwrap();

    daemon.status_is(1, TaskStatus::Done).await;
    let prompt = daemon.recorded(1, ".fake-args");
    assert!(prompt.contains("Use the disk, and call the file store.db"), "{prompt}");
    daemon.stop().await.unwrap();
}

#[tokio::test]
async fn answers_from_the_dashboard_are_recorded_as_such() {
    let mut daemon = waiting_for_answers().await;
    let client = Client::Dashboard {
        login: "you@example.com".parse().unwrap(),
    };

    daemon
        .connect()
        .await
        .request(client, answer(1, 1, &[(1, "ok")]))
        .await
        .unwrap();

    let state = daemon.status_is(1, TaskStatus::Done).await;
    assert_eq!(answered_via(&state), AnsweredVia::Dashboard);
    daemon.stop().await.unwrap();
}

#[tokio::test]
async fn the_event_of_an_answer_names_the_channel() {
    let mut daemon = waiting_for_answers().await;
    let mut events = daemon.subscribe().await;

    daemon.ask(answer(1, 1, &[(1, "ok")])).await.unwrap();

    let via = events
        .until("the answer", |event| match &event.event {
            agent_core::protocol::EventKind::QuestionAnswered { via, round, .. } if *round == n(1) => Some(*via),
            _ => None,
        })
        .await;
    assert_eq!(via, AnsweredVia::Command);
    daemon.stop().await.unwrap();
}

#[tokio::test]
async fn a_saved_file_marked_answered_resumes_the_task_with_its_answers() {
    let mut daemon = waiting_for_answers().await;
    let path = daemon.questions_file(1, 1);
    let mut file = QuestionsFile::parse(&std::fs::read_to_string(&path).unwrap()).unwrap();
    file.set_verdict(
        n(1),
        Some(
            agent_core::questions::Verdict::from_text("Use the disk")
                .unwrap()
                .unwrap(),
        ),
    )
    .unwrap();
    file.mark_answered();

    std::fs::write(&path, file.render()).unwrap();

    let state = daemon.status_is(1, TaskStatus::Done).await;
    assert_eq!(answered_via(&state), AnsweredVia::SavedFile);
    let prompt = daemon.recorded(1, ".fake-args");
    assert!(prompt.contains("Use the disk"), "{prompt}");
    daemon.stop().await.unwrap();
}

#[tokio::test]
async fn a_saved_file_marked_answered_with_a_question_unanswered_tells_the_human_and_changes_nothing() {
    let mut daemon = waiting_for_answers().await;
    let path = daemon.questions_file(1, 1);
    let mut file = QuestionsFile::parse(&std::fs::read_to_string(&path).unwrap()).unwrap();
    file.mark_answered();

    std::fs::write(&path, file.render()).unwrap();

    let note = daemon.notification_with("answers are not complete").await;
    assert!(note.contains("question 1 (Which store?): no verdict"), "{note}");
    assert!(note.contains("questions-1.md"), "{note}");
    let state = daemon.state();
    assert_eq!(
        state.find_task(&task(1)).unwrap().member.status(),
        TaskStatus::Questions
    );
    daemon.stop().await.unwrap();
}

#[tokio::test]
async fn answers_saved_while_the_daemon_was_down_are_found_when_it_starts() {
    let mut daemon = waiting_for_answers().await;
    daemon.stop().await.unwrap();
    let path = daemon.questions_file(1, 1);
    let mut file = QuestionsFile::parse(&std::fs::read_to_string(&path).unwrap()).unwrap();
    file.set_verdict(
        n(1),
        Some(agent_core::questions::Verdict::from_text("ok").unwrap().unwrap()),
    )
    .unwrap();
    file.mark_answered();
    std::fs::write(&path, file.render()).unwrap();

    daemon.start().await;

    let state = daemon.status_is(1, TaskStatus::Done).await;
    assert_eq!(answered_via(&state), AnsweredVia::SavedFile);
    daemon.stop().await.unwrap();
}

#[tokio::test]
async fn answers_for_a_question_the_file_does_not_have_are_refused_and_the_task_goes_on_waiting() {
    let mut daemon = waiting_for_answers().await;

    let (kind, message) = refusal(daemon.ask(answer(1, 1, &[(1, "ok"), (9, "x")])).await);

    assert_eq!(kind, FailureKind::Invalid);
    assert!(message.contains('9'), "{message}");
    let state = daemon.state();
    assert_eq!(
        state.find_task(&task(1)).unwrap().member.status(),
        TaskStatus::Questions
    );
    assert!(state.find_task(&task(1)).unwrap().member.open_question().is_some());
    daemon.stop().await.unwrap();
}

#[tokio::test]
async fn answers_for_a_round_that_is_not_open_are_refused() {
    let mut daemon = waiting_for_answers().await;

    let (kind, message) = refusal(daemon.ask(answer(1, 2, &[(1, "ok")])).await);

    assert_eq!(kind, FailureKind::Conflict);
    assert!(message.contains("round 1"), "{message}");
    daemon.stop().await.unwrap();
}

#[tokio::test]
async fn answers_for_a_task_that_does_not_wait_for_any_are_refused() {
    let mut daemon = waiting_for_answers().await;
    daemon.ask(answer(1, 1, &[(1, "ok")])).await.unwrap();
    daemon.status_is(1, TaskStatus::Done).await;

    let (kind, _) = refusal(daemon.ask(answer(1, 1, &[(1, "ok")])).await);

    assert_eq!(kind, FailureKind::Conflict);
    daemon.stop().await.unwrap();
}

#[tokio::test]
async fn answers_for_a_task_that_is_not_there_are_refused_as_not_found() {
    let mut daemon = waiting_for_answers().await;

    let (kind, _) = refusal(daemon.ask(answer(7, 1, &[(1, "ok")])).await);

    assert_eq!(kind, FailureKind::NotFound);
    daemon.stop().await.unwrap();
}

#[tokio::test]
async fn a_task_that_asks_again_gets_its_next_round_in_the_next_file() {
    let mut daemon = waiting_for_answers().await;
    let second = daemon.questions_file(1, 2);
    daemon.scenario(1, &format!("questions {}", second.display()));

    daemon.ask(answer(1, 1, &[(1, "ok")])).await.unwrap();

    daemon
        .state_where("the second round", |state| {
            state.find_task(&task(1)).unwrap().member.questions().len() == 2
        })
        .await;
    daemon.status_is(1, TaskStatus::Questions).await;
    assert!(second.exists());
    let (kind, _) = refusal(daemon.ask(answer(1, 1, &[(1, "ok")])).await);
    assert_eq!(
        kind,
        FailureKind::Conflict,
        "round 1 is answered; round 2 is the open one"
    );
    let reply = daemon.ask(answer(1, 2, &[(1, "ok")])).await.unwrap();
    assert_eq!(reply, Reply::Applied { tasks: vec![task(1)] });
    daemon.stop().await.unwrap();
}

#[tokio::test]
async fn answers_that_a_run_crashed_before_reading_are_given_again_when_the_task_is_resumed() {
    let mut daemon = waiting_for_answers().await;
    daemon.scenario(1, "crash");
    daemon.ask(answer(1, 1, &[(1, "Use the disk")])).await.unwrap();
    daemon.status_is(1, TaskStatus::Failed).await;
    daemon.scenario(1, "done");

    daemon
        .ask(Command::Resume(ControlRequest {
            target: Target::Task(task(1)),
        }))
        .await
        .unwrap();

    daemon.status_is(1, TaskStatus::Done).await;
    let prompt = daemon.recorded(1, ".fake-args");
    assert!(prompt.contains("Use the disk"), "{prompt}");
    daemon.stop().await.unwrap();
}

#[tokio::test]
async fn answers_that_a_run_read_are_not_given_again_to_the_next_run() {
    let mut daemon = waiting_for_answers().await;
    daemon.scenario(1, "nostatus");
    daemon.ask(answer(1, 1, &[(1, "Use the disk")])).await.unwrap();
    daemon.status_is(1, TaskStatus::Failed).await;
    daemon.scenario(1, "done");

    daemon
        .ask(Command::Resume(ControlRequest {
            target: Target::Task(task(1)),
        }))
        .await
        .unwrap();

    daemon.status_is(1, TaskStatus::Done).await;
    let prompt = daemon.recorded(1, ".fake-args");
    assert!(!prompt.contains("Use the disk"), "{prompt}");
    daemon.stop().await.unwrap();
}

// ---- the editor -------------------------------------------------------------------------------------------------

/// The files the stand-in opened in the window whose record is `opened`.
fn opened_in(opened: &std::path::Path) -> Vec<String> {
    std::fs::read_to_string(opened)
        .unwrap_or_default()
        .lines()
        .map(str::to_owned)
        .collect()
}

#[tokio::test]
async fn a_new_questions_file_is_opened_in_the_window_of_vscode_when_that_is_configured() {
    let mut daemon = Daemon::new(&solo_state(), None);
    let servers = daemon.surroundings.runtime_dir.join("servers");
    Daemon::vscode_server(&servers);
    let opened = daemon.vscode_window();
    daemon.configure(&format!(
        "[questions]\nopen = \"vscode\"\nvscode_server_dirs = [\"{}\"]",
        servers.display()
    ));
    let file = daemon.questions_file(1, 1);
    daemon.scenario(1, &format!("questions {}", file.display()));

    daemon.start().await;
    daemon.status_is(1, TaskStatus::Questions).await;
    // Stopping ends the task that opens files after it has done what was queued.
    daemon.stop().await.unwrap();

    assert_eq!(opened_in(&opened), [file.display().to_string()]);
}

#[tokio::test]
async fn the_servers_are_looked_for_in_the_home_of_the_user_when_the_configuration_names_none() {
    let mut daemon = Daemon::new(&solo_state(), None);
    let home = daemon.surroundings.user_home.clone().unwrap();
    Daemon::vscode_server(&home.join(".vscode-server"));
    let opened = daemon.vscode_window();
    daemon.configure("[questions]\nopen = \"vscode\"");
    let file = daemon.questions_file(1, 1);
    daemon.scenario(1, &format!("questions {}", file.display()));

    daemon.start().await;
    daemon.status_is(1, TaskStatus::Questions).await;
    daemon.stop().await.unwrap();

    assert_eq!(opened_in(&opened), [file.display().to_string()]);
}

#[tokio::test]
async fn a_new_questions_file_is_not_opened_unless_the_configuration_asks_for_it() {
    let mut daemon = Daemon::new(&solo_state(), None);
    let servers = daemon.surroundings.runtime_dir.join("servers");
    Daemon::vscode_server(&servers);
    let opened = daemon.vscode_window();
    daemon.configure(&format!(
        "[questions]\nvscode_server_dirs = [\"{}\"]",
        servers.display()
    ));
    let file = daemon.questions_file(1, 1);
    daemon.scenario(1, &format!("questions {}", file.display()));

    daemon.start().await;
    daemon.status_is(1, TaskStatus::Questions).await;
    daemon.stop().await.unwrap();

    assert_eq!(opened_in(&opened), Vec::<String>::new());
}

#[tokio::test]
async fn a_window_that_cannot_be_reached_does_not_keep_the_human_from_being_told() {
    let mut daemon = Daemon::new(&solo_state(), None);
    let servers = daemon.surroundings.runtime_dir.join("servers");
    Daemon::vscode_server(&servers);
    daemon.configure(&format!(
        "[questions]\nopen = \"vscode\"\nvscode_server_dirs = [\"{}\"]",
        servers.display()
    ));
    let file = daemon.questions_file(1, 1);
    daemon.scenario(1, &format!("questions {}", file.display()));

    daemon.start().await;

    // No window is connected: the notification is the way the human hears of it.
    daemon.notification_with("questions-1.md").await;
    daemon.stop().await.unwrap();
}

#[tokio::test]
async fn a_rescan_reads_the_open_files_whose_save_was_not_reported() {
    let mut daemon = waiting_for_answers().await;
    daemon.stop().await.unwrap();
    let path = daemon.questions_file(1, 1);
    let mut file = QuestionsFile::parse(&std::fs::read_to_string(&path).unwrap()).unwrap();
    file.set_verdict(
        n(1),
        Some(agent_core::questions::Verdict::from_text("ok").unwrap().unwrap()),
    )
    .unwrap();
    file.mark_answered();
    std::fs::write(&path, file.render()).unwrap();
    let (mut owner, _sinks) = daemon.bare_owner().await;

    owner.reported(Report::Rescan).await.unwrap();

    let member = owner.parts.state.find_task(&task(1)).unwrap().member;
    assert_ne!(
        member.status(),
        TaskStatus::Questions,
        "the answers in the file were not read"
    );
}
