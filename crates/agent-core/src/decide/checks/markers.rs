//! The markers of the checks: the lines that skip a test, suppress a lint or ask whether the code runs under test, as a
//! built-in table per file extension.

/// The kind of a marker.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, strum::Display)]
#[strum(serialize_all = "snake_case")]
pub enum Marker {
    /// The line skips a test (TDD-11).
    SkipsTest,
    /// The line suppresses a lint (LINT-3).
    SuppressesLint,
    /// The line asks whether the code runs under test (TDD-12).
    ChecksUnderTest,
}

/// Whether files with this extension have markers: the added lines of the others are not looked at.
#[must_use]
pub fn scans(_extension: &str) -> bool {
    false
}

/// The kinds of markers that `line` of a file with this `extension` has, in the order of [`Marker`].
#[must_use]
pub fn markers_of(_extension: &str, _line: &str) -> Vec<Marker> {
    Vec::new()
}

#[cfg(test)]
mod tests;
