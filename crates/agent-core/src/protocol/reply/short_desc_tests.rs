//! The words that say what a team's task is, in the summary of a team and on the wire.

use super::*;
use crate::protocol::{Event, EventKind, Frame, ReplyFormat};
use crate::state::fixtures::{LIST, at, team};

/// The sample team with the header line `Short desc: Given words` in its task list.
fn team_with_a_description() -> crate::state::Team {
    let mut value = serde_json::to_value(team(1)).unwrap();
    value["task_list"] = serde_json::json!(LIST.replacen("\n\n", "\nShort desc: Given words\n\n", 1));
    serde_json::from_value(value).unwrap()
}

#[test]
fn the_summary_of_a_team_whose_list_names_no_description_has_the_derived_one() {
    let summary = TeamSummary::from(&team(1));

    assert_eq!(summary.short_desc.as_deref(), Some("Build the thing."));
}

#[test]
fn the_summary_of_a_team_whose_list_names_one_has_it() {
    let summary = TeamSummary::from(&team_with_a_description());

    assert_eq!(summary.short_desc.as_deref(), Some("Given words"));
}

#[test]
fn a_summary_without_the_field_is_read_as_one_without_a_description() {
    let mut value = serde_json::to_value(TeamSummary::from(&team(1))).unwrap();
    value.as_object_mut().unwrap().remove("short_desc");

    let summary: TeamSummary = serde_json::from_value(value).unwrap();

    assert_eq!(summary.short_desc, None);
}

#[test]
fn a_summary_without_a_description_does_not_write_the_field() {
    let mut summary = TeamSummary::from(&team(1));
    summary.short_desc = None;

    let value = serde_json::to_value(&summary).unwrap();

    assert!(value.get("short_desc").is_none(), "{value}");
}

/// A frame with the summary of the sample team, in a response and in an event.
fn frames() -> Vec<(Frame, &'static str)> {
    let summary = TeamSummary::from(&team_with_a_description());
    vec![
        (
            Frame::Response(Response::ok(
                RequestId(1),
                Reply::Teams {
                    teams: vec![summary.clone()],
                },
            )),
            "/result/reply/teams/0",
        ),
        (
            Frame::Event(Event::new(
                at(1),
                EventKind::TeamUpdated {
                    summary: Box::new(summary),
                },
            )),
            "/event/summary",
        ),
    ]
}

#[test]
fn the_current_protocol_carries_the_description_and_the_older_ones_do_not() {
    for (frame, path) in frames() {
        for (format, carried) in [
            (ReplyFormat::Legacy, false),
            (ReplyFormat::Split, false),
            (ReplyFormat::Plan, false),
            (ReplyFormat::Executor, true),
        ] {
            let wire: serde_json::Value = serde_json::from_str(&frame.to_line_for(format).unwrap()).unwrap();
            let summary = wire.pointer(path).unwrap();

            assert_eq!(summary.get("short_desc").is_some(), carried, "{format:?} {path}");
            if carried {
                assert_eq!(summary["short_desc"], "Given words", "{format:?} {path}");
            }
        }
    }
}

#[test]
fn a_frame_with_the_description_round_trips_in_the_current_protocol() {
    for (frame, _) in frames() {
        let line = frame.to_line_for(ReplyFormat::Executor).unwrap();

        assert_eq!(Frame::from_line_negotiated(&line).unwrap(), frame);
    }
}
