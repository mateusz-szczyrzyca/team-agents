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

#[test]
fn every_swap_refusal_converts_to_failure() {
    use crate::protocol::{Failure, FailureKind};

    let invalid_cases = [
        SwapRefusal::UnknownAgent { agent: agent("glm-9") },
        SwapRefusal::Worker { agent: agent("glm-1") },
    ];
    for refusal in invalid_cases {
        let failure = Failure::from(refusal.clone());
        assert_eq!(failure.kind, FailureKind::Invalid);
        assert_eq!(failure.message, refusal.to_string());
    }

    let conflict_cases = [
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
        SwapRefusal::Role {
            task: task(),
            role: Role::Reviewer,
        },
        SwapRefusal::Done { task: task() },
    ];
    for refusal in conflict_cases {
        let failure = Failure::from(refusal.clone());
        assert_eq!(failure.kind, FailureKind::Conflict);
        assert_eq!(failure.message, refusal.to_string());
    }
}

#[test]
fn swap_protocol_version_negotiation_and_round_trip() {
    use crate::protocol::{
        Client, Frame, ProtocolError, Reply, ReplyFormat, Request, RequestId, Response, SWAP_PROTOCOL_VERSION,
        UnsupportedVersion,
    };

    let request = Request::new(
        RequestId(1),
        Client::Cli,
        Command::Swap(SwapRequest {
            task: task(),
            agent: agent("claude-9"),
        }),
    );
    let request_wire = request.to_line_for(ReplyFormat::Swap).unwrap();
    let request_json: serde_json::Value = serde_json::from_str(&request_wire).unwrap();
    assert_eq!(request_json["protocol"], SWAP_PROTOCOL_VERSION);
    assert_eq!(
        Request::from_line_negotiated(&request_wire).unwrap(),
        (request, ReplyFormat::Swap)
    );

    let frame = Frame::Response(Response::ok(RequestId(1), Reply::Applied { tasks: vec![task()] }));
    let frame_wire = frame.to_line_for(ReplyFormat::Swap).unwrap();
    let frame_json: serde_json::Value = serde_json::from_str(&frame_wire).unwrap();
    assert_eq!(frame_json["protocol"], SWAP_PROTOCOL_VERSION);
    assert_eq!(Frame::from_line_negotiated(&frame_wire).unwrap(), frame);

    // An older decoder expecting PROTOCOL_VERSION (3) returns UnsupportedVersion:
    assert!(matches!(
        Request::from_line(&request_wire),
        Err(ProtocolError::UnsupportedVersion(UnsupportedVersion { found })) if found == u64::from(SWAP_PROTOCOL_VERSION)
    ));
}
