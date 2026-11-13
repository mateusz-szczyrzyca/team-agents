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

/// Where in a line the text of a marker has to be.
#[derive(Debug, Clone, Copy)]
enum Place {
    /// The line starts with the text: an attribute, or the call of a skip that is a statement of its own.
    Start,
    /// The line starts with the text as a word: `skip`, but not `skipped=1`.
    Word,
    /// The text is a method call and nothing before it quotes or comments it out: `t.Skip(`, `s.T().SkipNow(`.
    Call,
    /// The text is anywhere in the line: a comment that follows code, a name in a condition.
    Anywhere,
}

/// The texts of one kind of marker for the files with some extensions.
#[derive(Debug, Clone, Copy)]
struct Rule {
    /// The extensions of the files the texts are looked for in, without the dot.
    extensions: &'static [&'static str],
    /// The kind the texts mark.
    marker: Marker,
    /// Where in a line they have to be.
    place: Place,
    /// The texts, in the form of a line whose runs of white space are one space.
    texts: &'static [&'static str],
}

/// The extensions of Go files.
const GO: &[&str] = &["go"];

/// The extensions of Rust files.
const RUST: &[&str] = &["rs"];

/// The extensions of shell scripts and bats tests.
const SHELL: &[&str] = &["sh", "bash", "bats"];

/// The extensions of Python files.
const PYTHON: &[&str] = &["py"];

/// The extensions of JavaScript and TypeScript files.
const SCRIPT: &[&str] = &["js", "jsx", "ts", "tsx", "mjs", "cjs"];

/// The table of markers. Each text is what the language's own tools look for, in the spellings they accept.
const RULES: &[Rule] = &[
    Rule {
        extensions: GO,
        marker: Marker::SkipsTest,
        place: Place::Call,
        texts: &[".Skip(", ".Skipf(", ".SkipNow("],
    },
    Rule {
        extensions: RUST,
        marker: Marker::SkipsTest,
        place: Place::Start,
        texts: &["#[ignore"],
    },
    Rule {
        extensions: SHELL,
        marker: Marker::SkipsTest,
        place: Place::Word,
        texts: &["skip"],
    },
    Rule {
        extensions: PYTHON,
        marker: Marker::SkipsTest,
        place: Place::Start,
        texts: &["pytest.skip(", "@pytest.mark.skip", "@unittest.skip", "self.skipTest("],
    },
    Rule {
        extensions: SCRIPT,
        marker: Marker::SkipsTest,
        place: Place::Start,
        texts: &[
            "it.skip(",
            "test.skip(",
            "describe.skip(",
            "xit(",
            "xtest(",
            "xdescribe(",
        ],
    },
    Rule {
        extensions: GO,
        marker: Marker::SuppressesLint,
        place: Place::Anywhere,
        texts: &["//nolint", "// nolint", "#nosec", "# nosec"],
    },
    Rule {
        extensions: RUST,
        marker: Marker::SuppressesLint,
        place: Place::Start,
        texts: &["#[allow(", "#![allow(", "#[expect(", "#![expect("],
    },
    Rule {
        extensions: SHELL,
        marker: Marker::SuppressesLint,
        place: Place::Anywhere,
        texts: &["# shellcheck disable=", "#shellcheck disable="],
    },
    Rule {
        extensions: PYTHON,
        marker: Marker::SuppressesLint,
        place: Place::Anywhere,
        texts: &[
            "# noqa",
            "#noqa",
            "# type: ignore",
            "#type: ignore",
            "# nosec",
            "#nosec",
        ],
    },
    Rule {
        extensions: SCRIPT,
        marker: Marker::SuppressesLint,
        place: Place::Anywhere,
        texts: &["@ts-ignore", "@ts-expect-error", "@ts-nocheck", "eslint-disable"],
    },
    Rule {
        extensions: GO,
        marker: Marker::ChecksUnderTest,
        place: Place::Anywhere,
        texts: &["testing.Testing()", "flag.Lookup(\"test."],
    },
    Rule {
        extensions: RUST,
        marker: Marker::ChecksUnderTest,
        place: Place::Anywhere,
        texts: &["cfg!(test)"],
    },
    Rule {
        extensions: SHELL,
        marker: Marker::ChecksUnderTest,
        place: Place::Anywhere,
        texts: &["BATS_"],
    },
    Rule {
        extensions: PYTHON,
        marker: Marker::ChecksUnderTest,
        place: Place::Anywhere,
        texts: &[
            "PYTEST_CURRENT_TEST",
            "\"pytest\" in sys.modules",
            "'pytest' in sys.modules",
        ],
    },
    Rule {
        extensions: SCRIPT,
        marker: Marker::ChecksUnderTest,
        place: Place::Anywhere,
        texts: &[
            "NODE_ENV === 'test'",
            "NODE_ENV === \"test\"",
            "NODE_ENV==='test'",
            "NODE_ENV===\"test\"",
            "JEST_WORKER_ID",
        ],
    },
];

/// Whether files with this extension have markers: the added lines of the others are not looked at.
#[must_use]
pub fn scans(extension: &str) -> bool {
    RULES.iter().any(|rule| rule.extensions.contains(&extension))
}

/// The kinds of markers that `line` of a file with this `extension` has, in the order of [`Marker`].
#[must_use]
pub fn markers_of(extension: &str, line: &str) -> Vec<Marker> {
    let collapsed = line.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut found: Vec<Marker> = Vec::new();
    for rule in RULES.iter().filter(|rule| rule.extensions.contains(&extension)) {
        if !found.contains(&rule.marker) && rule.texts.iter().any(|text| rule.place.has(&collapsed, text)) {
            found.push(rule.marker);
        }
    }
    found.sort();
    found
}

impl Place {
    /// Whether `line`, with its runs of white space made one space, has `text` where this place says.
    fn has(self, line: &str, text: &str) -> bool {
        match self {
            Self::Start => line.starts_with(text),
            Self::Word => line
                .strip_prefix(text)
                .is_some_and(|rest| !rest.starts_with(|c: char| c.is_alphanumeric() || c == '_')),
            Self::Call => line
                .match_indices(text)
                .any(|(at, _)| line.get(..at).is_some_and(|before| !before.contains(['"', '`', '/']))),
            Self::Anywhere => line.contains(text),
        }
    }
}

#[cfg(test)]
mod tests;
