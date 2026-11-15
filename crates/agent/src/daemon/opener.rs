//! The opener: opens the questions files the owner asks for in the window of VS Code, so that a slow command never holds
//! up the state.

use std::path::PathBuf;

use agent_core::log::LogEvent;
use agent_runtime::editor::{Opened, VscodeOpener};
use agent_runtime::process::error_chain;
use tokio::sync::mpsc;

/// How many files may wait to be opened; more are dropped and logged.
pub(super) const QUEUE: usize = 16;

/// Opens what arrives on `queue` until it is closed and empty.
///
/// A file that cannot be opened is logged and goes no further: the human is told by the notification, and the file is
/// where it was.
pub(super) async fn run(mut queue: mpsc::Receiver<PathBuf>, opener: VscodeOpener) {
    while let Some(file) = queue.recv().await {
        match opener.open(&file).await {
            Ok(Opened::Window { socket }) => {
                tracing::info!(
                    event = LogEvent::QuestionsFileOpened.as_str(),
                    outcome = "window",
                    file = %file.display(),
                    socket = %socket.display(),
                    "a questions file is opened in VS Code"
                );
            }
            Ok(Opened::NoServer) => {
                tracing::info!(
                    event = LogEvent::QuestionsFileOpened.as_str(),
                    outcome = "no_server",
                    file = %file.display(),
                    "no VS Code server on this host: the file is not opened"
                );
            }
            Ok(Opened::NoWindow) => {
                tracing::info!(
                    event = LogEvent::QuestionsFileOpened.as_str(),
                    outcome = "no_window",
                    file = %file.display(),
                    "no window of VS Code is connected: the file is not opened"
                );
            }
            Err(error) => {
                tracing::warn!(
                    event = LogEvent::QuestionsFileInvalid.as_str(),
                    file = %file.display(),
                    error = %error_chain(&error),
                    "a questions file cannot be opened in VS Code"
                );
            }
        }
    }
}
