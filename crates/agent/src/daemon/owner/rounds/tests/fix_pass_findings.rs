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

    let file = rig.home.fix_pass_findings_file(&tasks[0]);
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
    std::fs::create_dir_all(rig.home.fix_pass_findings_file(&tasks[0])).unwrap();
    rig.notification_with("The lead's fix response was refused").await;
    rig.stop().await.unwrap();

    let first = rig.kept_prompt(&tasks[0], 2);
    let correction = rig.kept_prompt(&tasks[0], 3);

    assert!(
        first.contains("FINDING"),
        "the pass still carries its findings: {first}"
    );
    for prompt in [&first, &correction] {
        assert!(!prompt.contains("fix-pass-findings.md"), "{prompt}");
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
