//! The human renews a task that waits for them: its next run is a new session, briefed with what the human said.

use agent_core::decide::ladder::Trouble;
use agent_core::history::{HistoryEvent, Record};
use agent_core::ids::TaskId;
use agent_core::protocol::{Client, Command, ControlAction, ControlRequest, Reply, SayRequest, Target};
use agent_core::state::{MergeMode, Renewals};
use agent_core::status::TaskStatus;
use agent_runtime::history::Reader;

use super::rig::{ONE, Rig, investigator, normal};
use crate::client::{ClientError, Connection};

/// A first-pass finding followed by acceptance of the committed fix.
const CHANGES: &str = "verify:VERDICT: REQUEST_CHANGES\\nFINDINGS\\n1. [MAJOR] TDD-2 src/lib.rs:7 — the case is untested → add coverage\\n   CHECK: read src/lib.rs:7,verify:VERDICT: APPROVE\\nSUMMARY: fixed";
/// A reviewer with no additional findings.
const APPROVE: &str = "verify:VERDICT: APPROVE\\nSUMMARY: fine";
/// The header every briefing of a new session starts with.
const BRIEFING: &str = "# A NEW SESSION ON A TASK THAT WAS ALREADY STARTED";
/// What the human says to the lead before they renew its task.
const SAID: &str = "use branch alternative-fix";

/// Sends `command` as the command line does and returns the reply.
async fn ask(rig: &Rig, command: Command) -> Result<Reply, ClientError> {
    Connection::connect(&rig.home.socket_file())
        .await
        .unwrap()
        .request(Client::Cli, command)
        .await
}

// Rule: the human renews a solo task that waits after its fix pass was sent back to the limit ("not published"), after
// a word to it; the next run does not resume the old session, its prompt is the briefing with the reason `human` and
// the word, and the history has the `renew` control. No renewal is counted.
#[tokio::test]
async fn a_task_that_waits_for_the_human_is_renewed_and_its_next_run_reads_what_the_human_said() {
    let mut rig = Rig::new(
        &[
            normal("claude-1", "work,fix-done"),
            investigator("inv-r1", CHANGES),
            investigator("inv-r2", APPROVE),
        ],
        5,
        true,
    )
    .await;
    rig.start().await;
    let (team, tasks) = rig.run(&["claude-1"], ONE, MergeMode::Autopilot, None).await;
    let task: TaskId = tasks[0].clone();
    rig.notification_with("FIXED needs a commit").await;
    rig.status_is(&task, TaskStatus::Waiting).await;
    let runs_before = rig.runs(&task);
    let target = Target::Task(task.clone());
    let said = ask(
        &rig,
        Command::Say(SayRequest {
            target: target.clone(),
            text: SAID.to_owned().try_into().unwrap(),
        }),
    )
    .await
    .unwrap();
    assert_eq!(
        said,
        Reply::Applied {
            tasks: vec![task.clone()]
        }
    );

    let renewed = ask(&rig, Command::Renew(ControlRequest { target: target.clone() })).await;

    assert!(
        matches!(&renewed, Ok(Reply::Applied { tasks: reached }) if *reached == [task.clone()]),
        "{renewed:?}"
    );
    let next = runs_before + 1;
    let briefing = rig.prompt(&task, next).await;
    let args = std::fs::read_to_string(rig.worktree(&task).join(format!(".fake-args-{next}"))).unwrap();
    rig.stop().await.unwrap();
    assert!(briefing.starts_with(BRIEFING), "{briefing}");
    assert!(
        briefing.contains("no run went bad: the human started this new session with `agent renew`"),
        "{briefing}"
    );
    assert!(briefing.contains(SAID), "{briefing}");
    assert!(!args.contains("--resume"), "the old session was resumed: {args}");
    let state = rig.state();
    let member = state.find_task(&task).unwrap().member;
    assert_eq!(member.renewals().map(Renewals::reason), Some(Trouble::Human));
    assert_eq!(member.renewals().map(Renewals::used), Some(0), "not counted");
    let round = state.team(&team).unwrap().rounds()[0].id().clone();
    let stored = Reader::open(&rig.home.history_file())
        .unwrap()
        .round_records(&round)
        .unwrap();
    let renews: Vec<_> = stored
        .into_iter()
        .filter_map(|row| match row.record {
            Record::Event(event) => match event.event {
                HistoryEvent::Controlled {
                    action: ControlAction::Renew,
                    target,
                    by,
                    tasks,
                } => Some((target, by, tasks)),
                _ => None,
            },
            _ => None,
        })
        .collect();
    assert_eq!(renews, [(target, Client::Cli, vec![task])]);
}
