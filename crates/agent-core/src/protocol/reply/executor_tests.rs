//! Executor metadata follows the existing legacy projection for responses and events.

use super::*;
use crate::protocol::{Event, EventKind, Frame, ReplyFormat};
use crate::state::fixtures::{at, round_id, team};

#[test]
fn legacy_frames_omit_executor_metadata_while_split_and_plan_frames_retain_it() {
    let mut summary = TeamSummary::from(&team(1));
    summary
        .part_assignments
        .insert(round_id(1), "claude-1".parse().unwrap());
    let metadata = serde_json::to_value(&summary.part_assignments).unwrap();
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
                ReplyFormat::Legacy => None,
                ReplyFormat::Split | ReplyFormat::Plan => Some(&metadata),
            };
            assert_eq!(value.pointer(pointer), expected, "{format:?}");
        }
    }
}
