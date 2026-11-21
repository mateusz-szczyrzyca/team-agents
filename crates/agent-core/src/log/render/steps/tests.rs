//! The line of a step: each kind of step, for each CLI's recorded form, with one and with two agents followed.

use std::num::NonZeroU32;

use crate::cli::ActivityEvent;
use crate::config::AbsolutePath;
use crate::history::{AgentIdentity, RunStart, RunWhy, Step};
use crate::ids::{AgentName, RunId, TaskId, TeamId};
use crate::kinds::{CliKind, Role, Tier};
use crate::local_time::Zone;
use crate::log::render::steps::step_line;
use crate::log::render::steps::step_line_in;
use crate::protocol::{Client, ControlAction};
use time::UtcOffset;

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

#[test]
fn the_end_of_a_call_is_told_as_each_cli_records_it_with_one_agent_and_with_two() {
    // What the adapters give today: Codex turns a command that exited with a code other than 0 into an error naming the
    // code and the command; Claude Code, OpenCode and Antigravity give the text of the error of a failed call. No CLI
    // gives a step for an exit code 0: the result of a call that went well is not recorded as a step.
    let rows: Vec<(&str, CliKind, ActivityEvent, &str)> = vec![
        (
            "codex-1",
            CliKind::Codex,
            ActivityEvent::Error("exit 101: /usr/bin/bash -lc 'cargo test'".to_owned()),
            "error: exit 101: /usr/bin/bash -lc 'cargo test'",
        ),
        (
            "inv-claude-sonnet",
            CliKind::Claude,
            ActivityEvent::Error("Exit code 1\nfatal: not a git repository".to_owned()),
            "error: Exit code 1…",
        ),
        (
            "opencode-1",
            CliKind::Opencode,
            ActivityEvent::Error("Invalid arguments for tool \"shell\":\n- command: Missing key".to_owned()),
            "error: Invalid arguments for tool \"shell\":…",
        ),
        (
            "gemini-1",
            CliKind::Gemini,
            ActivityEvent::Error("tool call failed: permission denied".to_owned()),
            "error: tool call failed: permission denied",
        ),
    ];
    for (name, cli, event, expected) in &rows {
        let run = start(name, *cli, "task-20261121-132909-8cgk");
        let alone = step_line(&step(event.clone(), 3), Some(&run), &[agent(name)]);
        let beside = step_line(&step(event.clone(), 3), Some(&run), &[agent(name), agent("other-1")]);
        assert_eq!(alone, format!("{AT} {expected}"));
        assert_eq!(beside, format!("{AT} {name} {expected}"));
    }
}

#[test]
fn step_line_in_writes_the_time_of_the_step_in_the_zone_given() {
    let zone = Zone::Fixed(UtcOffset::from_hms(-5, 0, 0).expect("a literal offset is valid"));
    let line = step_line_in(
        &step(ActivityEvent::Command("cargo test".to_owned()), 1),
        Some(&start(
            "inv-claude-sonnet",
            CliKind::Claude,
            "task-20261121-132909-ummt",
        )),
        &[],
        &zone,
    );
    assert_eq!(line, "2026-11-21 07:50:32 -05:00 command: cargo test");
}
