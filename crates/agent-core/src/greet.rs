//! Formats a greeting for a supplied name.

/// Returns `Hello, <name>!`, preserving the supplied name exactly.
#[must_use]
pub fn greet(name: &str) -> String {
    format!("Hello, {name}!")
}

#[cfg(test)]
mod tests;
