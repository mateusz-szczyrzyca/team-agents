//! Tests for the swap command and swap refusal protocol types.

use crate::ids::{AgentName, TaskId, TeamId};
use crate::kinds::Role;
use crate::protocol::{Command, SwapRefusal, SwapRequest};

fn task() -> TaskId {
    "task-20261120-144101-r7q2".parse().unwrap()
}

fn agent(name: &str) -> AgentName {
    name.parse().unwrap()
}

fn team() -> TeamId {
    "team-20261120-144101-r7q2".parse().unwrap()
}

#[test]
fn swap_request_round_trips() {
    let request = SwapRequest {
        task: task(),
        agent: agent("claude-9"),
    };

    let json = serde_json::to_string(&request).expect("serializes to json");
    let back: SwapRequest = serde_json::from_str(&json).expect("deserializes from json");
    assert_eq!(request, back);

    let command = Command::Swap(request);
    let cmd_json = serde_json::to_string(&command).expect("command serializes to json");
    assert!(cmd_json.contains("\"command\":\"swap\""));
    let cmd_back: Command = serde_json::from_str(&cmd_json).expect("command deserializes from json");
    assert_eq!(command, cmd_back);
}

#[test]
fn every_swap_refusal_round_trips() {
    let refusals = [
        SwapRefusal::UnknownAgent { agent: agent("glm-9") },
        SwapRefusal::SameHolder {
            task: task(),
            agent: agent("claude-1"),
        },
        SwapRefusal::InTeam {
            agent: agent("claude-2"),
            team: team(),
            task: task(),
        },
        SwapRefusal::InRun {
            agent: agent("claude-2"),
            task: task(),
        },
        SwapRefusal::Worker { agent: agent("glm-1") },
        SwapRefusal::Role {
            task: task(),
            role: Role::Reviewer,
        },
        SwapRefusal::Done { task: task() },
    ];

    for refusal in &refusals {
        let json = serde_json::to_string(refusal).expect("refusal serializes to json");
        assert!(json.contains("\"refusal\":"), "expected refusal tag in {json}");
        let back: SwapRefusal = serde_json::from_str(&json).expect("refusal deserializes from json");
        assert_eq!(refusal, &back);
    }
}
