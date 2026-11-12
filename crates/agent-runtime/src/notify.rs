//! Notifications: the file the human can read and the command that carries it to the human's phone.
//!
//! A notification is written as a file — the title on the first line, the body after it, both already rendered by the
//! caller from a template of `prompts/` — and the configured command is run with the path of that file as its last
//! argument. The file is always written; a command that fails is reported with the file's path, so that nothing is
//! lost. The secrets the command needs (`[notification_env]` of `secrets.toml`) are in its environment and nowhere
//! else: not in its arguments, not in the file, not in an error.

use std::collections::BTreeMap;
use std::io;
use std::path::PathBuf;
use std::time::Duration;

use agent_core::config::{CommandLine, EnvName, Notifications, Secret, Secrets};
use agent_core::state::Timestamp;
use tokio::task::JoinError;

use crate::process::{CaptureError, ExitReport};
use crate::store::Home;

/// The longest title, in bytes: one line of a phone's notification.
pub const MAX_TITLE_BYTES: usize = 200;

/// The longest body, in bytes.
pub const MAX_BODY_BYTES: usize = 64 << 10;

/// How long the notification command may run before its group is killed.
const COMMAND_TIMEOUT: Duration = Duration::from_secs(30);

/// How many bytes of the command's output are kept.
const MAX_COMMAND_OUTPUT_BYTES: usize = 64 << 10;

/// How many characters of the command's error output an error carries.
const MAX_DIAGNOSTIC_CHARS: usize = 2048;

/// How many files notifications of one second may have: a bound on the names tried, far above what the harness sends.
pub const MAX_PER_SECOND: u32 = 100;

/// What replaces a secret in the text of an error.
const REDACTED: &str = "[redacted]";

/// Why a text is not a notification.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum InvalidNotification {
    /// The title is empty or only blanks.
    #[error("the title of a notification must not be empty")]
    EmptyTitle,
    /// The title has a line break or another control character.
    #[error("the title of a notification must be one line without control characters")]
    TitleNotOneLine,
    /// The title is longer than [`MAX_TITLE_BYTES`].
    #[error("the title has {len} bytes; the limit is {max}")]
    TitleTooLong {
        /// The title's length, in bytes.
        len: usize,
        /// The limit.
        max: usize,
    },
    /// The body is longer than [`MAX_BODY_BYTES`].
    #[error("the body has {len} bytes; the limit is {max}")]
    BodyTooLong {
        /// The body's length, in bytes.
        len: usize,
        /// The limit.
        max: usize,
    },
}

/// A notification: a one-line title and a body, both final text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notification {
    /// The first line of the file.
    title: String,
    /// What follows the first line.
    body: String,
}

impl Notification {
    /// A notification with `title` and `body`.
    ///
    /// # Errors
    /// [`InvalidNotification`]: an empty or multi-line title, a title over [`MAX_TITLE_BYTES`], a body over
    /// [`MAX_BODY_BYTES`].
    pub fn new(title: &str, body: &str) -> Result<Self, InvalidNotification> {
        let _ = (title, body, MAX_TITLE_BYTES, MAX_BODY_BYTES);
        Ok(Self {
            title: String::new(),
            body: String::new(),
        })
    }
}

/// Why a notification could not be delivered.
#[derive(Debug, thiserror::Error)]
pub enum NotifyError {
    /// The directory of the notification files cannot be created.
    #[error("cannot create the notification directory {path:?}")]
    CreateDir {
        /// The directory.
        path: PathBuf,
        /// What the operating system said.
        source: io::Error,
    },
    /// The notification file cannot be written.
    #[error("cannot write the notification file in {path:?}")]
    Write {
        /// The directory the file is written in.
        path: PathBuf,
        /// What the operating system said.
        source: io::Error,
    },
    /// The directory already holds [`MAX_PER_SECOND`] notifications of this second.
    #[error("{limit} notifications of one second are already in {path:?}")]
    TooManyInOneSecond {
        /// The directory.
        path: PathBuf,
        /// The limit.
        limit: u32,
    },
    /// The thread that writes the file failed.
    #[error("the task that writes a notification failed")]
    Worker {
        /// How the task ended.
        source: JoinError,
    },
    /// The command could not be run to its end: not started, or over its time. The file is written.
    #[error("cannot run the notification command for {file:?}")]
    Command {
        /// The notification file, written.
        file: PathBuf,
        /// Why the command did not run to its end.
        source: CaptureError,
    },
    /// The command ended with a failure. The file is written.
    #[error("the notification command for {file:?} failed with {exit:?}: {stderr}")]
    CommandFailed {
        /// The notification file, written.
        file: PathBuf,
        /// How the command ended.
        exit: ExitReport,
        /// What it said, cut, with the secrets removed; invalid UTF-8 in it is replaced, because it is for a human to
        /// read.
        stderr: String,
    },
}

/// Writes notification files and runs the configured command on them.
#[derive(Debug, Clone)]
pub struct Notifier {
    /// Where the files are written.
    directory: PathBuf,
    /// The command to run with the file's path as its last argument; without one the files are all.
    command: Option<CommandLine>,
    /// The variables the command is run with, besides the daemon's own environment.
    env: BTreeMap<EnvName, Secret>,
    /// How long the command may run.
    command_timeout: Duration,
}

impl Notifier {
    /// The notifier of the configuration: the directory of `[notifications]` or, without one, `notifications/` in
    /// `home`; its command; and the `[notification_env]` of the secrets.
    #[must_use]
    pub fn from_config(config: &Notifications, home: &Home, secrets: &Secrets) -> Self {
        let _ = (config, secrets, COMMAND_TIMEOUT);
        Self {
            directory: home.root().to_path_buf(),
            command: None,
            env: BTreeMap::new(),
            command_timeout: Duration::ZERO,
        }
    }

    /// The same, with another time for the command.
    #[must_use]
    pub fn with_command_timeout(mut self, timeout: Duration) -> Self {
        self.command_timeout = timeout;
        self
    }

    /// Writes `notification` as a file named after `at` and runs the command on it. Returns the file's path.
    ///
    /// The file is `notification-<time>-<n>.md`, the owner's alone (mode 0600), and appears whole under that name.
    ///
    /// # Errors
    /// [`NotifyError::CreateDir`], [`NotifyError::Write`], [`NotifyError::TooManyInOneSecond`] and
    /// [`NotifyError::Worker`] when no file could be written;
    /// [`NotifyError::Command`] and [`NotifyError::CommandFailed`] when the file is written and the command is not
    /// run, or fails.
    pub async fn notify(&self, at: Timestamp, notification: &Notification) -> Result<PathBuf, NotifyError> {
        let _ = (
            at,
            notification,
            MAX_PER_SECOND,
            &self.command,
            &self.env,
            MAX_COMMAND_OUTPUT_BYTES,
            MAX_DIAGNOSTIC_CHARS,
            REDACTED,
        );
        Ok(self.directory.clone())
    }
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::PermissionsExt;
    use std::path::Path;
    use std::str::FromStr;

    use agent_core::config::AbsolutePath;

    use super::*;

    /// A secret the tests look for in places it must not be.
    const SECRET: &str = "s3cr3t-token-value";

    fn abs(path: &Path) -> AbsolutePath {
        AbsolutePath::from_str(path.to_str().unwrap()).unwrap()
    }

    fn at() -> Timestamp {
        "2026-11-12T10:00:00Z".parse().unwrap()
    }

    fn notification(title: &str, body: &str) -> Notification {
        Notification::new(title, body).unwrap()
    }

    fn command(words: &[&str]) -> CommandLine {
        CommandLine::try_from(words.iter().map(|word| (*word).to_owned()).collect::<Vec<_>>()).unwrap()
    }

    fn secrets() -> Secrets {
        Secrets::parse(&format!("[notification_env]\nNOTIFY_TOKEN = \"{SECRET}\"\n"), 0o600).unwrap()
    }

    /// A notifier that writes to `dir/notes` and has no command.
    fn notifier(dir: &Path, command: Option<CommandLine>) -> Notifier {
        let config = Notifications {
            command,
            directory: Some(abs(&dir.join("notes"))),
        };
        Notifier::from_config(&config, &Home::new(&abs(dir)), &secrets())
    }

    fn files_in(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().into_string().unwrap())
            .collect();
        names.sort();
        names
    }

    // ---- the notification ---------------------------------------------------------------------------------------

    // Rule: a title and a body are kept as they are, also in other scripts and with line breaks in the body.
    #[test]
    fn a_notification_keeps_its_title_and_body() {
        let made = Notification::new("Gotowe: żółć 日本語", "line one\nline two\n").unwrap();

        assert_eq!(
            made,
            Notification {
                title: "Gotowe: żółć 日本語".to_owned(),
                body: "line one\nline two\n".to_owned()
            }
        );
    }

    // Rule: the title is one line of text that says something.
    #[test]
    fn a_title_is_one_non_empty_line() {
        for (title, expected) in [
            ("", InvalidNotification::EmptyTitle),
            ("   ", InvalidNotification::EmptyTitle),
            ("two\nlines", InvalidNotification::TitleNotOneLine),
            ("carriage\rreturn", InvalidNotification::TitleNotOneLine),
            ("bell\u{7}", InvalidNotification::TitleNotOneLine),
        ] {
            assert_eq!(Notification::new(title, "body"), Err(expected), "{title:?}");
        }
    }

    // Rule: the title and the body have a size limit, and the limit itself is allowed.
    #[test]
    fn the_title_and_the_body_are_bounded() {
        assert!(Notification::new(&"t".repeat(MAX_TITLE_BYTES), &"b".repeat(MAX_BODY_BYTES)).is_ok());
        assert_eq!(
            Notification::new(&"t".repeat(MAX_TITLE_BYTES + 1), ""),
            Err(InvalidNotification::TitleTooLong {
                len: MAX_TITLE_BYTES + 1,
                max: MAX_TITLE_BYTES
            })
        );
        assert_eq!(
            Notification::new("title", &"b".repeat(MAX_BODY_BYTES + 1)),
            Err(InvalidNotification::BodyTooLong {
                len: MAX_BODY_BYTES + 1,
                max: MAX_BODY_BYTES
            })
        );
        // Bytes, not characters: 'ż' has two.
        assert_eq!(
            Notification::new(&"ż".repeat(MAX_TITLE_BYTES / 2 + 1), ""),
            Err(InvalidNotification::TitleTooLong {
                len: MAX_TITLE_BYTES + 2,
                max: MAX_TITLE_BYTES
            })
        );
    }

    // ---- the file -----------------------------------------------------------------------------------------------

    // Rule: the file has the title on the first line and the body after it, and is in the directory that was
    // configured, created when missing.
    #[tokio::test]
    async fn the_file_has_the_title_first_and_the_body_after_it() {
        let dir = tempfile::tempdir().unwrap();

        let file = notifier(dir.path(), None)
            .notify(at(), &notification("Task done", "Item 4.1 is merged.\nNext: 4.2\n"))
            .await
            .unwrap();

        assert_eq!(file.parent().unwrap(), dir.path().join("notes"));
        assert_eq!(
            std::fs::read_to_string(&file).unwrap(),
            "Task done\nItem 4.1 is merged.\nNext: 4.2\n"
        );
        assert_eq!(
            files_in(&dir.path().join("notes")),
            [file.file_name().unwrap().to_str().unwrap()]
        );
    }

    // Rule: a notification without a body is the title and its line break.
    #[tokio::test]
    async fn a_notification_without_a_body_is_one_line() {
        let dir = tempfile::tempdir().unwrap();

        let file = notifier(dir.path(), None)
            .notify(at(), &notification("Paused", ""))
            .await
            .unwrap();

        assert_eq!(std::fs::read_to_string(file).unwrap(), "Paused\n");
    }

    // Rule: a file is named after the time of its notification, so that a directory sorts by when things happened; a
    // second notification of the same second gets the next number; no temporary file stays.
    #[tokio::test]
    async fn files_are_named_after_their_time_and_numbered_within_a_second() {
        let dir = tempfile::tempdir().unwrap();
        let notifier = notifier(dir.path(), None);
        let later: Timestamp = "2026-11-12T10:05:00Z".parse().unwrap();

        let first = notifier.notify(at(), &notification("One", "")).await.unwrap();
        let second = notifier.notify(at(), &notification("Two", "")).await.unwrap();
        let third = notifier.notify(later, &notification("Three", "")).await.unwrap();

        let name = |path: &Path| path.file_name().unwrap().to_str().unwrap().to_owned();
        assert_eq!(name(&first), "notification-2026-11-12T100000Z-1.md");
        assert_eq!(name(&second), "notification-2026-11-12T100000Z-2.md");
        assert_eq!(name(&third), "notification-2026-11-12T100500Z-1.md");
        assert_eq!(std::fs::read_to_string(&first).unwrap(), "One\n");
        assert_eq!(std::fs::read_to_string(&second).unwrap(), "Two\n");
        assert_eq!(files_in(&dir.path().join("notes")).len(), 3);
    }

    // Rule: the numbers of one second are bounded; the notification that does not fit is an error, and nothing of it
    // stays in the directory.
    #[tokio::test]
    async fn the_notifications_of_one_second_are_bounded() {
        let dir = tempfile::tempdir().unwrap();
        let notifier = notifier(dir.path(), None);
        for _ in 0..MAX_PER_SECOND {
            notifier.notify(at(), &notification("Burst", "")).await.unwrap();
        }

        let result = notifier.notify(at(), &notification("One too many", "")).await;

        assert!(
            matches!(&result, Err(NotifyError::TooManyInOneSecond { path, limit: MAX_PER_SECOND }) if *path == dir.path().join("notes")),
            "{result:?}"
        );
        assert_eq!(
            files_in(&dir.path().join("notes")).len(),
            usize::try_from(MAX_PER_SECOND).unwrap()
        );
    }

    // Rule: the file is the owner's alone: a notification may say what the human has not published.
    #[tokio::test]
    async fn the_file_is_readable_by_its_owner_only() {
        let dir = tempfile::tempdir().unwrap();

        let file = notifier(dir.path(), None)
            .notify(at(), &notification("Private", ""))
            .await
            .unwrap();

        assert_eq!(std::fs::metadata(file).unwrap().permissions().mode() & 0o777, 0o600);
    }

    // Rule: a directory that cannot be created is an error that names it.
    #[tokio::test]
    async fn a_directory_that_cannot_be_created_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("notes"), "a file").unwrap();

        let result = notifier(dir.path(), None).notify(at(), &notification("x", "")).await;

        assert!(
            matches!(&result, Err(NotifyError::CreateDir { path, .. }) if *path == dir.path().join("notes")),
            "{result:?}"
        );
    }

    // ---- the configuration --------------------------------------------------------------------------------------

    // Rule: without a directory in the configuration the files go to `notifications/` of the home.
    #[tokio::test]
    async fn the_default_directory_is_in_the_home() {
        let dir = tempfile::tempdir().unwrap();
        let notifier = Notifier::from_config(&Notifications::default(), &Home::new(&abs(dir.path())), &secrets());

        let file = notifier.notify(at(), &notification("Default", "")).await.unwrap();

        assert_eq!(file.parent().unwrap(), dir.path().join("notifications"));
    }

    // Rule: a notifier never shows the secrets it holds.
    #[test]
    fn debug_output_shows_no_secret() {
        let dir = tempfile::tempdir().unwrap();

        let shown = format!("{:?}", notifier(dir.path(), None));

        assert!(shown.contains("NOTIFY_TOKEN"), "{shown}");
        assert!(!shown.contains(SECRET), "{shown}");
    }

    // ---- the command --------------------------------------------------------------------------------------------

    /// A command that records what it got into `dir/record`: its extra argument, the path it was given last, the
    /// secret in its environment and its working directory.
    fn recording_command(dir: &Path) -> CommandLine {
        let script = format!(
            "printf '%s|%s|%s|%s' \"$1\" \"$2\" \"$NOTIFY_TOKEN\" \"$PWD\" > '{}'",
            dir.join("record").display()
        );
        command(&["sh", "-c", &script, "sh", "extra-argument"])
    }

    // Rule: the command is run with its own arguments and the file's path as the last one, with the secrets in its
    // environment, in the notification directory.
    #[tokio::test]
    async fn the_command_gets_the_file_as_its_last_argument_and_the_secrets_in_its_environment() {
        let dir = tempfile::tempdir().unwrap();
        let notifier = notifier(dir.path(), Some(recording_command(dir.path())));

        let file = notifier.notify(at(), &notification("Run", "body")).await.unwrap();

        let record = std::fs::read_to_string(dir.path().join("record")).unwrap();
        let notes = dir.path().join("notes").canonicalize().unwrap();
        assert_eq!(
            record,
            format!("extra-argument|{}|{SECRET}|{}", file.display(), notes.display())
        );
    }

    // Rule: the secrets are never in the file or in the arguments.
    #[tokio::test]
    async fn the_secrets_are_not_in_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let notifier = notifier(dir.path(), Some(recording_command(dir.path())));

        let file = notifier.notify(at(), &notification("Run", "body")).await.unwrap();

        assert!(!std::fs::read_to_string(file).unwrap().contains(SECRET));
    }

    // Rule: a command that fails leaves the file and is reported with it; what it said is cut and has no secret.
    #[tokio::test]
    async fn a_failing_command_is_reported_with_the_file_and_without_secrets() {
        let dir = tempfile::tempdir().unwrap();
        let failing = command(&["sh", "-c", "echo \"login with $NOTIFY_TOKEN failed\" >&2; exit 3"]);
        let notifier = notifier(dir.path(), Some(failing));

        let result = notifier.notify(at(), &notification("Fail", "body")).await;

        let Err(NotifyError::CommandFailed { file, exit, stderr }) = result else {
            panic!("expected a failed command: {result:?}")
        };
        assert_eq!(exit, ExitReport::Exited { code: 3 });
        assert_eq!(stderr, "login with [redacted] failed");
        assert_eq!(std::fs::read_to_string(file).unwrap(), "Fail\nbody");
    }

    // Rule: a command that cannot be started is reported with the file that was written.
    #[tokio::test]
    async fn a_command_that_cannot_be_started_is_reported_with_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let notifier = notifier(dir.path(), Some(command(&["/nonexistent/agent-test-notify"])));

        let result = notifier.notify(at(), &notification("Missing", "")).await;

        let Err(NotifyError::Command {
            file,
            source: CaptureError::Spawn { .. },
        }) = result
        else {
            panic!("expected a command that does not start: {result:?}")
        };
        assert_eq!(std::fs::read_to_string(file).unwrap(), "Missing\n");
    }

    // Rule: a command that hangs is killed after its time.
    #[tokio::test]
    async fn a_command_over_its_time_is_killed_and_reported() {
        let dir = tempfile::tempdir().unwrap();
        let notifier = notifier(dir.path(), Some(command(&["sh", "-c", "sleep 300"])))
            .with_command_timeout(Duration::from_millis(300));

        let result = notifier.notify(at(), &notification("Slow", "")).await;

        assert!(
            matches!(
                &result,
                Err(NotifyError::Command {
                    source: CaptureError::Timeout { .. },
                    ..
                })
            ),
            "{result:?}"
        );
    }
}
