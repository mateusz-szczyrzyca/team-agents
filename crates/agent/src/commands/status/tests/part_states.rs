//! The state of every part, in the full view and in the running view.

use super::*;
use crate::fixtures::{at, team_state};
use agent_core::protocol::{Command, Frame, Reply, Response};
use time::UtcOffset;

/// The sample team with its item 1.1 split into `members` parts, each depending on the parts before it.
fn split_team(members: usize) -> agent_core::state::Team {
    let mut team = team_state().teams()[0].clone();
    let item = "1.1".parse().unwrap();
    team.open_round("round-20261111-153101-q1w2".parse().unwrap(), Some(item), at(1))
        .unwrap();
    team.advance_item(
        item,
        agent_core::state::ItemPhase::InProgress {
            assignee: "claude-1".parse().unwrap(),
            branch: "agents/claude-1/1.1".parse().unwrap(),
        },
    )
    .unwrap();
    let parts: Vec<serde_json::Value> = (0..members)
        .map(|position| {
            let depends: Vec<String> = (0..position)
                .filter_map(agent_core::state::split::PartKey::at_position)
                .map(|key| key.to_string())
                .collect();
            serde_json::json!({
                "title": "A part", "delivers": "An independent guide", "checks": "Read the guide",
                "depends_on": depends,
            })
        })
        .collect();
    team.activate_split(serde_json::from_value(serde_json::json!({"parent":"1.1","parts":parts})).unwrap())
        .unwrap();
    team
}

/// A round of item 1.1, open or closed, bound to the part `part` if any.
fn round(id: &str, part: Option<&str>, life: &str) -> serde_json::Value {
    let mut round = serde_json::json!({
        "id": id, "item": "1.1", "opened_at": at(2),
        "state": if life == "open" {
            serde_json::json!({"status": "open"})
        } else {
            serde_json::json!({"status": "closed", "at": at(3), "how": life})
        },
    });
    match part {
        Some(key) if life == "merged" => {
            round["part"] = serde_json::json!({"status": "merged", "key": key, "pr": crate::fixtures::pr(3)});
        }
        Some(key) => round["part"] = serde_json::json!({"status": "working", "key": key}),
        None => {}
    }
    round
}

/// The sample summary with the rounds `rounds` instead of its own.
fn with_rounds(team: &agent_core::state::Team, rounds: serde_json::Value) -> TeamSummary {
    let mut wire = serde_json::to_value(TeamSummary::from(team)).unwrap();
    wire["rounds"] = rounds;
    serde_json::from_value(wire).unwrap()
}

/// A state of a part and its text.
type PartRow = (String, &'static str);

/// The state of every part of the plan of item 1.1, as `splits::states` decides it.
fn states_of(team: &TeamSummary) -> Vec<PartRow> {
    let states = super::splits::states(team);
    states
        .first()
        .map(|(_, parts)| {
            parts
                .iter()
                .map(|(id, state)| {
                    let text = match state {
                        super::splits::PartState::Done => "done",
                        super::splits::PartState::InProgress => "in progress",
                        super::splits::PartState::Ready => "to do",
                        super::splits::PartState::WaitingFor(_) => "waiting",
                        super::splits::PartState::WaitingForTier { .. } => "waiting for a tier",
                    };
                    (id.to_string(), text)
                })
                .collect()
        })
        .unwrap_or_default()
}

// Every part takes the same state from the full team in the full view and in the running view: a merged part is done,
// and a part whose round was closed without a merge is not.
#[test]
fn a_part_takes_its_state_from_the_full_team() {
    let cases = [
        (
            "merged by its round",
            with_rounds(
                &split_team(2),
                serde_json::json!([
                    round("round-20261111-153102-q1w2", Some("a"), "merged"),
                    round("round-20261111-153103-q1w2", Some("b"), "open"),
                ]),
            ),
            vec![("1.1a".to_owned(), "done"), ("1.1b".to_owned(), "in progress")],
        ),
        (
            "its round closed without a merge",
            with_rounds(
                &split_team(2),
                serde_json::json!([round("round-20261111-153102-q1w2", Some("a"), "abandoned")]),
            ),
            vec![("1.1a".to_owned(), "to do"), ("1.1b".to_owned(), "waiting")],
        ),
        (
            "a dependency not merged",
            with_rounds(
                &split_team(3),
                serde_json::json!([round("round-20261111-153102-q1w2", None, "open")]),
            ),
            vec![
                ("1.1a".to_owned(), "to do"),
                ("1.1b".to_owned(), "waiting"),
                ("1.1c".to_owned(), "waiting"),
            ],
        ),
    ];
    for (case, team, expected) in cases {
        assert_eq!(states_of(&team), expected, "{case}");
    }
}

/// The part lines of a rendered status.
fn part_lines(shown: &str) -> Vec<String> {
    shown
        .lines()
        .filter(|line| line.starts_with("    1.1"))
        .map(std::string::ToString::to_string)
        .collect()
}

// The running view is the full team with only its open round lines: a merged part is done in both, and the item
// whose parts are all merged shows no part line in the running view and all done in the full view.
#[test]
fn the_running_view_prints_the_parts_of_the_full_team() {
    let merged = with_rounds(
        &split_team(2),
        serde_json::json!([
            round("round-20261111-153102-q1w2", Some("a"), "merged"),
            round("round-20261111-153103-q1w2", Some("b"), "merged"),
            {
                "id": "round-20261111-153104-q1w2", "item": "1.2", "opened_at": at(4),
                "state": {"status": "open"},
            },
        ]),
    );

    let full = render(std::slice::from_ref(&merged));
    let running = render_running(std::slice::from_ref(&merged));

    assert_eq!(part_lines(&full), ["    1.1a  done  A part", "    1.1b  done  A part"]);
    assert!(part_lines(&running).is_empty(), "{running}");
    assert!(
        running.lines().all(|line| !line.contains("round-20261111-153102-q1w2")),
        "{running}"
    );
    assert!(
        running.lines().any(|line| line.contains("round-20261111-153104-q1w2")),
        "{running}"
    );
}

/// The part lines of both views of `team`, which must be `expected`.
fn assert_both_views(team: &TeamSummary, expected: &[&str], case: &str) {
    for (view, shown) in [
        ("full", render(std::slice::from_ref(team))),
        ("running", render_running(std::slice::from_ref(team))),
    ] {
        assert_eq!(part_lines(&shown), expected, "{case}: {view} view");
    }
}

// The same part has the same state in both views, whichever of the six situations it is in.
#[test]
fn every_part_has_the_same_state_in_both_views() {
    let cases: [(&str, TeamSummary, &[&str]); 6] = [
        (
            "merged by its round",
            with_rounds(
                &split_team(2),
                serde_json::json!([
                    round("round-20261111-153102-q1w2", Some("a"), "merged"),
                    round("round-20261111-153103-q1w2", Some("b"), "open"),
                ]),
            ),
            &["    1.1a  done  A part", "    1.1b  in progress  A part"],
        ),
        (
            "its round open",
            with_rounds(
                &split_team(2),
                serde_json::json!([round("round-20261111-153102-q1w2", Some("a"), "open")]),
            ),
            &["    1.1a  in progress  A part", "    1.1b  waiting for 1.1a  A part"],
        ),
        (
            "its round closed without a merge",
            with_rounds(
                &split_team(2),
                serde_json::json!([round("round-20261111-153102-q1w2", Some("a"), "abandoned")]),
            ),
            &["    1.1a  to do  A part", "    1.1b  waiting for 1.1a  A part"],
        ),
        (
            "nothing started and no dependency, after another part merged",
            with_rounds(
                &split_team(2),
                serde_json::json!([round("round-20261111-153102-q1w2", None, "open")]),
            ),
            &["    1.1a  to do  A part", "    1.1b  waiting for 1.1a  A part"],
        ),
        (
            "a dependency not merged",
            with_rounds(
                &split_team(3),
                serde_json::json!([round("round-20261111-153102-q1w2", None, "open")]),
            ),
            &[
                "    1.1a  to do  A part",
                "    1.1b  waiting for 1.1a  A part",
                "    1.1c  waiting for 1.1a, 1.1b  A part",
            ],
        ),
        (
            "two dependencies of which one is merged",
            with_rounds(
                &split_team(3),
                serde_json::json!([
                    round("round-20261111-153102-q1w2", Some("a"), "merged"),
                    round("round-20261111-153103-q1w2", Some("b"), "open"),
                ]),
            ),
            &[
                "    1.1a  done  A part",
                "    1.1b  in progress  A part",
                "    1.1c  waiting for 1.1b  A part",
            ],
        ),
    ];
    for (case, team, expected) in cases {
        assert_both_views(&team, expected, case);
    }
}

// `agent status --running` is the command that used to drop the merged rounds before it printed the parts: through the
// daemon's own reply, a merged part is `done` and the part after it is in progress.
/// The activity of a home in `dir` that has none.
fn no_activity(dir: &std::path::Path) -> Activity {
    let home = Home::new(
        &dir.to_str()
            .unwrap()
            .parse::<agent_core::config::AbsolutePath>()
            .unwrap(),
    );
    Activity::new(home, Zone::Fixed(UtcOffset::UTC))
}

/// Serves one connection of a daemon that speaks every protocol: it answers a status request with `summary` and the
/// request to show a round with the detail of the round of `team`.
async fn serve_one(listener: tokio::net::UnixListener, summary: TeamSummary, team: agent_core::state::Team) {
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt};
    let (stream, _) = listener.accept().await.unwrap();
    let (read, mut write) = stream.into_split();
    let mut reader = tokio::io::BufReader::new(read);
    let mut line = String::new();
    while reader.read_line(&mut line).await.unwrap() != 0 {
        let (request, format) = agent_core::protocol::Request::from_line_negotiated(&line).unwrap();
        let reply = match &request.command {
            Command::Status(_) => Reply::Teams {
                teams: vec![summary.clone()],
            },
            Command::Show(show) => {
                let agent_core::protocol::Target::Round(id) = &show.target else {
                    panic!("only rounds are shown: {show:?}");
                };
                Reply::Round {
                    round: Box::new(agent_core::protocol::RoundDetail::new(
                        &team,
                        team.round(id).unwrap(),
                        Vec::new(),
                    )),
                }
            }
            other => panic!("the status asks for teams and rounds only: {other:?}"),
        };
        let answer = Frame::Response(Response::ok(request.id, reply))
            .to_line_for(format)
            .unwrap();
        write.write_all(answer.as_bytes()).await.unwrap();
        line.clear();
    }
}

// `agent status --running` is the command that used to drop the merged rounds before it printed the parts: through the
// daemon's own reply, a merged part is `done` and the part after it is in progress.
#[tokio::test]
async fn the_running_command_prints_a_merged_part_as_done() {
    let dir = tempfile::tempdir().unwrap();
    // The round that the sample team opened is the open one, bound to part b here; its detail is the team's own.
    let team = split_team(2);
    let summary = with_rounds(
        &team,
        serde_json::json!([
            round("round-20261111-153102-q1w2", Some("a"), "merged"),
            round("round-20261111-153101-q1w2", Some("b"), "open"),
        ]),
    );
    let socket = dir.path().join("agent.sock");
    let listener = tokio::net::UnixListener::bind(&socket).unwrap();
    let serving = tokio::spawn(serve_one(listener, summary, team));
    let mut connection = Connection::connect(&socket).await.unwrap();
    let mut out = Vec::new();

    status_running(&mut connection, &no_activity(dir.path()), &mut out)
        .await
        .unwrap();
    drop(connection);
    serving.await.unwrap();

    let shown = String::from_utf8(out).unwrap();
    assert_eq!(
        part_lines(&shown),
        ["    1.1a  done  A part", "    1.1b  in progress  A part"],
        "{shown}"
    );
}

// An item whose parts all merged is over in the running view whatever else is open: none of its lines is printed, the
// ones that say where a part came from included; the full view still prints them.
#[test]
fn the_running_view_prints_nothing_of_the_parts_of_a_closed_item() {
    let merged = with_rounds(
        &split_team(2),
        serde_json::json!([
            round("round-20261111-153102-q1w2", Some("a"), "merged"),
            round("round-20261111-153103-q1w2", Some("b"), "merged"),
            {
                "id": "round-20261111-153104-q1w2", "item": "1.2", "opened_at": at(4),
                "state": {"status": "open"},
            },
        ]),
    );

    let full = render(std::slice::from_ref(&merged));
    let running = render_running(std::slice::from_ref(&merged));

    assert!(full.lines().any(|line| line.contains("Split 1.1a")), "{full}");
    assert!(running.lines().all(|line| !line.contains("1.1a")), "{running}");
    assert!(running.lines().all(|line| !line.contains("1.1b")), "{running}");
}

// The wait for an agent of a tier is the one of the part that the lead goes on with: the next part says it, a part that
// is ready as well but comes later is only to do.
#[test]
fn the_wait_for_a_tier_is_told_of_the_next_part_only() {
    let plan = serde_json::json!({"parent": "1.1", "parts": [
        {"title": "First", "delivers": "A", "checks": "B", "depends_on": []},
        {"title": "Second", "delivers": "A", "checks": "B", "depends_on": ["a"]},
        {"title": "Third", "delivers": "A", "checks": "B", "depends_on": ["a"]},
    ]});
    let mut wire = serde_json::to_value(TeamSummary::from(&split_team(2))).unwrap();
    wire["splits"] = serde_json::json!([plan]);
    wire["tier_waits"] = serde_json::json!({"1.1": "investigator"});
    wire["rounds"] = serde_json::json!([round("round-20261111-153102-q1w2", Some("a"), "merged")]);
    let summary: TeamSummary = serde_json::from_value(wire).unwrap();

    assert_both_views(
        &summary,
        &[
            "    1.1a  done  First",
            "    1.1b  waiting for investigator agent  Second",
            "    1.1c  to do  Third",
        ],
        "two parts ready after a, a tier wait",
    );
}
