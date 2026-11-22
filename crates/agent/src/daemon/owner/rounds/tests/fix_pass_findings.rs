//! The findings of a fix pass reach the lead whole and stay where it can read them again: in the first prompt of the pass,
//! and in a file of its task directory that every later prompt of the pass names.

use super::rig::{ONE, Rig, investigator, normal};
use agent_core::state::MergeMode;

// Rule: the first run of the lead after the verdict has the text of every open finding, the same text is written to the
// file of its task directory, and the prompt names the file; so does the request to correct a wrong response.
#[tokio::test]
async fn the_findings_of_a_fix_pass_are_in_its_first_prompt_and_in_a_file_that_the_correction_names() {
    let mut rig = Rig::new(
        &[
            normal("claude-1", "work,fix-work:99"),
            investigator("inv-r1", "review-reasons"),
            investigator("inv-r2", "review-reasons"),
        ],
        1,
        false,
    )
    .await;
    rig.start().await;
    let (_, tasks) = rig.run(&["claude-1"], ONE, MergeMode::Autopilot, None).await;
    rig.notification_with("The lead's fix response was refused").await;
    rig.stop().await.unwrap();

    let file = rig.home.fix_pass_findings_file_of(&tasks[0], 1);
    let kept = std::fs::read_to_string(&file).unwrap_or_default();
    let named = file.display().to_string();
    let first = rig.kept_prompt(&tasks[0], 2);
    let correction = rig.kept_prompt(&tasks[0], 3);

    assert!(!kept.trim().is_empty(), "{named} is empty or missing");
    assert!(
        first.contains(kept.trim()),
        "the prompt does not hold the text of the file: {first}"
    );
    for prompt in [&first, &correction] {
        assert!(prompt.contains(&named), "the prompt does not name {named}: {prompt}");
    }
}

// Rule: a findings file that cannot be written does not fail the prompt of the fix pass: the pass is given its findings, the
// failure is a warning, and no prompt names a file that is not there.
#[tokio::test]
async fn a_findings_file_that_cannot_be_written_does_not_fail_the_fix_pass() {
    let mut rig = Rig::new(
        &[
            normal("claude-1", "work,fix-work:99"),
            investigator("inv-r1", "review-reasons"),
            investigator("inv-r2", "review-reasons"),
        ],
        1,
        false,
    )
    .await;
    rig.start().await;
    let (_, tasks) = rig.run(&["claude-1"], ONE, MergeMode::Autopilot, None).await;
    // A directory where the file is to be written: the atomic replacement cannot succeed.
    std::fs::create_dir_all(rig.home.fix_pass_findings_file_of(&tasks[0], 1)).unwrap();
    rig.notification_with("The lead's fix response was refused").await;
    rig.stop().await.unwrap();

    let first = rig.kept_prompt(&tasks[0], 2);
    let correction = rig.kept_prompt(&tasks[0], 3);

    assert!(
        first.contains("FINDING"),
        "the pass still carries its findings: {first}"
    );
    for prompt in [&first, &correction] {
        assert!(!prompt.contains("fix-pass-1-findings.md"), "{prompt}");
    }
}

// Golden: the request to correct a response, as the lead reads it: the numbers of the findings it speaks of with the answers
// each allows, and the file that holds their full text. The home and the task are written as `<home>` and `<task>`.
#[tokio::test]
async fn the_request_to_correct_a_response_is_as_its_golden_says() {
    let mut rig = Rig::new(
        &[
            normal("claude-1", "work,fix-work:99"),
            investigator("inv-r1", "review-reasons"),
            investigator("inv-r2", "review-reasons"),
        ],
        1,
        false,
    )
    .await;
    rig.start().await;
    let (_, tasks) = rig.run(&["claude-1"], ONE, MergeMode::Autopilot, None).await;
    rig.notification_with("The lead's fix response was refused").await;
    rig.stop().await.unwrap();

    let prompt = rig.kept_prompt(&tasks[0], 3);
    let start = prompt
        .find("# CORRECT YOUR FIX RESPONSE")
        .expect("the request to correct a response");
    // The harness's own closing instructions follow the request; they are not a part of it.
    let end = prompt
        .find("# YOUR PART OF THE PULL REQUEST DESCRIPTION")
        .unwrap_or(prompt.len());
    let request = prompt
        .get(start..end)
        .unwrap()
        .trim_end()
        .replace(&rig.home.root().display().to_string(), "<home>")
        .replace(tasks[0].as_str(), "<task>");

    super::history_stories_tests::golden("fix-pass-correction.txt", &request);
}

/// A rig whose solo agent runs `script` and whose reviewers ask for changes (one fix pass).
async fn changes_requested(script: &'static str) -> Rig {
    Rig::new(
        &[
            normal("claude-1", script),
            investigator("inv-r1", "review-reasons"),
            investigator("inv-r2", "review-reasons"),
        ],
        1,
        false,
    )
    .await
}

// Rule: a lead run that ended just before the verdict is not judged: its message answers findings it never had. The fix
// pass starts with its findings, with no request to correct a response and no bounce.
#[tokio::test]
async fn a_lead_run_that_ended_just_before_the_verdict_is_not_judged() {
    let mut rig = changes_requested("work,fix-held:1").await;
    rig.start().await;
    let (team, tasks) = rig.run(&["claude-1"], ONE, MergeMode::Autopilot, None).await;
    let lead = rig.worktree(&tasks[0]);
    rig.wait_for("the fix pass run to start", || lead.join(".fake-started-2").exists())
        .await;

    let state = rig.state();
    let first = rig.kept_prompt(&tasks[0], 2);
    let runs = rig.kept_runs(&tasks[0]);
    std::fs::write(lead.join(".fake-release-2"), "").unwrap();
    rig.stop().await.unwrap();

    let bounces = state.team(&team).unwrap().items()[0]
        .findings()
        .and_then(agent_core::state::Findings::disputes)
        .map_or(0, |disputes| disputes.bounces);
    assert_eq!(runs, 2, "no run was sent back before the fix pass began");
    assert_eq!(bounces, 0);
    assert!(first.contains("THE REVIEWERS ASKED FOR CHANGES"), "{first}");
    assert!(!first.contains("# CORRECT YOUR FIX RESPONSE"), "{first}");
}

// Rule: a restart of the daemon between the verdict and the end of the lead's run delivers the findings once: the run that
// goes on after it has them once, and the file still holds them.
#[tokio::test]
async fn a_restart_between_the_verdict_and_the_lead_run_delivers_the_findings_once() {
    let mut rig = changes_requested("work,fix-held:1,answer:1").await;
    rig.start().await;
    let (_, tasks) = rig.run(&["claude-1"], ONE, MergeMode::Autopilot, None).await;
    let lead = rig.worktree(&tasks[0]);
    rig.wait_for("the fix pass run to start", || lead.join(".fake-started-2").exists())
        .await;
    let file = rig.home.fix_pass_findings_file_of(&tasks[0], 1);
    let kept = std::fs::read_to_string(&file).unwrap_or_default();
    let before = rig.kept_prompt(&tasks[0], 2);

    rig.stop().await.unwrap();
    rig.start().await;
    let after = rig.prompt(&tasks[0], 3).await;
    rig.stop().await.unwrap();

    assert!(!kept.trim().is_empty(), "{} is empty or missing", file.display());
    assert_eq!(before.matches(kept.trim()).count(), 1, "{before}");
    assert_eq!(after.matches(kept.trim()).count(), 1, "{after}");
    assert_eq!(std::fs::read_to_string(&file).unwrap_or_default(), kept);
}

// Rule: a pass begun by an older daemon has no findings file: after a restart the correction that is waiting for the lead
// carries the findings file again, written from the findings the state holds.
#[tokio::test]
async fn a_correction_waiting_across_a_restart_gets_the_findings_file_written_again() {
    let mut rig = changes_requested("work,fix-work:99,fix-held:1,answer:1").await;
    rig.start().await;
    let (_, tasks) = rig.run(&["claude-1"], ONE, MergeMode::Autopilot, None).await;
    let lead = rig.worktree(&tasks[0]);
    rig.wait_for("the corrected run to start", || lead.join(".fake-started-3").exists())
        .await;
    let file = rig.home.fix_pass_findings_file_of(&tasks[0], 1);
    let kept = std::fs::read_to_string(&file).unwrap_or_default();
    // What an older daemon left: a pass with no file.
    std::fs::remove_file(&file).unwrap();

    rig.stop().await.unwrap();
    rig.start().await;
    let after = rig.prompt(&tasks[0], 4).await;
    rig.stop().await.unwrap();

    assert!(after.contains("# CORRECT YOUR FIX RESPONSE"), "{after}");
    assert!(after.contains(&file.display().to_string()), "{after}");
    assert_eq!(std::fs::read_to_string(&file).unwrap_or_default(), kept);
}

// Rule: the findings of each pass are in a file of their own: a pass whose file could not be written cannot name the file of
// the pass before.
#[tokio::test]
async fn each_fix_pass_has_a_findings_file_of_its_own() {
    let rig = changes_requested("work").await;
    let task: agent_core::ids::TaskId = "task-20261122-000000-ab12".parse().unwrap();

    let first = rig.home.fix_pass_findings_file_of(&task, 1);
    let second = rig.home.fix_pass_findings_file_of(&task, 2);

    assert_ne!(first, second);
    assert_eq!(first.parent(), second.parent());
}
