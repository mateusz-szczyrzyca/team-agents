//! What `agent status` says about the history when it is not whole: how it is doing and how many records it lost.

use agent_core::history::{HistoryHealth, HistoryState};
use agent_core::protocol::{Frame, Response};

use super::*;
use crate::fixtures::team_state;
use crate::testing::{Answer, FakeDaemon};

fn teams() -> Vec<TeamSummary> {
    vec![TeamSummary::from(&team_state().teams()[0])]
}

/// What `agent status` prints when the daemon answers `reply`.
async fn printed(reply: Reply) -> String {
    let dir = tempfile::tempdir().unwrap();
    let daemon = FakeDaemon::serve(&dir.path().join("agent.sock"), move |request| {
        Answer::Lines(vec![
            Frame::Response(Response::ok(request.id, reply.clone()))
                .to_line()
                .unwrap(),
        ])
    });
    let mut connection = Connection::connect(daemon.socket()).await.unwrap();
    let mut out = Vec::new();

    status(&mut connection, None, &mut out).await.unwrap();

    String::from_utf8(out).unwrap()
}

#[tokio::test]
async fn a_history_that_cannot_be_written_is_said_with_why_and_how_many_records_were_lost() {
    let history = HistoryHealth {
        state: HistoryState::Unavailable {
            reason: "the history database \"/home/you/agents/history.db\" is of version 2, newer than the 1 this build writes".to_owned(),
        },
        lost: 12,
    };

    let text = printed(Reply::Overview {
        teams: teams(),
        history,
    })
    .await;

    assert_eq!(
        text,
        format!(
            "{}history: not written: the history database \"/home/you/agents/history.db\" is of version 2, newer than the 1 this build writes; 12 records lost\n",
            render(&teams())
        )
    );
}

#[tokio::test]
async fn a_history_that_lost_records_but_is_written_says_how_many() {
    let history = HistoryHealth {
        state: HistoryState::Writing,
        lost: 1,
    };

    let text = printed(Reply::Overview {
        teams: teams(),
        history,
    })
    .await;

    assert_eq!(text, format!("{}history: written; 1 record lost\n", render(&teams())));
}
