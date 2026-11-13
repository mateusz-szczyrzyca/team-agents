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
    /// The text is an attribute that the `cfg_attr` the line starts with applies, as a word of its own:
    /// `#[cfg_attr(miri, ignore)]`, but not `#[cfg_attr(test, my_allow(x))]`.
    Applied,
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

/// How an attribute starts that applies other attributes under a condition: `#[cfg_attr(not(test), expect(…))]`.
const CFG_ATTR: [&str; 2] = ["#[cfg_attr(", "#![cfg_attr("];

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
        extensions: RUST,
        marker: Marker::SkipsTest,
        place: Place::Applied,
        texts: &["ignore"],
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
        extensions: RUST,
        marker: Marker::SuppressesLint,
        place: Place::Applied,
        texts: &["allow(", "expect("],
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

/// An attribute that rustfmt wrapped over lines, as far as it has been read.
#[derive(Debug)]
struct Wrapped {
    /// The number of its first line: where its markers are reported.
    first: u32,
    /// The number of the last line read into it.
    last: u32,
    /// Its lines so far, each trimmed, joined by a space.
    text: String,
}

impl Wrapped {
    /// The markers of the whole attribute, in a file with this `extension`, at the number of its first line.
    fn markers(&self, extension: &str) -> impl Iterator<Item = (u32, Marker)> {
        let first = self.first;
        markers_of(extension, &self.text)
            .into_iter()
            .map(move |marker| (first, marker))
    }
}

/// The markers of the added lines of one file with this `extension`, each with the number of the line that has it:
/// `lines` are the numbers and texts of the file's added lines, in the order of their numbers.
///
/// Each line is read on its own, and an attribute that rustfmt wrapped — its opening `#[cfg_attr(` alone on a line, what
/// it applies on the lines after it, up to its closing `)]` — is read once more as one line, at the number of its first.
/// A line that does not follow the one before it ends the attribute: what lies between was not added and is not known.
#[must_use]
pub fn markers_in(extension: &str, lines: &[(u32, &str)]) -> Vec<(u32, Marker)> {
    let mut found: Vec<(u32, Marker)> = Vec::new();
    let mut open: Option<Wrapped> = None;
    for &(number, text) in lines {
        found.extend(markers_of(extension, text).into_iter().map(|marker| (number, marker)));
        let trimmed = text.trim();
        if let Some(mut wrapped) = open.take() {
            if wrapped.last.checked_add(1) == Some(number) {
                wrapped.text.push(' ');
                wrapped.text.push_str(trimmed);
                wrapped.last = number;
                if trimmed.starts_with(")]") {
                    found.extend(wrapped.markers(extension));
                } else {
                    open = Some(wrapped);
                }
                continue;
            }
            found.extend(wrapped.markers(extension));
        }
        if CFG_ATTR.contains(&trimmed) {
            open = Some(Wrapped {
                first: number,
                last: number,
                text: trimmed.to_owned(),
            });
        }
    }
    if let Some(wrapped) = open {
        found.extend(wrapped.markers(extension));
    }
    found
}

/// Whether `c` may be part of a name in Rust, or joins a name to a call as `.` does.
fn is_name_char(c: char) -> bool {
    c.is_alphanumeric() || matches!(c, '_' | '.' | ':')
}

/// Whether `before`, the start of a line of Rust, ends inside a string that one of its quotes opened.
fn ends_in_string(before: &str) -> bool {
    let mut inside = false;
    let mut chars = before.chars();
    while let Some(c) = chars.next() {
        match c {
            // An escape in a string takes the character after it with it, a quote too.
            '\\' if inside => {
                chars.next();
            }
            '"' => inside = !inside,
            _ => {}
        }
    }
    inside
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
            Self::Applied => {
                CFG_ATTR.iter().any(|start| line.starts_with(start))
                    && line.match_indices(text).any(|(at, _)| {
                        let starts_word = line
                            .get(..at)
                            .is_some_and(|before| !before.ends_with(is_name_char) && !ends_in_string(before));
                        // A text that ends with its `(` ends a word; another one where no name character follows it.
                        let ends_word = text.ends_with('(')
                            || line
                                .get(at..)
                                .and_then(|from| from.strip_prefix(text))
                                .is_some_and(|rest| !rest.starts_with(is_name_char));
                        starts_word && ends_word
                    })
            }
        }
    }
}

#[cfg(test)]
mod tests;
