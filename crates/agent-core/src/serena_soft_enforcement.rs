//! Soft enforcement to remind agents to use Serena for code searches.
//!
//! This module implements the logic to detect when an agent searches code without Serena
//! too many times in a session and track reminder state. The decision to remind is pure
//! and deterministic, taking no I/O.

/// A tool call that might search code without Serena.
///
/// This is the minimal information needed to decide if a call counts toward the reminder threshold.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ToolCall {
    /// The name of the CLI that made the call (e.g., "opencode", "claude-code").
    pub cli: &'static str,
    /// The name of the tool within the CLI (e.g., "grep", "Grep", "execute").
    pub tool: &'static str,
}

impl ToolCall {
    /// Construct a tool call.
    pub const fn new(cli: &'static str, tool: &'static str) -> Self {
        Self { cli, tool }
    }
}

/// File extensions that identify code files.
///
/// These extensions are recognized as code by Serena.  Text files like Markdown,
/// TOML, JSON and logs do not count as code for the reminder threshold.
#[derive(Debug, Clone, Copy)]
pub struct CodeExtensions;

impl CodeExtensions {
    /// Whether a file extension indicates a code file.
    ///
    /// This checks for source code files in programming languages, but excludes
    /// configuration files (TOML, YAML, JSON, XML) and documentation (Markdown, text)
    /// as those are not typically where Serena's symbol search is most useful.
    #[must_use]
    pub fn is_code_file(ext: &str) -> bool {
        // Extensions recognized by Serena as code-containing languages.
        // This list covers the most common languages Serena supports.
        // Notably excluded: toml, yaml, yml, xml, json, md, txt, log
        matches!(
            ext.to_lowercase().as_str(),
            "rs" | "go" | "py" | "js" | "ts" | "jsx" | "tsx" | "java" | "c" | "cpp" | "h"
                | "hpp" | "cc" | "cxx" | "cs" | "kt" | "scala" | "rb" | "php" | "swift"
                | "m" | "mm" | "lua" | "pl" | "sh" | "bash" | "zsh" | "fish" | "ps1"
                | "gradle" | "maven" | "cmake" | "makefile" | "make"
                | "html" | "css" | "scss" | "less"
                | "sql" | "vb" | "f90" | "f95" | "f03" | "f08"
        )
    }
}

/// Whether a shell command searches code without using Serena.
///
/// Shell commands that search code (`grep`, `rg`, `sed`, `cat`, `head`, `tail`, `find`)
/// applied to code files count as code searches.
#[must_use]
pub fn is_code_search_shell_command(command: &str, file_path: Option<&str>) -> bool {
    let cmd_name = command.split_whitespace().next().unwrap_or("");

    // Only these commands are considered code searches.
    let is_search_cmd = matches!(cmd_name, "grep" | "rg" | "sed" | "cat" | "head" | "tail" | "find");

    if !is_search_cmd {
        return false;
    }

    // If we know the file path, check if it's a code file.
    if let Some(path) = file_path {
        let ext = path.split('.').last().unwrap_or("");
        return CodeExtensions::is_code_file(ext);
    }

    // Without a file path, conservatively assume it might be code.
    true
}

/// Whether a CLI tool call searches code.
///
/// Certain tools of CLIs that provide code-finding services count as searching code.
/// These are the tools that search by symbol or by grep on code files.
#[must_use]
pub fn is_code_search_cli_tool(cli: &str, tool: &str) -> bool {
    match (cli, tool) {
        // OpenCode provides `grep`, `glob`, `read` on any file; these count if used on code.
        // Since we don't know the file path at this layer, we treat them conservatively.
        ("opencode", name) => matches!(name, "grep" | "glob" | "read"),
        // Claude Code (built-in browser) provides similar tools.
        ("claude-code", name) => matches!(name, "Grep" | "Glob" | "Read"),
        // Codex and other CLIs: add their patterns here.
        _ => false,
    }
}

/// The state of Serena reminder tracking in a session.
///
/// A session is a contiguous sequence of runs of a task. The reminder is sent at most
/// once per session, when the count of code searches without Serena reaches the threshold.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReminderState {
    /// How many code searches without Serena have been counted in this session.
    code_searches_without_serena: usize,
    /// Whether Serena has been called at least once in this session.
    serena_called: bool,
    /// Whether a reminder has been sent in this session.
    reminder_sent: bool,
}

impl ReminderState {
    /// Create a new reminder state at the start of a session.
    pub const fn new() -> Self {
        Self {
            code_searches_without_serena: 0,
            serena_called: false,
            reminder_sent: false,
        }
    }

    /// Record that a code search without Serena was made.
    pub fn record_code_search_without_serena(&mut self) {
        if !self.serena_called {
            self.code_searches_without_serena = self.code_searches_without_serena.saturating_add(1);
        }
    }

    /// Record that Serena was called.
    ///
    /// Once Serena is called, no more reminders will be sent in this session,
    /// and the code search counter resets.
    pub fn record_serena_call(&mut self) {
        self.serena_called = true;
        self.code_searches_without_serena = 0;
    }

    /// Record that a reminder was sent.
    pub fn record_reminder_sent(&mut self) {
        self.reminder_sent = true;
    }

    /// Whether a reminder should be sent now.
    ///
    /// A reminder is sent when:
    /// - The code search count reaches the threshold,
    /// - Serena is available (has been mentioned to the run),
    /// - Serena has not been called yet,
    /// - No reminder has been sent yet in this session.
    #[must_use]
    pub fn should_remind(&self, threshold: usize) -> bool {
        !self.serena_called
            && !self.reminder_sent
            && threshold > 0
            && self.code_searches_without_serena >= threshold
    }

    /// Whether a reminder has been sent in this session.
    #[must_use]
    pub fn reminder_sent(&self) -> bool {
        self.reminder_sent
    }

    /// How many code searches without Serena have been recorded.
    #[must_use]
    pub fn code_search_count(&self) -> usize {
        self.code_searches_without_serena
    }

    /// Whether Serena has been called in this session.
    #[must_use]
    pub fn serena_was_called(&self) -> bool {
        self.serena_called
    }
}

impl Default for ReminderState {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn code_extensions_recognizes_rust() {
        assert!(CodeExtensions::is_code_file("rs"));
        assert!(CodeExtensions::is_code_file("RS"));
        assert!(CodeExtensions::is_code_file("Rs"));
    }

    #[test]
    fn code_extensions_recognizes_many_languages() {
        assert!(CodeExtensions::is_code_file("go"));
        assert!(CodeExtensions::is_code_file("py"));
        assert!(CodeExtensions::is_code_file("js"));
        assert!(CodeExtensions::is_code_file("ts"));
        assert!(CodeExtensions::is_code_file("java"));
    }

    #[test]
    fn code_extensions_rejects_text_files() {
        assert!(!CodeExtensions::is_code_file("md"));
        assert!(!CodeExtensions::is_code_file("markdown"));
        assert!(!CodeExtensions::is_code_file("txt"));
        assert!(!CodeExtensions::is_code_file("log"));
    }

    #[test]
    fn is_code_search_shell_command_detects_grep_on_code() {
        assert!(is_code_search_shell_command("grep -r foo", Some("file.rs")));
        assert!(is_code_search_shell_command("grep foo", Some("main.py")));
    }

    #[test]
    fn is_code_search_shell_command_ignores_non_code_files() {
        assert!(!is_code_search_shell_command("grep foo", Some("README.md")));
        assert!(!is_code_search_shell_command("grep foo", Some("config.toml")));
    }

    #[test]
    fn is_code_search_shell_command_detects_rg_conservatively() {
        // Without a file path, we assume it might be code.
        assert!(is_code_search_shell_command("rg pattern", None));
    }

    #[test]
    fn is_code_search_shell_command_ignores_non_search_commands() {
        assert!(!is_code_search_shell_command("echo hello", None));
        assert!(!is_code_search_shell_command("ls -la", None));
    }

    #[test]
    fn is_code_search_cli_tool_detects_opencode_grep() {
        assert!(is_code_search_cli_tool("opencode", "grep"));
        assert!(is_code_search_cli_tool("opencode", "glob"));
        assert!(is_code_search_cli_tool("opencode", "read"));
    }

    #[test]
    fn is_code_search_cli_tool_detects_claude_code_tools() {
        assert!(is_code_search_cli_tool("claude-code", "Grep"));
        assert!(is_code_search_cli_tool("claude-code", "Glob"));
        assert!(is_code_search_cli_tool("claude-code", "Read"));
    }

    #[test]
    fn is_code_search_cli_tool_ignores_unknown_tools() {
        assert!(!is_code_search_cli_tool("opencode", "unknown_tool"));
        assert!(!is_code_search_cli_tool("unknown-cli", "grep"));
    }

    #[test]
    fn reminder_state_starts_empty() {
        let state = ReminderState::new();
        assert_eq!(state.code_search_count(), 0);
        assert!(!state.serena_was_called());
        assert!(!state.reminder_sent());
    }

    #[test]
    fn reminder_state_counts_code_searches() {
        let mut state = ReminderState::new();
        state.record_code_search_without_serena();
        state.record_code_search_without_serena();
        assert_eq!(state.code_search_count(), 2);
    }

    #[test]
    fn reminder_state_stops_counting_after_serena_call() {
        let mut state = ReminderState::new();
        state.record_code_search_without_serena();
        state.record_code_search_without_serena();
        state.record_serena_call();
        state.record_code_search_without_serena();
        assert_eq!(state.code_search_count(), 0); // Reset after Serena call
        assert!(state.serena_was_called());
    }

    #[test]
    fn reminder_state_should_remind_at_threshold() {
        let mut state = ReminderState::new();
        state.record_code_search_without_serena();
        state.record_code_search_without_serena();
        state.record_code_search_without_serena();
        assert!(!state.should_remind(10)); // Below threshold
        assert!(state.should_remind(3)); // At threshold
        assert!(state.should_remind(2)); // Above threshold
    }

    #[test]
    fn reminder_state_no_remind_after_serena_called() {
        let mut state = ReminderState::new();
        state.record_code_search_without_serena();
        state.record_serena_call();
        assert!(!state.should_remind(1)); // Serena was called
    }

    #[test]
    fn reminder_state_no_remind_twice() {
        let mut state = ReminderState::new();
        state.record_code_search_without_serena();
        state.record_code_search_without_serena();
        state.record_reminder_sent();
        assert!(!state.should_remind(2)); // Already reminded
    }

    #[test]
    fn reminder_state_no_remind_at_zero_threshold() {
        let mut state = ReminderState::new();
        state.record_code_search_without_serena();
        assert!(!state.should_remind(0)); // Threshold disabled
    }
}
