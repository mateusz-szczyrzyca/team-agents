//! Executor metadata follows the existing legacy projection for responses and events.

use super::*;
use crate::protocol::{Event, EventKind, Frame, ReplyFormat};
use crate::state::fixtures::{at, round_id, team};

#[test]
fn older_protocol_frames_omit_executor_metadata() {
    let mut summary = TeamSummary::from(&team(1));
    summary
        .part_assignments
        .insert(round_id(1), "claude-1".parse().unwrap());
    let response = Frame::Response(Response::ok(
        RequestId(1),
        Reply::Teams {
            teams: vec![summary.clone()],
        },
    ));
    let event = Frame::Event(Event::new(
        at(1),
        EventKind::TeamUpdated {
            summary: Box::new(summary),
        },
    ));
    for (frame, pointer) in [
        (response, "/result/reply/teams/0/part_assignments"),
        (event, "/event/summary/part_assignments"),
    ] {
        for format in [ReplyFormat::Legacy, ReplyFormat::Split, ReplyFormat::Plan] {
            let value: serde_json::Value = serde_json::from_str(&frame.to_line_for(format).unwrap()).unwrap();
            let expected = match format {
                ReplyFormat::Legacy | ReplyFormat::Split | ReplyFormat::Plan => None,
            };
            assert_eq!(value.pointer(pointer), expected, "{format:?}");
        }
    }
}
