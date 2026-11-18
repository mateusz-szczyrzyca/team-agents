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
    /// Configuration cannot be parsed to locate notification files.
    #[error("cannot parse notification configuration in {path:?}")]
    Parse {
        /// Configuration file path.
        path: PathBuf,
        /// Invalid configuration and its original cause.
        source: agent_core::config::ConfigError,
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
    let mut lines = Vec::new();
    if secrets.github_token().is_some() {
        lines.push("github_token".to_owned());
    }
    if !secrets.agent_env().is_empty() {
        if !lines.is_empty() {
            lines.push(String::new());
        }
        lines.push("[agent_env]".to_owned());
        for key in secrets.agent_env().keys() {
            lines.push(key.to_string());
        }
    }
    if !secrets.notification_env().is_empty() {
        if !lines.is_empty() {
            lines.push(String::new());
        }
        lines.push("[notification_env]".to_owned());
        for key in secrets.notification_env().keys() {
            lines.push(key.to_string());
        }
    }
    if lines.is_empty() {
        String::new()
    } else {
        format!("{}\n", lines.join("\n"))
    }
}

/// Reads the configuration file `agents.toml` if present.
///
/// # Errors
/// Returns [`ConfigError`] when reading fails or when the file does not exist.
pub(super) async fn collect_config(home: &Home) -> Result<String, ConfigError> {
    let path = home.config_file();
    match read_text_file(&path, MAX_CONFIG_BYTES).await {
        Ok(Some(text_file)) => Ok(text_file.text),
        Ok(None) => Err(ConfigError::Missing { path }),
        Err(source) => Err(ConfigError::Read { path, source }),
    }
}

/// Resolves the notifier's configured directory, defaulting only when no setting exists.
///
/// # Errors
/// Returns [`ConfigError`] when an existing configuration cannot be read or parsed.
pub(super) async fn notification_directory(home: &Home) -> Result<PathBuf, ConfigError> {
    let text = match collect_config(home).await {
        Ok(text) => text,
        Err(ConfigError::Missing { .. }) => return Ok(home.notifications_dir()),
        Err(error) => return Err(error),
    };
    let config = agent_core::config::Config::from_toml_str(&text).map_err(|source| ConfigError::Parse {
        path: home.config_file(),
        source,
    })?;
    Ok(config.notifications().directory.as_ref().map_or_else(
        || home.notifications_dir(),
        |directory| directory.as_path().to_path_buf(),
    ))
}
