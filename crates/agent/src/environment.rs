//! What a command reads from the process's environment, once, at its start.

use std::ffi::{OsStr, OsString};
use std::io;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::Arc;

use agent_core::config::{AbsolutePath, CommandLine, InvalidValue};
use agent_core::local_time::Zone;
use agent_runtime::settings::{Settings, SettingsError};
use agent_runtime::store::{HOME_VARIABLE, Home};
use time::error::IndeterminateOffset;
use time::{OffsetDateTime, UtcOffset};

/// The names of the variables that name an editor, in the order they are tried.
const EDITOR_VARIABLES: [&str; 2] = ["VISUAL", "EDITOR"];

/// The environment variables a command uses, and the machine's offset from UTC.
#[derive(Debug, Clone)]
pub(crate) struct Environment {
    /// `AGENTS_HOME`, as the process has it.
    home: Option<OsString>,
    /// The directory the process runs in, or why it could not be told.
    directory: Result<PathBuf, Arc<io::Error>>,
    /// `VISUAL` and `EDITOR`, as the process has them, the first that is set and not blank.
    editor: Option<OsString>,
    /// The machine's offset from UTC, or UTC when the system cannot say.
    local_offset: UtcOffset,
    /// Why the offset is UTC, when it is for that reason.
    offset_error: Option<IndeterminateOffset>,
}

/// The machine's offset from UTC at `at`, or nothing when the machine cannot tell it: the zone then shows UTC, marked.
fn system_offset_at(at: OffsetDateTime) -> Option<UtcOffset> {
    if !zone_is_known() {
        return None;
    }
    // A failure is the specified fallback of `Zone::Lookup`, which marks the time as UTC; the reason is not kept.
    UtcOffset::local_offset_at(at).ok()
}

/// Whether the zone of the process is one the system knows. glibc reads an unknown `TZ` as UTC without a failure, so the
/// name is checked here: without `TZ` the system's zone is `/etc/localtime`.
fn zone_is_known() -> bool {
    match std::env::var_os("TZ") {
        None => Path::new("/etc/localtime").exists(),
        Some(value) => tz_is_known(&value),
    }
}

/// Whether `value`, a `TZ` the process has, names a zone: a file of the zone database (absolute, or in `TZDIR` or
/// `/usr/share/zoneinfo`), or a POSIX rule such as `UTC0` or `CET-1CEST,M3.5.0,M10.5.0/3`, which has a digit and no
/// leading `:`.
fn tz_is_known(value: &OsStr) -> bool {
    let Some(text) = value.to_str() else {
        return false;
    };
    let (name, colon) = match text.strip_prefix(':') {
        Some(name) => (name, true),
        None => (text, false),
    };
    if name.is_empty() {
        return false;
    }
    if name.starts_with('/') {
        return Path::new(name).is_file();
    }
    let tzdir = std::env::var_os("TZDIR").map_or_else(|| PathBuf::from("/usr/share/zoneinfo"), PathBuf::from);
    if tzdir.join(name).is_file() {
        return true;
    }
    !colon && name.bytes().any(|byte| byte.is_ascii_digit())
}

/// Why `AGENTS_HOME` cannot be used.
#[derive(Debug, thiserror::Error)]
pub(crate) enum EnvironmentError {
    /// The variable is not set.
    #[error("{HOME_VARIABLE} is not set: it names the directory the harness keeps its files in")]
    NoHome,
    /// The variable is not UTF-8 text.
    #[error("{HOME_VARIABLE} is not UTF-8 text: {value:?}")]
    NotText {
        /// The value.
        value: OsString,
    },
    /// The variable is not an absolute path.
    #[error("{HOME_VARIABLE} must be an absolute path, not {value:?}")]
    NotAbsolute {
        /// The value.
        value: String,
        /// What is wrong with it.
        source: InvalidValue,
    },
    /// The directory the command runs in cannot be found.
    #[error("cannot find the directory the command runs in")]
    NoDirectory(#[source] Arc<io::Error>),
}

impl Environment {
    /// The environment of this process.
    ///
    /// Call it at the start of the process, while it has one thread: the system refuses to name the time zone of a
    /// process that has more.
    pub(crate) fn from_process() -> Self {
        // `time` reads `TZ` and the zone database once, while the process has one thread. `None` means they could not be
        // read now; the lookup of each moment then reports the zone it cannot find as UTC.
        if time::util::refresh_tz().is_none() {
            // Nothing else to do: the lookup of each moment is the one that decides the zone.
        }
        let (local_offset, offset_error) = match UtcOffset::current_local_offset() {
            Ok(offset) => (offset, None),
            // UTC is the specified fallback; the daemon logs why.
            Err(error) => (UtcOffset::UTC, Some(error)),
        };
        Self {
            home: std::env::var_os(HOME_VARIABLE),
            directory: std::env::current_dir().map_err(Arc::new),
            editor: EDITOR_VARIABLES
                .iter()
                .copied()
                .filter_map(std::env::var_os)
                .find(|value| !value.to_string_lossy().trim().is_empty()),
            local_offset,
            offset_error,
        }
    }

    /// An environment whose `AGENTS_HOME` is `home`, in the time zone UTC, with no directory.
    #[cfg(test)]
    pub(crate) fn new(home: Option<OsString>) -> Self {
        Self {
            home,
            directory: Err(Arc::new(io::Error::from(io::ErrorKind::NotFound))),
            editor: None,
            local_offset: UtcOffset::UTC,
            offset_error: None,
        }
    }

    /// The same, running in `directory`.
    #[cfg(test)]
    pub(crate) fn in_directory(mut self, directory: PathBuf) -> Self {
        self.directory = Ok(directory);
        self
    }

    /// The same, with `VISUAL` or `EDITOR` set to `editor`.
    #[cfg(test)]
    pub(crate) fn with_editor(mut self, editor: &str) -> Self {
        self.editor = Some(OsString::from(editor));
        self
    }

    /// The directory the command runs in.
    ///
    /// # Errors
    /// [`EnvironmentError::NoDirectory`] when the process cannot tell it.
    pub(crate) fn directory(&self) -> Result<&Path, EnvironmentError> {
        self.directory
            .as_deref()
            .map_err(|error| EnvironmentError::NoDirectory(Arc::clone(error)))
    }

    /// The zone the times a person reads in this process are written in: the offset the machine has at each moment, so
    /// a time before a change of summer time has the offset of its day. UTC, marked, when the machine cannot tell it.
    pub(crate) fn zone() -> Zone {
        Zone::Lookup(system_offset_at)
    }

    /// The machine's offset from UTC.
    pub(crate) const fn local_offset(&self) -> UtcOffset {
        self.local_offset
    }

    /// Why the offset is UTC although the machine may not be, if that is why.
    pub(crate) const fn offset_error(&self) -> Option<&IndeterminateOffset> {
        self.offset_error.as_ref()
    }

    /// The home of the harness.
    ///
    /// # Errors
    /// [`EnvironmentError`] when `AGENTS_HOME` is not set, not text or not an absolute path.
    pub(crate) fn home(&self) -> Result<Home, EnvironmentError> {
        let value = self.home.as_ref().ok_or(EnvironmentError::NoHome)?;
        let text = value
            .to_str()
            .ok_or_else(|| EnvironmentError::NotText { value: value.clone() })?;
        let root = AbsolutePath::from_str(text).map_err(|source| EnvironmentError::NotAbsolute {
            value: text.to_owned(),
            source,
        })?;
        Ok(Home::new(&root))
    }
}

impl Environment {
    /// The editor `agent answer` starts: the one `[questions] editor` of the configuration of `home` names, else the one
    /// `VISUAL` or `EDITOR` names (a program and its arguments, split at blanks).
    ///
    /// # Errors
    /// [`EditorError`].
    pub(crate) async fn editor(&self, home: &Home) -> Result<CommandLine, EditorError> {
        let settings = Settings::load(home).await?;
        if let Some(editor) = settings.config.questions().editor.clone() {
            return Ok(editor);
        }
        let text = self
            .editor
            .as_ref()
            .ok_or(EditorError::NoEditor)?
            .to_str()
            .ok_or(EditorError::NotText)?;
        let words: Vec<String> = text.split_whitespace().map(str::to_owned).collect();
        CommandLine::try_from(words).map_err(|source| EditorError::NotACommand {
            text: text.to_owned(),
            source,
        })
    }
}

/// Why no editor can be started.
#[derive(Debug, thiserror::Error)]
pub(crate) enum EditorError {
    /// The settings of the home cannot be read.
    #[error("cannot read the settings to find the editor")]
    Settings(#[from] SettingsError),
    /// No editor is configured or named by the environment.
    #[error(
        "no editor: set `editor` in [questions] of agents.toml, or $VISUAL or $EDITOR, or give your answers in a file with --file"
    )]
    NoEditor,
    /// The variable is not UTF-8 text.
    #[error("$VISUAL or $EDITOR is not UTF-8 text")]
    NotText,
    /// The variable is not a program and its arguments.
    #[error("$VISUAL or $EDITOR, {text:?}, is not a command")]
    NotACommand {
        /// What the variable says.
        text: String,
        /// What is wrong with it.
        source: InvalidValue,
    },
}

#[cfg(test)]
mod tests;
