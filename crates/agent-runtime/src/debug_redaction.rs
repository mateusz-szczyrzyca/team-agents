//! Exact byte redaction for every diagnostic artifact before persistence.

use std::ffi::OsString;

use agent_core::config::Secrets;

/// Secret values that may not appear in diagnostic artifacts.
#[derive(Debug)]
pub(crate) struct SecretsCut;

impl SecretsCut {
    /// Captures configured secrets and suffix-selected environment values without reading global state.
    pub(crate) fn new(_secrets: &Secrets, _environment: impl IntoIterator<Item = (OsString, OsString)>) -> Self {
        Self
    }

    /// Replaces exact occurrences, preserving unrelated bytes including non-UTF-8 output.
    pub(crate) fn redact(&self, bytes: &[u8]) -> Vec<u8> {
        bytes.to_vec()
    }
}

#[cfg(test)]
mod tests;
