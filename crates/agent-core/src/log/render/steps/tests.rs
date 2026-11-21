//! The line of a step: each kind of step, for each CLI's recorded form, with one and with two agents followed.

use std::num::NonZeroU32;

use crate::cli::ActivityEvent;
use crate::config::AbsolutePath;
use crate::history::{AgentIdentity, RunStart, RunWhy, Step};
use crate::ids::{AgentName, RunId, TaskId, TeamId};
use crate::kinds::{CliKind, Role, Tier};
use crate::log::render::steps::step_line;
use crate::protocol::{Client, ControlAction};

/// The time of every step of the table.
const AT: &str = "2026-11-21T12:50:32Z";

/// The start of the run of `agent`, on `cli` and `task`.
fn start(agent: &str, cli: CliKind, task: &str) -> RunStart {
    RunStart {
        run: "run-20261121-132909-2fqp"
            .parse::<RunId>()
            .expect("a literal id is valid"),
        team: "team-20261121-132909-4qem"
            .parse::<TeamId>()
            .expect("a literal id is valid"),
        task: task.parse::<TaskId>().expect("a literal id is valid"),
        at: "2026-11-21T12:50:00Z".parse().expect("a literal time is valid"),
        agent: AgentIdentity {
            name: agent.to_owned(),
            tier: Tier::Normal,
            cli,
            model: None,
        },
        role: Role::Solo,
        rounds: Vec::new(),
        why: RunWhy::FirstRun,
        prompt: String::new(),
        worktree: "/home/example/worktrees/example/task"
            .parse::<AbsolutePath>()
            .expect("a literal path is valid"),
        branch: "agents/example/task".parse().expect("a literal branch is valid"),
        session: None,
    }
}

/// The `number`th step of a run, with `event`.
fn step(event: ActivityEvent, number: u32) -> Step {
    Step {
        run: "run-20261121-132909-2fqp".parse().expect("a literal id is valid"),
        number: NonZeroU32::new(number).expect("a step number starts at one"),
        at: AT.parse().expect("a literal time is valid"),
        event,
    }
}

/// The name of an agent.
fn agent(name: &str) -> AgentName {
    name.parse().expect("a literal name is valid")
}

#[test]
fn step_line_names_its_agent_beside_another_and_tells_each_kind_of_step_for_each_cli() {
    let cut = format!("{}…", "y".repeat(120));
    let multibyte = format!("{}…", "ż".repeat(120));
    let rows: Vec<(&str, ActivityEvent, String)> = vec![
        (
            "inv-claude-sonnet",
            ActivityEvent::Text("I read the log command.\nThen I wrote the line.".to_owned()),
            "inv-claude-sonnet I read the log command.…".to_owned(),
        ),
        (
            "inv-claude-sonnet",
            ActivityEvent::Command("cargo test -p agent log".to_owned()),
            "inv-claude-sonnet command: cargo test -p agent log".to_owned(),
        ),
        (
            "codex-1",
            ActivityEvent::Command("y".repeat(300)),
            format!("codex-1 command: {cut}"),
        ),
        (
            "agy-1",
            ActivityEvent::FileEdit {
                path: "crates/agent/src/commands/log/rows.rs".to_owned(),
            },
            "agy-1 file edit: crates/agent/src/commands/log/rows.rs".to_owned(),
        ),
        (
            "oc-1",
            ActivityEvent::ToolCall {
                name: "read_file".to_owned(),
                summary: "crates/agent/src/commands/log/rows.rs".to_owned(),
            },
            "oc-1 tool call: read_file crates/agent/src/commands/log/rows.rs".to_owned(),
        ),
        (
            "inv-claude-sonnet",
            ActivityEvent::BusCall {
                tool: "send_message".to_owned(),
                arguments: "to=inv-claude-sonnet wake=false".to_owned(),
            },
            "inv-claude-sonnet bus call: send_message to=inv-claude-sonnet wake=false".to_owned(),
        ),
        (
            "codex-1",
            ActivityEvent::Error("error: no such crate".to_owned()),
            "codex-1 error: error: no such crate".to_owned(),
        ),
        (
            "inv-claude-sonnet",
            ActivityEvent::Info("model claude-sonnet-5".to_owned()),
            "inv-claude-sonnet info: model claude-sonnet-5".to_owned(),
        ),
        (
            "agy-1",
            ActivityEvent::Control {
                action: ControlAction::Pause,
                by: Client::Cli,
            },
            "agy-1 control: pause from the terminal".to_owned(),
        ),
        (
            "codex-1",
            ActivityEvent::Text("ż".repeat(200)),
            format!("codex-1 {multibyte}"),
        ),
    ];
    let mut number = 0_u32;
    for cli in [CliKind::Claude, CliKind::Codex, CliKind::Gemini, CliKind::Opencode] {
        for (who, event, rest) in &rows {
            number = number.checked_add(1).expect("the table is short");
            let run = start(who, cli, "task-20261121-132909-ummt");
            let line = step_line(
                &step(event.clone(), number),
                Some(&run),
                &[agent(who), agent("codex-1")],
            );
            assert_eq!(line, format!("{AT} {rest}"), "{cli:?} {event:?}");
        }
    }
}
