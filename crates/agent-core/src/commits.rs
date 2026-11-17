//! Linking commit hashes in text to full commit identifiers.
//!
//! Given text and a callback to resolve short commit hashes to full IDs, finds and links
//! 7-40 character hex sequences, creating Markdown links. Avoids re-linking inside code
//! fences and existing Markdown links.

/// Stub: returns text unchanged
pub fn link_commits<F>(text: &str, _repo_url: &str, _resolve: F) -> String
where
    F: FnMut(&str) -> Option<String>,
{
    text.to_string()
}

#[cfg(test)]
mod tests;
