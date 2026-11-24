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
pub use prompt::{ReminderText, ReminderTextError};
mod tally;
#[cfg(test)]
mod tests;
pub use tally::{Counted, DEFAULT_REMIND_AFTER, RemindAfter, RemindAfterError, Reminder, SessionTally};

/// The extensions, without the dot and in lower case, of the files of code: the languages Serena's language servers read —
/// Rust, Go, Python, TypeScript and JavaScript, Java, C and C++ with their headers, C#, Ruby, PHP, Kotlin and Swift — and
/// the shell, SQL, Elixir, Erlang and Lua that a repository of these agents holds too. Markdown, TOML, YAML and a log are
/// text: they are what `grep` is for.
const CODE_EXTENSIONS: [&str; 33] = [
    "rs", "go", "py", "pyi", "ts", "tsx", "js", "jsx", "mjs", "cjs", "java", "c", "h", "cc", "cpp", "cxx", "hpp", "hh",
    "cs", "rb", "php", "kt", "kts", "swift", "sh", "bash", "zsh", "sql", "ex", "exs", "erl", "lua", "pl",
];

/// The names of the CLI's own tools that search a file for a pattern or read it, as Claude Code and opencode name them:
/// `Grep` and `grep`, `Read` and `read`. `Glob` and `glob` find the names of files by a pattern, which names no file.
const SEARCH_TOOLS: [&str; 4] = ["Grep", "Read", "grep", "read"];

/// The names of the shell commands that search or read files.
const SEARCH_COMMANDS: [&str; 7] = ["grep", "rg", "sed", "cat", "head", "tail", "find"];

/// The files a step searched or read, when it searched or read code without Serena; `None` for a step that is not
/// one: Serena's own calls, agentbus, a file change, a message, and a search of text only.
#[must_use]
pub fn code_search(event: &ActivityEvent) -> Option<CodeSearch> {
    let names = match event {
        ActivityEvent::Command(command) => command_filenames(command),
        ActivityEvent::ToolCall { name, summary } if SEARCH_TOOLS.contains(&name.as_str()) => {
            summary.split_whitespace().flat_map(one_of_its_words).collect()
        }
        ActivityEvent::ToolCall { .. }
        | ActivityEvent::BusCall { .. }
        | ActivityEvent::FileEdit { .. }
        | ActivityEvent::Text(_)
        | ActivityEvent::Error(_)
        | ActivityEvent::Info(_)
        | ActivityEvent::Control { .. } => Vec::new(),
    };
    let paths: Vec<String> = names
        .into_iter()
        .filter(|word| is_code_file(word))
        .map(str::to_owned)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    (!paths.is_empty()).then_some(CodeSearch { paths })
}

/// The files of code one step searched or read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodeSearch {
    /// The paths it read, as the CLI named them, in order and without a path twice.
    pub paths: Vec<String>,
}

/// The file names of `command`, as the shell gives them to the commands that search or read files: of every search or
/// read of the line, its own words. A search that reads a pipe (`git log | head`) names no file.
fn command_filenames(command: &str) -> Vec<&str> {
    let mut names = Vec::new();
    let mut reading = false;
    let mut reads_pipe = false;
    let mut after_pipe = false;
    for word in command.split_whitespace() {
        if word == "|" {
            after_pipe = true;
            reading = false;
            continue;
        }
        if word == "&&" || word == ";" {
            continue;
        }
        let (word, ends) = match word.strip_suffix("&&") {
            Some(head) => (head, true),
            None => match word.strip_suffix(';') {
                Some(head) => (head, true),
                None => (word, false),
            },
        };
        let name = word.rsplit_once('/').map_or(word, |(_, last)| last);
        if SEARCH_COMMANDS.contains(&name) || matches!(name, "awk" | "python" | "python3") {
            reading = true;
            reads_pipe = after_pipe;
            after_pipe = false;
        } else if reading && !reads_pipe {
            names.extend(one_of_its_words(word));
        }
        if ends {
            reading = false;
            after_pipe = false;
        }
    }
    names
}

/// The text of `word` as one argument of a command or a file: its quoting and its redirection taken off.
fn one_of_its_words(word: &str) -> Vec<&str> {
    word.split(['"', '\'', '`', '>', '<', ',', ';', '('])
        .filter(|part| !part.is_empty())
        .collect()
}

/// Whether `path` is a file of code, by the extension of its name.
fn is_code_file(path: &str) -> bool {
    Path::new(path)
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| CODE_EXTENSIONS.contains(&extension.to_ascii_lowercase().as_str()))
}
