//! The table of the rule: which step is a search of code without Serena, and which is not.

use crate::cli::ActivityEvent;

use super::{CodeSearch, code_search};

/// The cases of one class of steps: the step and the paths the rule tells, or `None` for no search of code.
fn table(cases: Vec<(ActivityEvent, Option<Vec<&str>>)>) {
    for (event, expected) in cases {
        let found = code_search(&event);
        let told: Option<Vec<&str>> = found
            .as_ref()
            .map(|found| found.paths.iter().map(String::as_str).collect());
        assert_eq!(told, expected, "{event:?}");
    }
}

/// A command of the shell, as a CLI's stream names it.
fn command(text: &str) -> ActivityEvent {
    ActivityEvent::Command(text.to_owned())
}

/// A call of the tool `name` of a CLI, with the summary its stream gives.
fn call(name: &str, summary: &str) -> ActivityEvent {
    ActivityEvent::ToolCall {
        name: name.to_owned(),
        summary: summary.to_owned(),
    }
}

#[test]
fn a_shell_command_over_a_file_of_code_is_a_search_of_code() {
    table(vec![
        (command("grep -n foo src/main.rs"), Some(vec!["src/main.rs"])),
        (
            command("rg find_symbol crates/agent-core/src/lib.rs"),
            Some(vec!["crates/agent-core/src/lib.rs"]),
        ),
        (command("sed -n 10,20p src/run.ts"), Some(vec!["src/run.ts"])),
        (command("cat include/app.h"), Some(vec!["include/app.h"])),
        (command("head -40 web/index.jsx"), Some(vec!["web/index.jsx"])),
        (command("tail -n 5 tools/gen.py"), Some(vec!["tools/gen.py"])),
        (command("find crates -name main.go"), Some(vec!["main.go"])),
        (command("grep -rn TODO ./lib/parse.cpp"), Some(vec!["./lib/parse.cpp"])),
    ]);
}

#[test]
fn a_shell_command_over_text_is_no_search_of_code() {
    table(vec![
        (command("grep -n serena README.md"), None),
        (command("rg remind_after config/agents.toml"), None),
        (command("sed -n 1,40p daemon.log"), None),
        (command("cat ChangeLog.diff"), None),
        (command("head -20 notes.txt"), None),
        (command("tail -f run.out"), None),
        (command("find . -name '*.md'"), None),
        (command("grep -rn serena ."), None),
    ]);
}

#[test]
fn a_shell_command_with_a_pipe_reads_the_text_the_pipe_gives_not_a_file() {
    table(vec![
        (command("git log --oneline | head -5"), None),
        (command("grep -rn TODO src | wc -l"), Some(vec!["src"])),
        (command("ps aux | grep serena"), None),
    ]);
}

#[test]
fn a_cli_tool_that_reads_a_file_of_code_is_a_search_of_code() {
    table(vec![
        (
            call("Read", "/work/tree/src/main.rs"),
            Some(vec!["/work/tree/src/main.rs"]),
        ),
        (call("Grep", "fn main src/lib.rs"), Some(vec!["src/lib.rs"])),
        (call("glob", "src/**/*.rs"), None),
        (call("read", "lib/handler.py"), Some(vec!["lib/handler.py"])),
        (call("grep", "src"), None),
    ]);
}

#[test]
fn a_cli_tool_that_reads_text_is_no_search_of_code() {
    table(vec![
        (call("Read", "/work/tree/docs/configuration.md"), None),
        (call("Grep", "serena config/agents.toml"), None),
        (call("read", "daemon.log"), None),
        (call("Read", "/work/tree/README.md"), None),
    ]);
}

#[test]
fn a_call_of_serena_and_every_other_step_is_no_search_of_code() {
    table(vec![
        (call("mcp__serena__find_symbol", "{\"name_path\":\"Foo\"}"), None),
        (call("serena.find_referencing_symbols", "Foo in src/main.rs"), None),
        (
            call(
                "mcp__agentbus__read_messages",
                "{\"to\":\"all\",\"path\":\"src/main.rs\"}",
            ),
            None,
        ),
        (
            ActivityEvent::FileEdit {
                path: "src/main.rs".to_owned(),
            },
            None,
        ),
        (ActivityEvent::Text("grep -n foo src/main.rs".to_owned()), None),
        (ActivityEvent::Error("cannot open src/main.rs".to_owned()), None),
        (ActivityEvent::Info("the run started".to_owned()), None),
    ]);
}

#[test]
fn a_command_that_reads_nothing_and_names_a_file_of_code_is_no_search_of_code() {
    table(vec![
        (command("cargo test -p agent-core serena"), None),
        (command("git add src/main.rs"), None),
        (command("echo src/main.rs"), None),
    ]);
}

#[test]
fn one_step_that_reads_two_files_of_code_tells_both_once() {
    table(vec![
        (
            command("cat src/main.rs src/lib.rs"),
            Some(vec!["src/lib.rs", "src/main.rs"]),
        ),
        (
            command("grep -n foo src/main.rs src/main.rs"),
            Some(vec!["src/main.rs"]),
        ),
        (
            command("grep -n foo src/main.rs; head -3 build.py"),
            Some(vec!["build.py", "src/main.rs"]),
        ),
        (
            command("grep -n foo src/main.rs && cat README.md"),
            Some(vec!["src/main.rs"]),
        ),
    ]);
}

#[test]
fn a_file_of_code_is_told_by_its_extension_in_any_case() {
    table(vec![
        (command("cat SRC/MAIN.RS"), Some(vec!["SRC/MAIN.RS"])),
        (command("cat Program.CS"), Some(vec!["Program.CS"])),
    ]);
}

#[test]
fn an_empty_path_is_no_search() {
    let found = code_search(&command("cat"));
    assert_eq!(found, None);
    assert_eq!(CodeSearch { paths: Vec::new() }.paths, Vec::<String>::new());
}
