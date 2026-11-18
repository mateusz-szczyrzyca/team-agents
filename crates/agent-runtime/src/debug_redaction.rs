//! Exact byte redaction for every diagnostic artifact before persistence.

use std::ffi::OsString;
use std::fmt;
use std::os::unix::ffi::OsStringExt;

use agent_core::config::Secrets;

/// Secret values that may not appear in diagnostic artifacts.
pub(crate) struct SecretsCut {
    /// Nonempty byte patterns, longest first so nested values cannot leave a secret suffix.
    values: Vec<Vec<u8>>,
}

impl fmt::Debug for SecretsCut {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SecretsCut").field("values", &"<redacted>").finish()
    }
}

impl SecretsCut {
    /// Captures configured secrets and suffix-selected environment values without reading global state.
    pub(crate) fn new(secrets: &Secrets, environment: impl IntoIterator<Item = (OsString, OsString)>) -> Self {
        let mut values: Vec<_> = secrets
            .github_token()
            .into_iter()
            .chain(secrets.agent_env().values())
            .chain(secrets.notification_env().values())
            .map(|value| value.expose().as_bytes().to_vec())
            .collect();
        for (name, value) in environment {
            if ["TOKEN", "SECRET", "PASSWORD", "KEY"]
                .iter()
                .any(|suffix| name.as_encoded_bytes().ends_with(suffix.as_bytes()))
                && !value.is_empty()
            {
                // An empty environment value contains no confidential bytes to replace.
                values.push(value.into_vec());
            }
        }
        values.sort_unstable_by(|left, right| right.len().cmp(&left.len()).then_with(|| left.cmp(right)));
        values.dedup();
        Self { values }
    }

    /// Replaces secret values that can occur in a UTF-8 presentation string.
    pub(crate) fn redact_text(&self, text: &str) -> String {
        let mut remaining = text;
        let mut redacted = String::with_capacity(text.len());
        loop {
            let mut chars = remaining.chars();
            let Some(first) = chars.next() else { break };
            if let Some(rest) = self.values.iter().find_map(|value| {
                match std::str::from_utf8(value) {
                    Ok(value) => remaining.strip_prefix(value),
                    // A non-UTF-8 environment value cannot occur inside UTF-8 text.
                    Err(_) => None,
                }
            }) {
                redacted.push_str("<cut>");
                remaining = rest;
            } else {
                redacted.push(first);
                remaining = chars.as_str();
            }
        }
        redacted
    }

    /// Replaces exact occurrences, preserving unrelated bytes including non-UTF-8 output.
    pub(crate) fn redact(&self, bytes: &[u8]) -> Vec<u8> {
        let mut remaining = bytes;
        let mut redacted = Vec::with_capacity(bytes.len());
        while let Some((first, tail)) = remaining.split_first() {
            if let Some(rest) = self
                .values
                .iter()
                .find_map(|value| remaining.strip_prefix(value.as_slice()))
            {
                redacted.extend_from_slice(b"<cut>");
                remaining = rest;
            } else {
                redacted.push(*first);
                remaining = tail;
            }
        }
        redacted
    }
}

#[cfg(test)]
mod tests;
