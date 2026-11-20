//! Split acknowledgements are shown with the ordinary answer confirmation.

use super::*;
use crate::fixtures::task;
use crate::testing::{Answer, FakeDaemon};

#[tokio::test]
async fn split_answer_reports_recorded_parts_and_rejection_reasons() {
    for acknowledgement in [
        "Recorded split parts: 1.1a — Żółw; 1.1b — Guide.",
        "No split recorded: use the explicit split verdict form.",
    ] {
        let dir = tempfile::tempdir().unwrap();
        let reply = acknowledgement.to_owned();
        let daemon = FakeDaemon::serve(&dir.path().join("agent.sock"), move |request| {
            let mut frame = serde_json::to_value(agent_core::protocol::Frame::Response(
                agent_core::protocol::Response::ok(request.id, Reply::Applied { tasks: vec![task(1)] }),
            ))
            .unwrap();
            frame["split"] = serde_json::json!(reply);
            Answer::Lines(vec![format!("{}\n", serde_json::to_string(&frame).unwrap())])
        });
        let mut connection = Connection::connect(daemon.socket()).await.unwrap();
        let open = WaitingQuestions {
            number: NonZeroU32::MIN,
            file: dir.path().join("questions.md"),
        };
        let delivery = Delivery {
            task: task(1),
            open: &open,
            origin: Origin::File,
        };
        let list = AnswerList::try_from(vec![AnswerInput {
            question: NonZeroU32::MIN,
            verdict: agent_core::questions::Verdict::from_text("ok").unwrap().unwrap(),
        }])
        .unwrap();
        let mut out = Vec::new();
        let result = send(&mut connection, &delivery, list, &mut out).await;
        assert_eq!(
            String::from_utf8(out).unwrap(),
            format!(
                "Answered questions number 1 of {}: it goes on with your answers.\n{acknowledgement}\n",
                task(1)
            )
        );
        result.unwrap();
    }
}
