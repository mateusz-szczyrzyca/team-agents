//! The table of the reminder: which step of a session counts, and which step is Serena.
//!
//! It uses the counter of a session that this module keeps itself, so that the table is about the counting and not about
//! when the reminder of a session is due, which is the table of [`super::tests`].

use crate::cli::ActivityEvent;

use super::Counted;

/// A command of the shell, as a CLI's stream names it.
fn command(text: &str) -> ActivityEvent {
    ActivityEvent::Command(text.to_owned())
}

/// A call of the tool `name` of a CLI.
fn call(name: &str, summary: &str) -> ActivityEvent {
    ActivityEvent::ToolCall {
        name: name.to_owned(),
        summary: summary.to_owned(),
    }
}

#[test]
fn a_session_counts_the_searches_of_code_its_steps_make() {
    let mut counted = Counted::default();
    let steps = [
        command("grep -n foo src/main.rs"),
        command("cat README.md"),
        call("Read", "/work/tree/lib/handler.py"),
        call("mcp__serena__find_symbol", "{}"),
        command("mcp__serena__find_symbol"),
        call("serena.get_symbols_overview", "{}"),
        call("serena_find_referencing_symbols", "{}"),
        command("rg todo crates/agent-core/src/lib.rs"),
        ActivityEvent::Text("grep -n foo src/main.rs".to_owned()),
    ];

    let told: Vec<Counted> = steps
        .iter()
        .map(|step| {
            counted.count(step);
            counted
        })
        .collect();

    assert_eq!(
        told,
        [
            Counted {
                searches: 1,
                used_serena: false
            },
            Counted {
                searches: 1,
                used_serena: false
            },
            Counted {
                searches: 2,
                used_serena: false
            },
            Counted {
                searches: 2,
                used_serena: true
            },
            Counted {
                searches: 2,
                used_serena: true
            },
            Counted {
                searches: 2,
                used_serena: true
            },
            Counted {
                searches: 2,
                used_serena: true
            },
            Counted {
                searches: 3,
                used_serena: true
            },
            Counted {
                searches: 3,
                used_serena: true
            },
        ]
    );
}

#[test]
fn every_tool_of_serena_counts_whichever_way_the_cli_names_it() {
    let steps = [
        "mcp__serena__find_symbol",
        "Serena.Find_Symbol",
        "agentbus_board_list",
        "mcp__agentbus__read_messages",
        "serena",
        "read",
    ];

    // Each step is counted alone: a counter that has read Serena once in an earlier step is no evidence of what a
    // later step was.
    let told: Vec<Counted> = steps
        .iter()
        .map(|name| {
            let mut step = Counted::default();
            step.count(&call(name, "{}"));
            step
        })
        .collect();

    assert_eq!(
        told,
        [
            Counted {
                searches: 0,
                used_serena: true
            },
            Counted {
                searches: 0,
                used_serena: true
            },
            Counted::default(),
            Counted::default(),
            Counted::default(),
            Counted::default(),
        ]
    );
}

#[test]
fn a_message_an_error_and_a_control_count_nothing() {
    let mut counted = Counted::default();
    let steps = [
        ActivityEvent::Text("src/main.rs".to_owned()),
        ActivityEvent::Error("cannot read src/main.rs".to_owned()),
        ActivityEvent::Info("started".to_owned()),
        ActivityEvent::FileEdit {
            path: "src/main.rs".to_owned(),
        },
        ActivityEvent::BusCall {
            tool: "read_messages".to_owned(),
            arguments: "{}".to_owned(),
        },
    ];

    for step in &steps {
        counted.count(step);
    }

    assert_eq!(counted, Counted::default());
}
