//! A session whose context outgrew its agent's bound is not resumed: the next run of the task starts a session of its
//! own, briefed with the size that ended the old one; a session below the bound is resumed as before.

use agent_core::decide::ladder::Trouble;
use agent_core::state::MergeMode;

use super::rig::{CHAIN, Rig, investigator, normal};

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
    let mut rig = Rig::new(
        &[
            normal("claude-1", "stream:first,work"),
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
