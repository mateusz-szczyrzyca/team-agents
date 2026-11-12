//! What a command reads from the process's environment, once, at its start.

use std::ffi::OsString;
use std::str::FromStr;

use agent_core::config::{AbsolutePath, InvalidValue};
use agent_runtime::store::Home;
use time::UtcOffset;
use time::error::IndeterminateOffset;

/// The name of the variable that says where the harness keeps its files.
pub(crate) const HOME_VARIABLE: &str = "AGENTS_HOME";

/// The environment variables a command uses, and the machine's offset from UTC.
#[derive(Debug, Clone)]
pub(crate) struct Environment {
    /// `AGENTS_HOME`, as the process has it.
    home: Option<OsString>,
    /// The machine's offset from UTC, or UTC when the system cannot say.
    local_offset: UtcOffset,
    /// Why the offset is UTC, when it is for that reason.
    offset_error: Option<IndeterminateOffset>,
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
}

impl Environment {
    /// The environment of this process.
    ///
    /// Call it at the start of the process, while it has one thread: the system refuses to name the time zone of a
    /// process that has more.
    pub(crate) fn from_process() -> Self {
        let (local_offset, offset_error) = match UtcOffset::current_local_offset() {
            Ok(offset) => (offset, None),
            // UTC is the specified fallback; the daemon logs why.
            Err(error) => (UtcOffset::UTC, Some(error)),
        };
        Self {
            home: std::env::var_os(HOME_VARIABLE),
            local_offset,
            offset_error,
        }
    }

    /// An environment whose `AGENTS_HOME` is `home`, in the time zone UTC.
    pub(crate) const fn new(home: Option<OsString>) -> Self {
        Self {
            home,
            local_offset: UtcOffset::UTC,
            offset_error: None,
        }
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

#[cfg(test)]
mod tests;
