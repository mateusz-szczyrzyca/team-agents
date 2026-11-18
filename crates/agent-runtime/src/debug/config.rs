//! Collects configuration files (`agents.toml`) and redacted secret key/table names (`secrets.toml`).

#[cfg(test)]
mod tests;

use std::path::PathBuf;

use agent_core::config::{MAX_CONFIG_BYTES, Secrets};

use crate::store::Home;
use crate::text_file::{TextFileError, read_text_file};

/// Failures reading configuration.
#[derive(Debug, thiserror::Error)]
pub(super) enum ConfigError {
    /// Reading `agents.toml` failed.
    #[error("cannot read configuration file {path:?}")]
    Read {
        /// File path.
        path: PathBuf,
        /// Source error.
        source: TextFileError,
    },
    /// The configuration file does not exist.
    #[error("configuration file {path:?} does not exist")]
    Missing {
        /// File path.
        path: PathBuf,
    },
}

/// Formats `secrets` preserving only table and key names, with no secret values.
#[must_use]
pub(super) fn format_secret_names(secrets: &Secrets) -> String {
    let _ = secrets;
    // STUB: returns empty string to fail RED test
    String::new()
}

/// Reads the configuration file `agents.toml` if present.
///
/// # Errors
/// Returns [`ConfigError`] when reading fails or when the file does not exist.
pub(super) async fn collect_config(home: &Home) -> Result<String, ConfigError> {
    let _ = home;
    // STUB: returns wrong value to fail RED test
    Ok("wrong content".to_owned())
}
