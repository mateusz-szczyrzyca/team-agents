//! After the last step of the ladder — a task handed over whose new holder goes bad too, or a lead given a member's work
//! that goes bad too — the human is asked once what becomes of the task, and each answer does what it says.

use std::num::NonZeroU32;

use agent_core::ids::TaskId;
use agent_core::protocol::{AnswerInput, AnswerList, AnswerRequest, Client, Command, Reply};
use agent_core::questions::Verdict;
use agent_core::state::{ItemPhase, MergeMode, Renewals};
use agent_core::status::TaskStatus;

use super::rig::{ONE, Rig, investigator, normal};

/// A reviewer that approves.
const APPROVE: &str = "verify:VERDICT: APPROVE\\nSUMMARY: fine";

/// Six runs of claude-1 without a status: both its sessions.
const BAD_IN_BOTH_SESSIONS: &str = "nostatus";

/// Adds `table` to the configuration of `rig`, before the daemon starts.
fn configure(rig: &Rig, table: &str) {
    let mut config = std::fs::read_to_string(rig.home.config_file()).unwrap();
    config.push_str(table);
    std::fs::write(rig.home.config_file(), config).unwrap();
}

/// The questions file number `number` of `task`, if it was written.
fn questions(rig: &Rig, task: &TaskId, number: u32) -> Option<String> {
    std::fs::read_to_string(rig.home.questions_file(task, NonZeroU32::new(number).unwrap())).ok()
}

/// Answers the first question of questions `number` of `task` with `text`, as `agent answer` does.
async fn answer(rig: &Rig, task: &TaskId, number: u32, text: &str) {
    let answers = AnswerList::try_from(vec![AnswerInput {
        question: NonZeroU32::MIN,
        verdict: Verdict::from_text(text).unwrap().unwrap(),
    }])
    .unwrap();
    let reply = crate::client::Connection::connect(&rig.home.socket_file())
        .await
        .unwrap()
        .request(
            Client::Cli,
            Command::Answer(AnswerRequest {
                task: task.clone(),
                number: NonZeroU32::new(number).unwrap(),
                answers,
            }),
        )
        .await
        .unwrap();
    assert!(matches!(reply, Reply::Applied { .. }), "{reply:?}");
}

/// A solo task whose claude-1 goes bad in both its sessions and is taken over by claude-2, which runs `scenario`
/// (its first run is the task's seventh), started and waiting for the human's answer after claude-2 went bad too.
async fn replacement_gone_bad(scenario: &'static str) -> (Rig, TaskId) {
    let mut rig = Rig::new(
        &[
            normal("claude-1", BAD_IN_BOTH_SESSIONS),
            normal("claude-2", scenario),
            investigator("inv-r1", APPROVE),
            investigator("inv-r2", APPROVE),
        ],
        5,
        false,
    )
    .await;
    configure(&rig, "[replacement]\nagents = [\"claude-2\"]\n");
    rig.start().await;
    let (_, tasks) = rig.run(&["claude-1"], ONE, MergeMode::Autopilot, None).await;
    let task = tasks[0].clone();
    rig.status_is(&task, TaskStatus::Questions).await;
    (rig, task)
}

// Rule: a task handed over whose new holder goes bad too, after its own retries, asks the human once, in one questions
// file of the harness's own text.
#[tokio::test]
async fn a_replacement_that_fails_too_asks_the_human_once() {
    let (mut rig, task) = replacement_gone_bad("nostatus").await;

    let state = rig.state();
    let text = questions(&rig, &task, 1).expect("the questions file the harness wrote");
    rig.stop().await.unwrap();

    let member = state.find_task(&task).unwrap().member;
    assert_eq!(member.agent().name(), "claude-2");
    assert_eq!(member.runs().len(), 9, "six runs of claude-1, three of claude-2");
    assert_eq!(
        member.open_question().map(agent_core::state::Question::number),
        NonZeroU32::new(1)
    );
    assert_eq!(member.asked_after_last_step(), NonZeroU32::new(1));
    assert!(text.starts_with(&format!("# Questions — {task}")), "{text}");
    assert!(text.contains("a) start it once more"), "{text}");
    assert!(text.contains("b) stop the task"), "{text}");
    assert!(
        text.contains("claude-1"),
        "it names the agent that held the task before: {text}"
    );
    assert!(questions(&rig, &task, 2).is_none(), "one questions file");
}

// Rule: the answer `a` starts the task once more, in a new session of the agent that holds it, briefed; it ends DONE.
#[tokio::test]
async fn answering_a_starts_the_task_once_more_in_a_new_session_of_its_agent() {
    let (mut rig, task) = replacement_gone_bad("done,done,done,done,done,done,nostatus,nostatus,nostatus,work").await;

    answer(&rig, &task, 1, "a").await;
    let state = rig
        .item_where("1.1", "merged after the answer", |phase| {
            matches!(phase, ItemPhase::Done { .. })
        })
        .await;
    let prompt = rig.prompt(&task, 10).await;
    rig.stop().await.unwrap();

    let member = state.find_task(&task).unwrap().member;
    assert_eq!(member.agent().name(), "claude-2");
    assert_eq!(member.runs().len(), 10);
    assert_eq!(
        member.renewals().map(Renewals::used),
        Some(1),
        "claude-1's one new session, no other is counted"
    );
    assert!(
        prompt.starts_with("# A NEW SESSION ON A TASK THAT WAS ALREADY STARTED"),
        "{prompt}"
    );
    assert!(prompt.contains("You are agent **claude-2**"), "{prompt}");
    let sessions: Vec<_> = member.runs().iter().map(|run| run.session().cloned()).collect();
    assert_ne!(
        sessions[9], sessions[8],
        "the run after the answer resumes no session: {sessions:?}"
    );
}

// Rule: the answer `b`, and accepting the recommendation, stop the task: nothing more runs on it.
#[tokio::test]
async fn answering_b_or_accepting_the_recommendation_stops_the_task() {
    for verdict in ["b", "ok"] {
        let (mut rig, task) = replacement_gone_bad("nostatus").await;

        answer(&rig, &task, 1, verdict).await;
        let state = rig.status_is(&task, TaskStatus::Stopped).await;
        rig.stop().await.unwrap();

        assert_eq!(
            state.find_task(&task).unwrap().member.runs().len(),
            9,
            "{verdict}: no run after the answer"
        );
    }
}

// Rule: the human is asked once: a task started once more by the answer that goes bad again fails as before.
#[tokio::test]
async fn a_task_that_goes_bad_again_after_its_question_fails_as_before() {
    let (mut rig, task) = replacement_gone_bad("nostatus").await;

    answer(&rig, &task, 1, "a").await;
    let state = rig.status_is(&task, TaskStatus::Failed).await;
    rig.stop().await.unwrap();

    let member = state.find_task(&task).unwrap().member;
    assert_eq!(
        member.runs().len(),
        12,
        "the three runs of the session the answer began"
    );
    assert_eq!(member.questions().len(), 1, "no second question");
    assert!(questions(&rig, &task, 2).is_none());
}

// Rule: a lead given a member's work that goes bad too, after the steps of its own ladder, asks the human once.
#[tokio::test]
async fn a_lead_given_a_members_work_that_fails_too_asks_the_human() {
    let mut rig = Rig::new(
        &[
            investigator("inv-lead", "sleep,nostatus"),
            normal("claude-1", BAD_IN_BOTH_SESSIONS),
            investigator("inv-r1", APPROVE),
            investigator("inv-r2", APPROVE),
        ],
        5,
        true,
    )
    .await;
    // The only agent listed is the lead itself: nobody fits, and the member's work goes to the lead.
    configure(&rig, "[replacement]\nagents = [\"inv-lead\"]\n");
    rig.start().await;
    let (_, tasks) = rig
        .run(&["inv-lead", "claude-1"], ONE, MergeMode::Autopilot, Some("inv-lead"))
        .await;

    let state = rig.status_is(&tasks[0], TaskStatus::Questions).await;
    let text = questions(&rig, &tasks[0], 1).expect("the questions file the harness wrote");
    rig.stop().await.unwrap();

    assert_eq!(state.find_task(&tasks[1]).unwrap().member.status(), TaskStatus::Stopped);
    let lead = state.find_task(&tasks[0]).unwrap().member;
    assert_eq!(lead.asked_after_last_step(), NonZeroU32::new(1));
    assert!(text.contains("inv-lead"), "{text}");
}
