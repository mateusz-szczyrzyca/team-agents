//! The reminder for a run with Serena that searches code without it.
//!
//! A prompt asks an agent to find code with Serena; it does not make it. A run that has Serena and goes on with the shell
//! and its CLI's own tools is told once, while it goes on, that Serena finds symbols and that these tools stay for text.
//! This module holds what that reminder needs: the rule of which step is a search of code without Serena. It is pure: the
//! run engine feeds the steps in and asks.

use std::collections::BTreeSet;
use std::path::Path;

use crate::cli::ActivityEvent;

mod prompt;
mod shell;
pub use prompt::{ReminderText, ReminderTextError};
mod tally;
#[cfg(test)]
mod tests;
pub use tally::{Counted, DEFAULT_REMIND_AFTER, RemindAfter, RemindAfterError, Reminder, SessionTally, is_serena_tool};

/// The extensions, without the dot and in lower case, of the files of code: the languages Serena's language servers read —
/// Rust, Go, Python, TypeScript and JavaScript, Java, C and C++ with their headers, C#, Ruby, PHP, Kotlin and Swift — and
/// the shell, SQL, Elixir, Erlang and Lua that a repository of these agents holds too. Markdown, TOML, YAML and a log are
/// text: they are what `grep` is for.
const CODE_EXTENSIONS: [&str; 33] = [
    "rs", "go", "py", "pyi", "ts", "tsx", "js", "jsx", "mjs", "cjs", "java", "c", "h", "cc", "cpp", "cxx", "hpp", "hh",
    "cs", "rb", "php", "kt", "kts", "swift", "sh", "bash", "zsh", "sql", "ex", "exs", "erl", "lua", "pl",
];

/// The extensions, without the dot and in lower case, of the files of text a search is told apart by: what `grep` is for,
/// and what a filter (`--glob '*.md'`, `--type md`) limits a search to.
const TEXT_EXTENSIONS: [&str; 14] = [
    "md", "markdown", "toml", "yaml", "yml", "json", "txt", "log", "diff", "patch", "lock", "out", "csv", "html",
];

/// The names of the directories that hold the code of a repository of these agents. A search over a directory with one of
/// these names in its path, or over `.`, searches code; a search over `docs/` or `config` searches text.
const CODE_DIRECTORIES: [&str; 12] = [
    "src", "crates", "lib", "cmd", "pkg", "internal", "app", "tests", "test", "tools", "bin", "agentbus",
];

/// The mark that ends the summary of a CLI's search (`Grep`, `grep`) that was given a path: `src (path)`. The summary of a
/// search with no path is its pattern, with no mark, and it searches the whole repository.
pub const SEARCH_PATH_MARK: &str = " (path)";

/// What a CLI's own tool does to the files it is given, by the name Claude Code and opencode give it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CliTool {
    /// `Read`, `read`, `Glob`, `glob`: it reads the files it names, or the files a pattern names (`src/**/*.rs`).
    Reads,
    /// `Grep`, `grep`: it searches a file or a directory for a pattern; with no path it searches the whole repository.
    Searches,
}

impl CliTool {
    /// The tool called `name`, if it searches or reads files.
    fn named(name: &str) -> Option<Self> {
        match name {
            "Read" | "read" | "Glob" | "glob" => Some(Self::Reads),
            "Grep" | "grep" => Some(Self::Searches),
            _ => None,
        }
    }

    /// The files and directories of code the call of the tool with the `summary` its stream gives names. A search that was
    /// given a path has a summary that ends with [`SEARCH_PATH_MARK`]: it names that path. A search without the mark is a
    /// pattern, and it searches the repository (`.`).
    fn names(self, summary: &str) -> Vec<String> {
        let path = match (self, summary.strip_suffix(SEARCH_PATH_MARK)) {
            (Self::Searches, None) => {
                return if summary.trim().is_empty() {
                    Vec::new()
                } else {
                    vec![".".to_owned()]
                };
            }
            (Self::Reads | Self::Searches, Some(path)) => path,
            (Self::Reads, None) => summary,
        };
        path.split_whitespace()
            .flat_map(one_of_its_words)
            .filter(|word| is_code_file(word) || (self == Self::Searches && is_code_directory(word)))
            .map(str::to_owned)
            .collect()
    }
}

/// Whether the tool called `name` is a CLI's own search (`Grep`, `grep`): the one whose summary says by
/// [`SEARCH_PATH_MARK`] that it names a path and not a pattern.
#[must_use]
pub fn is_search_tool(name: &str) -> bool {
    CliTool::named(name) == Some(CliTool::Searches)
}

/// The files a step searched or read, when it searched or read code without Serena; `None` for a step that is not
/// one: Serena's own calls, agentbus, a file change, a message, and a search of text only.
#[must_use]
pub fn code_search(event: &ActivityEvent) -> Option<CodeSearch> {
    let names = match event {
        ActivityEvent::Command(command) => shell::command_filenames(command),
        ActivityEvent::ToolCall { name, summary } => {
            CliTool::named(name).map_or_else(Vec::new, |tool| tool.names(summary))
        }
        ActivityEvent::BusCall { .. }
        | ActivityEvent::FileEdit { .. }
        | ActivityEvent::Text(_)
        | ActivityEvent::Error(_)
        | ActivityEvent::Info(_)
        | ActivityEvent::Control { .. } => Vec::new(),
    };
    let paths: Vec<String> = names.into_iter().collect::<BTreeSet<_>>().into_iter().collect();
    (!paths.is_empty()).then_some(CodeSearch { paths })
}

/// The files of code one step searched or read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodeSearch {
    /// The paths it read, as the CLI named them, in order and without a path twice.
    pub paths: Vec<String>,
}

/// The text of `word` as one argument of a command or a file: its quoting and its redirection taken off.
fn one_of_its_words(word: &str) -> Vec<&str> {
    word.split(['"', '\'', '`', '>', '<', ',', ';', '('])
        .filter(|part| !part.is_empty())
        .collect()
}

/// Whether `path` is a directory of code: `.` or a name without extension that has a code directory among its components.
/// A word that starts with `-` is an option, never a path.
fn is_code_directory(path: &str) -> bool {
    if path.starts_with('-') || path.starts_with("..") {
        return false;
    }
    let trimmed = path.trim_end_matches('/');
    if trimmed == "." {
        return true;
    }
    let has_extension = Path::new(trimmed).extension().is_some();
    !has_extension && trimmed.split('/').any(|part| CODE_DIRECTORIES.contains(&part))
}

/// Whether `path` is a file of code, by the extension of its name.
fn is_code_file(path: &str) -> bool {
    Path::new(path)
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| CODE_EXTENSIONS.contains(&extension.to_ascii_lowercase().as_str()))
}

/// Whether `extension` is the extension of a file of text, in any case.
fn is_text_extension(extension: &str) -> bool {
    TEXT_EXTENSIONS.contains(&extension.to_ascii_lowercase().as_str())
}

/// Whether `path` is a file of text, by the extension of its name.
fn has_text_extension(path: &str) -> bool {
    Path::new(path)
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(is_text_extension)
}
