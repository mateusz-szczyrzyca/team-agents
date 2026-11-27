//! A session whose context outgrew its agent's bound is not resumed: the next run of the task starts a session of its
//! own, briefed with the size that ended the old one; a session below the bound is resumed as before.

use agent_core::decide::ladder::Trouble;
use agent_core::state::{ItemPhase, MergeMode};

use super::rig::{CHAIN, ONE, Rig, bounded, investigator, normal};

/// The header every briefing of a new session starts with.
const BRIEFING: &str = "# A NEW SESSION ON A TASK THAT WAS ALREADY STARTED";

/// One line of Claude Code's stream JSON: a model call of `input` tokens of context, which the run reports as its usage.
fn usage_line(input: u64) -> String {
    format!(
        "{{\"type\":\"assistant\",\"message\":{{\"id\":\"msg-{input}\",\"role\":\"assistant\",\"content\":[{{\"type\":\"text\",\"text\":\"working\"}}],\"usage\":{{\"input_tokens\":{input},\"cache_read_input_tokens\":0,\"cache_creation_input_tokens\":0,\"output_tokens\":100}}}}}}\n"
    )
}

/// A rig whose agent streams one model call of `input` tokens in its first run, then works in the runs after it.
async fn rig_streaming(input: u64) -> Rig {
    rig_with_agent(normal("claude-1", "stream:first,work"), input).await
}

/// A rig whose agent `agent` streams one model call of `input` tokens in its first run, then works.
async fn rig_with_agent(agent: super::rig::Agent, input: u64) -> Rig {
    let mut rig = Rig::new(
        &[
            agent,
            investigator("inv-r1", "done:VERDICT: APPROVE"),
            investigator("inv-r2", "done:VERDICT: APPROVE"),
        ],
        5,
        false,
    )
    .await;
    let scripts = rig.home.root().join("fake-scripts");
    std::fs::create_dir_all(&scripts).unwrap();
    std::fs::write(scripts.join("first.jsonl"), usage_line(input)).unwrap();
    rig.start().await;
    rig
}

// Rule: a session that reached more tokens than its agent's bound is not resumed: the second run of the task has a
// session of its own, and its prompt is the briefing, which says why with the size; no renewal is counted.
#[tokio::test]
async fn a_session_above_its_agents_bound_is_not_resumed_and_the_next_run_is_briefed_with_its_size() {
    let mut rig = rig_streaming(200_000).await;
    let (_, tasks) = rig.run(&["claude-1"], CHAIN, MergeMode::Autopilot, None).await;
    let worktree = rig.worktree(&tasks[0]);
    let briefing = rig.prompt(&tasks[0], 2).await;
    let args = std::fs::read_to_string(worktree.join(".fake-args-2")).unwrap();
    let state = rig.state();
    rig.stop().await.unwrap();

    assert!(!args.contains("--resume"), "the old session was resumed: {args}");
    assert!(args.contains("--session-id"), "a new session: {args}");
    assert!(briefing.contains(BRIEFING), "{briefing}");
    assert!(briefing.contains("grown to 200000 tokens"), "{briefing}");
    let member = state.find_task(&tasks[0]).unwrap().member;
    let renewals = member.renewals().expect("the session above the bound began anew");
    assert_eq!(renewals.reason(), Trouble::ContextBound);
    assert_eq!(renewals.used(), 0, "a size above the bound is no renewal of the task");
}

// Rule: a session that stays at or below its agent's bound is resumed as before: the second run has the session of
// the first and its prompt is not a briefing.
#[tokio::test]
async fn a_session_below_its_agents_bound_is_resumed_as_before() {
    let mut rig = rig_streaming(42_000).await;
    let (_, tasks) = rig.run(&["claude-1"], CHAIN, MergeMode::Autopilot, None).await;
    let worktree = rig.worktree(&tasks[0]);
    let prompt = rig.prompt(&tasks[0], 2).await;
    let args = std::fs::read_to_string(worktree.join(".fake-args-2")).unwrap();
    rig.stop().await.unwrap();

    assert!(args.contains("--resume"), "the session is resumed: {args}");
    assert!(!prompt.contains(BRIEFING), "{prompt}");
}

// Rule: the bound of the agent replaces the `[session]` bound: a session above the agent's own bound is started anew
// although the default would resume it, and one below it is resumed although the default would start it anew.
#[tokio::test]
async fn an_agents_own_bound_replaces_the_session_bound_in_both_directions() {
    let mut lower = rig_with_agent(bounded("claude-1", "stream:first,work", 100_000), 120_000).await;
    let (_, lower_tasks) = lower.run(&["claude-1"], CHAIN, MergeMode::Autopilot, None).await;
    let lower_briefing = lower.prompt(&lower_tasks[0], 2).await;
    let lower_args = std::fs::read_to_string(lower.worktree(&lower_tasks[0]).join(".fake-args-2")).unwrap();
    lower.stop().await.unwrap();

    let mut higher = rig_with_agent(bounded("claude-1", "stream:first,work", 300_000), 200_000).await;
    let (_, higher_tasks) = higher.run(&["claude-1"], CHAIN, MergeMode::Autopilot, None).await;
    higher.prompt(&higher_tasks[0], 2).await;
    let higher_args = std::fs::read_to_string(higher.worktree(&higher_tasks[0]).join(".fake-args-2")).unwrap();
    higher.stop().await.unwrap();

    assert!(lower_briefing.contains("grown to 120000 tokens"), "{lower_briefing}");
    assert!(!lower_args.contains("--resume"), "{lower_args}");
    assert!(
        higher_args.contains("--resume"),
        "the session is resumed: {higher_args}"
    );
}

/// Asks the daemon to resume `task`, accepting that it is already going: a waiting task can be started by the daemon itself
/// between the test's read of its status and the request, under load.
async fn resume_unless_already_going(rig: &Rig, task: &agent_core::ids::TaskId) {
    use agent_core::protocol::{ControlRequest, Reply, Target};
    let reply = crate::client::Connection::connect(&rig.home.socket_file())
        .await
        .unwrap()
        .request(
            agent_core::protocol::Client::Cli,
            agent_core::protocol::Command::Resume(ControlRequest {
                target: Target::Task(task.clone()),
            }),
        )
        .await;
    match reply {
        Ok(Reply::Applied { .. }) => {}
        Err(crate::client::ClientError::Refused(failure)) if failure.message.contains("it is already going") => {}
        other => panic!("unexpected reply: {other:?}"),
    }
}

/// A reviewer that asks for a change the first time and approves afterwards: the fix pass the restart tests use.
const CHANGES_ONCE: &str = "verify:VERDICT: REQUEST_CHANGES\\nFINDINGS\\n1. [MAJOR] TDD-2 src/lib.rs:7 — the case is untested → add coverage\\n   CHECK: read src/lib.rs:7,verify:VERDICT: APPROVE\\nSUMMARY: fixed";

/// The approving reviewer of the fix pass.
const APPROVING: &str = "verify:VERDICT: APPROVE\\nSUMMARY: fine";

// Rule: a run that ends above its agent's bound leaves an uncommitted file behind; the next run is a new session whose
// briefing names that file, the commits of the task and its item, and the size; the new session finishes the item alone.
#[tokio::test]
async fn the_new_session_is_briefed_with_the_uncommitted_file_of_the_run_above_the_bound_and_finishes_the_item() {
    let mut rig = Rig::new(
        &[
            normal(
                "claude-1",
                "work-stream:first,ask-stream:second,work-if-briefed:grown to 200000 tokens|- draft.txt|Work of run 1 of claude-1|Do the second thing in the repository.|PRIOR-FINAL-MARKER|Continue the part?|HUMAN-ANSWER-MARKER keep the store",
            ),
            investigator("inv-r1", APPROVING),
            investigator("inv-r2", APPROVING),
        ],
        5,
        false,
    )
    .await;
    let scripts = rig.home.root().join("fake-scripts");
    std::fs::create_dir_all(&scripts).unwrap();
    std::fs::write(scripts.join("first.jsonl"), usage_line(42_000)).unwrap();
    std::fs::write(scripts.join("second.jsonl"), usage_line(200_000)).unwrap();
    std::fs::write(
        scripts.join("second.final.txt"),
        "The session before ends here: PRIOR-FINAL-MARKER\n",
    )
    .unwrap();
    rig.start().await;
    let (_, tasks) = rig.run(&["claude-1"], CHAIN, MergeMode::Autopilot, None).await;
    let worktree = rig.worktree(&tasks[0]);
    rig.status_is(&tasks[0], agent_core::status::TaskStatus::Questions)
        .await;
    answer_first_question(&rig, &tasks[0], 1, "HUMAN-ANSWER-MARKER keep the store").await;
    let briefing = rig.prompt(&tasks[0], 3).await;
    let args = std::fs::read_to_string(worktree.join(".fake-args-3")).unwrap();
    let state = rig
        .item_where("1.2", "done after the new session", |phase| {
            matches!(phase, ItemPhase::Done { .. })
        })
        .await;
    rig.stop().await.unwrap();

    assert!(briefing.contains(BRIEFING), "{briefing}");
    assert!(briefing.contains("grown to 200000 tokens"), "{briefing}");
    assert!(
        briefing.contains("- draft.txt"),
        "the uncommitted file is not listed: {briefing}"
    );
    assert!(
        briefing.contains("Work of run 1 of claude-1"),
        "the commit is not listed: {briefing}"
    );
    assert!(!args.contains("--resume"), "the old session was resumed: {args}");
    let member = state.find_task(&tasks[0]).unwrap().member;
    assert_eq!(
        member.renewals().map(agent_core::state::Renewals::reason),
        Some(Trouble::ContextBound)
    );
}

// Rule: a session below its agent's bound is resumed for a fix pass of its item as before: the fix pass runs in the
// session of the run it follows, and the item is merged after it.
#[tokio::test]
async fn a_session_below_the_bound_is_resumed_for_a_fix_pass() {
    let mut rig = Rig::new(
        &[
            normal("claude-1", "work-stream:first,fix-work"),
            investigator("inv-r1", CHANGES_ONCE),
            investigator("inv-r2", APPROVING),
        ],
        5,
        false,
    )
    .await;
    let scripts = rig.home.root().join("fake-scripts");
    std::fs::create_dir_all(&scripts).unwrap();
    std::fs::write(scripts.join("first.jsonl"), usage_line(42_000)).unwrap();
    rig.start().await;
    let (team, tasks) = rig.run(&["claude-1"], ONE, MergeMode::Autopilot, None).await;
    let worktree = rig.worktree(&tasks[0]);
    rig.prompt(&tasks[0], 2).await;
    let args = std::fs::read_to_string(worktree.join(".fake-args-2")).unwrap();
    let state = rig
        .item_where("1.1", "merged after the fix pass", |phase| {
            matches!(phase, ItemPhase::Done { .. })
        })
        .await;
    rig.stop().await.unwrap();

    assert!(
        args.contains("--resume"),
        "the session below the bound is resumed: {args}"
    );
    let progress = state.team(&team).unwrap().item("1.1".parse().unwrap()).unwrap();
    assert_eq!(progress.fix_passes().len(), 1, "the fix pass ran once");
}

/// Answers the first question of questions `number` of `task` with `text`, as `agent answer` does.
async fn answer_first_question(rig: &Rig, task: &agent_core::ids::TaskId, number: u32, text: &str) {
    use agent_core::protocol::{AnswerInput, AnswerList, AnswerRequest, Client, Command, Reply};
    use agent_core::questions::Verdict;
    let answers = AnswerList::try_from(vec![AnswerInput {
        question: std::num::NonZeroU32::MIN,
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
                number: std::num::NonZeroU32::new(number).unwrap(),
                answers,
            }),
        )
        .await
        .unwrap();
    assert!(matches!(reply, Reply::Applied { .. }), "unexpected reply: {reply:?}");
}

// Rule: a session above its agent's bound is started anew even when the run that reached it asked the human, and the new
// session is briefed with the item it still works on: the item title and its expected result are in the briefing.
#[tokio::test]
async fn the_new_session_after_an_answer_is_briefed_with_the_item_still_in_progress() {
    let mut rig = Rig::new(
        &[
            normal("claude-1", "work-stream:first,ask-stream:second,work"),
            investigator("inv-r1", APPROVING),
            investigator("inv-r2", APPROVING),
        ],
        5,
        false,
    )
    .await;
    let scripts = rig.home.root().join("fake-scripts");
    std::fs::create_dir_all(&scripts).unwrap();
    std::fs::write(scripts.join("first.jsonl"), usage_line(42_000)).unwrap();
    std::fs::write(scripts.join("second.jsonl"), usage_line(200_000)).unwrap();
    rig.start().await;
    let (_, tasks) = rig.run(&["claude-1"], CHAIN, MergeMode::Autopilot, None).await;
    let worktree = rig.worktree(&tasks[0]);
    rig.status_is(&tasks[0], agent_core::status::TaskStatus::Questions)
        .await;
    answer_first_question(&rig, &tasks[0], 1, "ok").await;
    let briefing = rig.prompt(&tasks[0], 3).await;
    let args = std::fs::read_to_string(worktree.join(".fake-args-3")).unwrap();
    rig.stop().await.unwrap();

    assert!(briefing.contains(BRIEFING), "{briefing}");
    assert!(briefing.contains("grown to 200000 tokens"), "{briefing}");
    assert!(
        briefing.contains("The second item"),
        "the item title is not in the briefing: {briefing}"
    );
    assert!(
        briefing.contains("Do the second thing in the repository."),
        "the item's text is not in the briefing: {briefing}"
    );
    assert!(
        briefing.contains("Continue the part?"),
        "the answer is not in the briefing: {briefing}"
    );
    assert!(
        briefing.contains("I have to ask."),
        "the last final message is not in the briefing: {briefing}"
    );
    assert!(!args.contains("--resume"), "the old session was resumed: {args}");
}

// Rule: a session above its agent's bound that waits for a message, its round closed, is started anew briefed from the
// task's newest round: the new session's prompt holds the size paragraph, the briefing and the task's whole prompt.
#[tokio::test]
async fn a_renewed_session_of_a_task_with_no_open_round_is_briefed_from_its_newest_round() {
    let mut rig = Rig::new(
        &[
            normal("claude-1", "work-stream:first,sleep-stream:second,work"),
            investigator("inv-r1", APPROVING),
            investigator("inv-r2", APPROVING),
        ],
        5,
        false,
    )
    .await;
    let scripts = rig.home.root().join("fake-scripts");
    std::fs::create_dir_all(&scripts).unwrap();
    std::fs::write(scripts.join("first.jsonl"), usage_line(42_000)).unwrap();
    std::fs::write(scripts.join("second.jsonl"), usage_line(200_000)).unwrap();
    // The second item's pull request cannot be merged, so nothing finishes the team between the read of the state and
    // the stop of the daemon below: the task waits there until the test moves it.
    rig.github.fail_calls_to(
        "PUT",
        &format!("/repos/{}/pulls/2/merge", super::fake_github::SLUG),
        500,
    );
    rig.start().await;
    let (_, tasks) = rig.run(&["claude-1"], CHAIN, MergeMode::Autopilot, None).await;
    let worktree = rig.worktree(&tasks[0]);
    rig.state_where("the task to wait with its run ended", |state| {
        state.find_task(&tasks[0]).is_some_and(|found| {
            found.member.status() == agent_core::status::TaskStatus::Waiting
                && found.member.runs().len() == 2
                && found.member.runs().last().is_some_and(|run| !run.is_running())
        })
    })
    .await;
    // The daemon is stopped before anything else moves: the state is then the one the run left. The rounds the waiting run
    // worked for are closed by hand, so that the task has no open round, and the state is saved and the daemon started again.
    rig.stop().await.unwrap();
    let mut state = rig.state();
    let open: Vec<_> = state
        .find_task(&tasks[0])
        .unwrap()
        .member
        .runs()
        .last()
        .unwrap()
        .rounds()
        .to_vec();
    let at: agent_core::state::Timestamp = "2999-01-01T00:00:00Z".parse().unwrap();
    for round in open {
        state
            .close_round(&round, at, agent_core::state::RoundEnd::Abandoned)
            .unwrap();
    }
    std::fs::write(rig.home.state_file(), state.to_json().unwrap()).unwrap();
    rig.start().await;
    resume_unless_already_going(&rig, &tasks[0]).await;
    let prompt = rig.prompt(&tasks[0], 3).await;
    let args = std::fs::read_to_string(worktree.join(".fake-args-3")).unwrap();
    rig.stop().await.unwrap();

    assert!(prompt.contains("grown to 200000 tokens"), "{prompt}");
    assert!(prompt.contains("# Working context"), "{prompt}");
    assert!(
        prompt.contains(BRIEFING),
        "the briefing of a task with no open round: {prompt}"
    );
    assert!(!args.contains("--resume"), "the old session was resumed: {args}");
}

// Rule: a renewal for the bound removes the session the next run would resume, so the next run is a whole new session: its
// prompt is the task's full prompt, not the short resume that follows a run which ended without a STATUS line.
#[tokio::test]
async fn a_context_renewal_after_no_status_starts_with_the_task_and_not_with_a_resume() {
    let mut rig = Rig::new(
        &[
            normal("claude-1", "nostatus-stream:first,work"),
            investigator("inv-r1", APPROVING),
            investigator("inv-r2", APPROVING),
        ],
        5,
        false,
    )
    .await;
    let scripts = rig.home.root().join("fake-scripts");
    std::fs::create_dir_all(&scripts).unwrap();
    std::fs::write(scripts.join("first.jsonl"), usage_line(200_000)).unwrap();
    rig.start().await;
    let (_, tasks) = rig.run(&["claude-1"], ONE, MergeMode::Autopilot, None).await;
    let prompt = rig.prompt(&tasks[0], 2).await;
    let args = std::fs::read_to_string(rig.worktree(&tasks[0]).join(".fake-args-2")).unwrap();
    rig.stop().await.unwrap();

    assert!(!args.contains("--resume"), "the old session was resumed: {args}");
    assert!(
        prompt.contains("Do the first thing in the repository."),
        "the task is not in the prompt: {prompt}"
    );
}

// Rule: the last message of the session before is given whole, whatever the history keeps: a history that cuts a text at a
// small limit must not cut the briefing's message.
#[tokio::test]
async fn the_last_message_of_the_session_before_is_whole_when_the_history_cuts_texts_short() {
    let mut rig = Rig::new(
        &[
            normal("claude-1", "work-stream:first,ask-stream:second,work"),
            investigator("inv-r1", APPROVING),
            investigator("inv-r2", APPROVING),
        ],
        5,
        false,
    )
    .await;
    let config = std::fs::read_to_string(rig.home.config_file()).unwrap();
    std::fs::write(
        rig.home.config_file(),
        format!("{config}\n[history]\nmax_text_bytes = 16\n"),
    )
    .unwrap();
    let scripts = rig.home.root().join("fake-scripts");
    std::fs::create_dir_all(&scripts).unwrap();
    std::fs::write(scripts.join("first.jsonl"), usage_line(42_000)).unwrap();
    std::fs::write(scripts.join("second.jsonl"), usage_line(200_000)).unwrap();
    std::fs::write(
        scripts.join("second.final.txt"),
        "The whole message of the session before, longer than sixteen bytes: END-OF-MESSAGE\n",
    )
    .unwrap();
    rig.start().await;
    let (_, tasks) = rig.run(&["claude-1"], CHAIN, MergeMode::Autopilot, None).await;
    rig.status_is(&tasks[0], agent_core::status::TaskStatus::Questions)
        .await;
    answer_first_question(&rig, &tasks[0], 1, "ok").await;
    let briefing = rig.prompt(&tasks[0], 3).await;
    rig.stop().await.unwrap();

    assert!(
        briefing.contains("END-OF-MESSAGE"),
        "the last message is cut: {briefing}"
    );
}

// Rule: a session above its agent's bound is replaced even when the history is disabled: the run's outcome file is the
// record of its size, and the history writes nothing.
#[tokio::test]
async fn a_session_above_the_bound_is_replaced_with_the_history_disabled() {
    let mut rig = Rig::new(
        &[
            normal("claude-1", "work-stream:first,ask-stream:second,work"),
            investigator("inv-r1", APPROVING),
            investigator("inv-r2", APPROVING),
        ],
        5,
        false,
    )
    .await;
    let config = std::fs::read_to_string(rig.home.config_file()).unwrap();
    std::fs::write(
        rig.home.config_file(),
        format!("{config}\n[history]\nenabled = false\n"),
    )
    .unwrap();
    let scripts = rig.home.root().join("fake-scripts");
    std::fs::create_dir_all(&scripts).unwrap();
    std::fs::write(scripts.join("first.jsonl"), usage_line(42_000)).unwrap();
    std::fs::write(scripts.join("second.jsonl"), usage_line(200_000)).unwrap();
    rig.start().await;
    let (_, tasks) = rig.run(&["claude-1"], CHAIN, MergeMode::Autopilot, None).await;
    rig.status_is(&tasks[0], agent_core::status::TaskStatus::Questions)
        .await;
    answer_first_question(&rig, &tasks[0], 1, "ok").await;
    let briefing = rig.prompt(&tasks[0], 3).await;
    let args = std::fs::read_to_string(rig.worktree(&tasks[0]).join(".fake-args-3")).unwrap();
    rig.stop().await.unwrap();

    assert!(briefing.contains("grown to 200000 tokens"), "{briefing}");
    assert!(!args.contains("--resume"), "the old session was resumed: {args}");
}

// Rule: a corrupt outcome file of the run above the bound does not decide alone: the history's size replaces an oversized
// session, and the history's copy of the last message briefs it.
#[tokio::test]
async fn a_corrupt_outcome_file_is_judged_by_the_history_size_and_replaces_the_session() {
    let mut rig = Rig::new(
        &[
            normal("claude-1", "work-stream:first,ask-stream:second,work"),
            investigator("inv-r1", APPROVING),
            investigator("inv-r2", APPROVING),
        ],
        5,
        false,
    )
    .await;
    let scripts = rig.home.root().join("fake-scripts");
    std::fs::create_dir_all(&scripts).unwrap();
    std::fs::write(scripts.join("first.jsonl"), usage_line(42_000)).unwrap();
    std::fs::write(scripts.join("second.jsonl"), usage_line(200_000)).unwrap();
    rig.start().await;
    let (_, tasks) = rig.run(&["claude-1"], CHAIN, MergeMode::Autopilot, None).await;
    rig.status_is(&tasks[0], agent_core::status::TaskStatus::Questions)
        .await;
    let run_dir = rig.home.run_dir(
        &tasks[0],
        rig.state()
            .find_task(&tasks[0])
            .unwrap()
            .member
            .runs()
            .last()
            .unwrap()
            .id(),
    );
    std::fs::write(run_dir.join("outcome.json"), "not an outcome").unwrap();
    answer_first_question(&rig, &tasks[0], 1, "ok").await;
    let prompt = rig.prompt(&tasks[0], 3).await;
    let args = std::fs::read_to_string(rig.worktree(&tasks[0]).join(".fake-args-3")).unwrap();
    rig.stop().await.unwrap();

    assert!(prompt.contains("grown to 200000 tokens"), "{prompt}");
    assert!(!args.contains("--resume"), "the old session was resumed: {args}");
}

// Rule: two agents of one team with different bounds decide alone: the same size, 120 000 tokens, replaces the session of the
// agent whose bound is 100 000 and resumes the session of the agent whose bound is the default of 150 000.
#[tokio::test]
async fn two_agents_of_one_team_with_different_bounds_decide_on_the_same_size_each_by_its_own() {
    let mut rig = Rig::new(
        &[
            bounded("claude-1", "ask-stream:first,work", 100_000),
            normal("claude-2", "ask-stream:first,work"),
            investigator("inv-r1", APPROVING),
            investigator("inv-r2", APPROVING),
        ],
        5,
        false,
    )
    .await;
    let scripts = rig.home.root().join("fake-scripts");
    std::fs::create_dir_all(&scripts).unwrap();
    std::fs::write(scripts.join("first.jsonl"), usage_line(120_000)).unwrap();
    rig.start().await;
    let (_, tasks) = rig
        .run(
            &["claude-1", "claude-2"],
            super::rig::PARALLEL,
            MergeMode::Autopilot,
            None,
        )
        .await;
    for task in &tasks {
        rig.status_is(task, agent_core::status::TaskStatus::Questions).await;
    }
    for task in &tasks {
        answer_first_question(&rig, task, 1, "ok").await;
    }
    let mut outcomes = Vec::new();
    for task in &tasks {
        let agent = rig.state().find_task(task).unwrap().member.agent().name().to_owned();
        let prompt = rig.prompt(task, 2).await;
        let args = std::fs::read_to_string(rig.worktree(task).join(".fake-args-2")).unwrap();
        outcomes.push((agent, prompt, args));
    }
    rig.stop().await.unwrap();

    for (agent, prompt, args) in outcomes {
        if agent == "claude-1" {
            assert!(prompt.contains("grown to 120000 tokens"), "{agent}: {prompt}");
            assert!(!args.contains("--resume"), "{agent} was resumed: {args}");
        } else {
            assert!(args.contains("--resume"), "{agent} was not resumed: {args}");
            assert!(!prompt.contains(BRIEFING), "{agent}: {prompt}");
        }
    }
}

// Rule: a last message read from the history because its run wrote no outcome file says so when the history cut it: the
// briefing names the limit at which it was cut, so that the new session knows the message is not whole.
#[tokio::test]
async fn a_legacy_run_s_last_message_cut_by_the_history_is_said_to_be_cut() {
    let mut rig = Rig::new(
        &[
            normal("claude-1", "work-stream:first,ask-stream:second,work"),
            investigator("inv-r1", APPROVING),
            investigator("inv-r2", APPROVING),
        ],
        5,
        false,
    )
    .await;
    let config = std::fs::read_to_string(rig.home.config_file()).unwrap();
    std::fs::write(
        rig.home.config_file(),
        format!("{config}\n[history]\nmax_text_bytes = 16\n"),
    )
    .unwrap();
    let scripts = rig.home.root().join("fake-scripts");
    std::fs::create_dir_all(&scripts).unwrap();
    std::fs::write(scripts.join("first.jsonl"), usage_line(42_000)).unwrap();
    std::fs::write(scripts.join("second.jsonl"), usage_line(200_000)).unwrap();
    std::fs::write(
        scripts.join("second.final.txt"),
        "The whole message of the session before, longer than sixteen bytes: END-OF-MESSAGE\n",
    )
    .unwrap();
    rig.start().await;
    let (_, tasks) = rig.run(&["claude-1"], CHAIN, MergeMode::Autopilot, None).await;
    rig.status_is(&tasks[0], agent_core::status::TaskStatus::Questions)
        .await;
    let last = rig
        .state()
        .find_task(&tasks[0])
        .unwrap()
        .member
        .runs()
        .last()
        .unwrap()
        .id()
        .clone();
    std::fs::remove_file(rig.home.run_dir(&tasks[0], &last).join("outcome.json")).unwrap();
    answer_first_question(&rig, &tasks[0], 1, "ok").await;
    let briefing = rig.prompt(&tasks[0], 3).await;
    rig.stop().await.unwrap();

    assert!(briefing.contains("cut at the history limit of 16 bytes"), "{briefing}");
    assert!(
        !briefing.contains("## The last message of the session before yours, whole"),
        "a message read from the history is not whole: {briefing}"
    );
    assert!(
        briefing.contains("## The last message of the session before yours\n"),
        "{briefing}"
    );
}

// Rule: a run whose outcome file cannot be read, and whose history has no size for it either, may be oversized: the session is
// replaced, not resumed, and the briefing says its size could not be read. The message comes from the history, which keeps it.
#[tokio::test]
async fn a_corrupt_outcome_file_with_no_size_in_the_history_replaces_the_session_with_an_unknown_size() {
    let mut rig = Rig::new(
        &[
            normal("claude-1", "work-stream:first,ask-here,work"),
            investigator("inv-r1", APPROVING),
            investigator("inv-r2", APPROVING),
        ],
        5,
        false,
    )
    .await;
    let scripts = rig.home.root().join("fake-scripts");
    std::fs::create_dir_all(&scripts).unwrap();
    std::fs::write(scripts.join("first.jsonl"), usage_line(42_000)).unwrap();
    rig.start().await;
    let (_, tasks) = rig.run(&["claude-1"], CHAIN, MergeMode::Autopilot, None).await;
    rig.status_is(&tasks[0], agent_core::status::TaskStatus::Questions)
        .await;
    let last = rig
        .state()
        .find_task(&tasks[0])
        .unwrap()
        .member
        .runs()
        .last()
        .unwrap()
        .id()
        .clone();
    std::fs::write(
        rig.home.run_dir(&tasks[0], &last).join("outcome.json"),
        "not an outcome",
    )
    .unwrap();
    answer_first_question(&rig, &tasks[0], 1, "ok").await;
    let prompt = rig.prompt(&tasks[0], 3).await;
    let args = std::fs::read_to_string(rig.worktree(&tasks[0]).join(".fake-args-3")).unwrap();
    rig.stop().await.unwrap();

    assert!(prompt.contains("its size could not be read"), "{prompt}");
    assert!(
        !args.contains("--resume"),
        "the session was resumed on a size nobody could read: {args}"
    );
}

// Rule: a run with neither an outcome file nor a usage row tells no size, and its session resumes as before.
#[tokio::test]
async fn a_run_with_no_outcome_file_and_no_usage_row_resumes_its_session() {
    let mut rig = Rig::new(
        &[
            normal("claude-1", "work-stream:first,ask-here,work"),
            investigator("inv-r1", APPROVING),
            investigator("inv-r2", APPROVING),
        ],
        5,
        false,
    )
    .await;
    let scripts = rig.home.root().join("fake-scripts");
    std::fs::create_dir_all(&scripts).unwrap();
    std::fs::write(scripts.join("first.jsonl"), usage_line(42_000)).unwrap();
    rig.start().await;
    let (_, tasks) = rig.run(&["claude-1"], CHAIN, MergeMode::Autopilot, None).await;
    rig.status_is(&tasks[0], agent_core::status::TaskStatus::Questions)
        .await;
    let last = rig
        .state()
        .find_task(&tasks[0])
        .unwrap()
        .member
        .runs()
        .last()
        .unwrap()
        .id()
        .clone();
    std::fs::remove_file(rig.home.run_dir(&tasks[0], &last).join("outcome.json")).unwrap();
    answer_first_question(&rig, &tasks[0], 1, "ok").await;
    let args = {
        let _prompt = rig.prompt(&tasks[0], 3).await;
        std::fs::read_to_string(rig.worktree(&tasks[0]).join(".fake-args-3")).unwrap()
    };
    rig.stop().await.unwrap();

    assert!(
        args.contains("--resume"),
        "the session tells no size and resumes: {args}"
    );
}

// Rule: a renewal whose last message is owed to the briefing fails the start, with the typed history error, when the history
// cannot be read: a run is never started unbriefed for want of the history.
#[tokio::test]
async fn an_older_run_whose_history_cannot_be_read_fails_the_start_with_the_history_error() {
    let mut rig = Rig::new(
        &[
            normal("claude-1", "work-stream:first,ask-stream:second,work"),
            investigator("inv-r1", APPROVING),
            investigator("inv-r2", APPROVING),
        ],
        5,
        false,
    )
    .await;
    let scripts = rig.home.root().join("fake-scripts");
    std::fs::create_dir_all(&scripts).unwrap();
    std::fs::write(scripts.join("first.jsonl"), usage_line(42_000)).unwrap();
    std::fs::write(scripts.join("second.jsonl"), usage_line(200_000)).unwrap();
    rig.start().await;
    let (_, tasks) = rig.run(&["claude-1"], CHAIN, MergeMode::Autopilot, None).await;
    rig.status_is(&tasks[0], agent_core::status::TaskStatus::Questions)
        .await;
    let last = rig
        .state()
        .find_task(&tasks[0])
        .unwrap()
        .member
        .runs()
        .last()
        .unwrap()
        .id()
        .clone();
    std::fs::remove_file(rig.home.run_dir(&tasks[0], &last).join("outcome.json")).unwrap();
    // A directory of the history's name: the reader cannot open it, and the writer's open file is unaffected.
    std::fs::remove_file(rig.home.history_file()).unwrap();
    std::fs::create_dir(rig.home.history_file()).unwrap();
    answer_first_question(&rig, &tasks[0], 1, "ok").await;
    rig.status_is(&tasks[0], agent_core::status::TaskStatus::Failed).await;
    let notice = rig
        .notification_with("cannot read the runs of the task from the history")
        .await;
    let started_third = rig.worktree(&tasks[0]).join(".fake-prompt-3").exists();
    rig.stop().await.unwrap();

    assert!(
        !started_third,
        "the run started unbriefed without its history: {notice}"
    );
    assert!(notice.contains("history"), "{notice}");
}

// Rule: the size paragraph of a replacement belongs to the first run of the new session only: a run after it that is resumed
// says nothing of the replacement, although the size of the session it replaced could not be read.
#[tokio::test]
async fn a_run_resumed_after_a_replacement_with_an_unknown_size_says_nothing_of_it() {
    let mut rig = Rig::new(
        &[
            normal("claude-1", "work-stream:first,ask-here,nostatus-stream:third,work"),
            investigator("inv-r1", APPROVING),
            investigator("inv-r2", APPROVING),
        ],
        5,
        false,
    )
    .await;
    let scripts = rig.home.root().join("fake-scripts");
    std::fs::create_dir_all(&scripts).unwrap();
    std::fs::write(scripts.join("first.jsonl"), usage_line(42_000)).unwrap();
    std::fs::write(scripts.join("third.jsonl"), usage_line(42_000)).unwrap();
    rig.start().await;
    let (_, tasks) = rig.run(&["claude-1"], CHAIN, MergeMode::Autopilot, None).await;
    rig.status_is(&tasks[0], agent_core::status::TaskStatus::Questions)
        .await;
    let last = rig
        .state()
        .find_task(&tasks[0])
        .unwrap()
        .member
        .runs()
        .last()
        .unwrap()
        .id()
        .clone();
    std::fs::write(
        rig.home.run_dir(&tasks[0], &last).join("outcome.json"),
        "not an outcome",
    )
    .unwrap();
    answer_first_question(&rig, &tasks[0], 1, "ok").await;
    let _third = rig.prompt(&tasks[0], 3).await;
    let fourth = rig.prompt(&tasks[0], 4).await;
    let args = std::fs::read_to_string(rig.worktree(&tasks[0]).join(".fake-args-4")).unwrap();
    rig.stop().await.unwrap();

    assert!(
        args.contains("--resume"),
        "the run after the replacement is resumed: {args}"
    );
    assert!(!fourth.contains("its size could not be read"), "{fourth}");
    assert!(!fourth.contains("replaces it as a precaution"), "{fourth}");
}

// Rule: the same holds after a replacement with a known size: a resumed run after it says nothing of the replacement.
#[tokio::test]
async fn a_run_resumed_after_a_replacement_with_a_known_size_says_nothing_of_it() {
    let mut rig = Rig::new(
        &[
            normal(
                "claude-1",
                "work-stream:first,ask-stream:second,nostatus-stream:third,work",
            ),
            investigator("inv-r1", APPROVING),
            investigator("inv-r2", APPROVING),
        ],
        5,
        false,
    )
    .await;
    let scripts = rig.home.root().join("fake-scripts");
    std::fs::create_dir_all(&scripts).unwrap();
    std::fs::write(scripts.join("first.jsonl"), usage_line(42_000)).unwrap();
    std::fs::write(scripts.join("second.jsonl"), usage_line(200_000)).unwrap();
    std::fs::write(scripts.join("third.jsonl"), usage_line(42_000)).unwrap();
    rig.start().await;
    let (_, tasks) = rig.run(&["claude-1"], CHAIN, MergeMode::Autopilot, None).await;
    rig.status_is(&tasks[0], agent_core::status::TaskStatus::Questions)
        .await;
    answer_first_question(&rig, &tasks[0], 1, "ok").await;
    let _third = rig.prompt(&tasks[0], 3).await;
    let fourth = rig.prompt(&tasks[0], 4).await;
    let args = std::fs::read_to_string(rig.worktree(&tasks[0]).join(".fake-args-4")).unwrap();
    rig.stop().await.unwrap();

    assert!(
        args.contains("--resume"),
        "the run after the replacement is resumed: {args}"
    );
    assert!(!fourth.contains("grown to"), "{fourth}");
    assert!(!fourth.contains("its size could not be read"), "{fourth}");
    assert!(!fourth.contains("replaces it as a precaution"), "{fourth}");
}

#[tokio::test]
async fn a_new_session_keeps_its_whole_prompt_and_a_changed_item_is_sent_on_resume() {
    let mut rig = rig_with_agent(normal("claude-1", "stream:first,hold"), 42_000).await;
    let (_, tasks) = rig.run(&["claude-1"], CHAIN, MergeMode::Autopilot, None).await;
    let prompt = rig.prompt(&tasks[0], 2).await;
    rig.stop().await.unwrap();
    let first = rig.kept_prompt(&tasks[0], 1);
    assert!(rig.home.task_dir(&tasks[0]).join("session-prompt.md").is_file());
    let saved = std::fs::read_to_string(rig.home.task_dir(&tasks[0]).join("session-prompt.md")).unwrap();
    assert_eq!(saved, first.strip_suffix('\n').unwrap());
    assert!(prompt.contains("# YOUR SESSION GOES ON"), "{prompt}");
    assert!(prompt.contains("## Your item: 1.2"), "{prompt}");
    assert!(!prompt.contains("# Rust developer rules"), "{prompt}");
}

#[tokio::test]
async fn a_session_replaced_by_its_bound_overwrites_the_saved_whole_prompt() {
    let mut rig = rig_with_agent(normal("claude-1", "stream:first,hold"), 200_000).await;
    let (_, tasks) = rig.run(&["claude-1"], CHAIN, MergeMode::Autopilot, None).await;
    let prompt = rig.prompt(&tasks[0], 2).await;
    rig.stop().await.unwrap();
    assert!(rig.home.task_dir(&tasks[0]).join("session-prompt.md").is_file());
    let saved = std::fs::read_to_string(rig.home.task_dir(&tasks[0]).join("session-prompt.md")).unwrap();
    assert_eq!(saved, prompt.strip_suffix('\n').unwrap());
    assert!(prompt.contains("# Working context"), "{prompt}");
    assert!(prompt.contains("# Rust developer rules"), "{prompt}");
    assert_ne!(saved, rig.kept_prompt(&tasks[0], 1));
}

#[tokio::test]
async fn a_human_stopped_session_resumes_with_the_whole_prompt() {
    use agent_core::protocol::{Client, Command, ControlRequest, Target};
    use agent_core::status::TaskStatus;
    let mut rig = Rig::new(
        &[
            normal("claude-1", "hold,work"),
            investigator("inv-r1", APPROVING),
            investigator("inv-r2", APPROVING),
        ],
        5,
        false,
    )
    .await;
    rig.start().await;
    let (_, tasks) = rig.run(&["claude-1"], ONE, MergeMode::Autopilot, None).await;
    rig.prompt(&tasks[0], 1).await;
    crate::client::Connection::connect(&rig.home.socket_file())
        .await
        .unwrap()
        .request(
            Client::Cli,
            Command::Stop(ControlRequest {
                target: Target::Task(tasks[0].clone()),
            }),
        )
        .await
        .unwrap();
    rig.status_is(&tasks[0], TaskStatus::Stopped).await;
    resume_unless_already_going(&rig, &tasks[0]).await;
    let prompt = rig.prompt(&tasks[0], 2).await;
    rig.stop().await.unwrap();
    assert!(prompt.contains("# Working context"), "{prompt}");
    assert!(prompt.contains("# Rust developer rules"), "{prompt}");
}
