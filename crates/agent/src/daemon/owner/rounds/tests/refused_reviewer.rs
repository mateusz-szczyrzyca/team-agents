//! A reviewer its provider refuses, as a whole process: the daemon, the fake CLIs and investigators of another provider
//! (issues #234 and #233). The reviewers of the round are `inv-1` and `inv-2`, the first two by name; `inv-3` is a Claude
//! investigator like `inv-1`, and `inv-4` an `OpenCode` one that is not in the pass.

use std::path::Path;

use agent_core::ids::TaskId;
use agent_core::state::{ItemPhase, MergeMode, Review, State};
use agent_core::status::TaskStatus;

use super::rig::{ONE, Rig, investigator, normal};

/// Makes the investigators `names` of `rig` `OpenCode` ones: their runs go to the same stand-in CLI, and their provider is
/// `opencode`, which no Claude investigator shares.
fn with_opencode_investigators(rig: &Rig, names: &[&str]) {
    let path = rig.home.config_file();
    let mut config = std::fs::read_to_string(&path).unwrap();
    for name in names {
        let claude = format!("[investigators.{name}]\ncli = \"claude\"");
        assert!(config.contains(&claude), "{config}");
        config = config.replace(&claude, &format!("[investigators.{name}]\ncli = \"opencode\""));
    }
    let stand_in = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/team-cli.sh");
    let config = config.replace(
        "[binaries]\n",
        &format!("[binaries]\nopencode = \"{}\"\n", stand_in.display()),
    );
    std::fs::write(path, config).unwrap();
}

/// Makes the patience of a reviewer and a quota pause far longer than the test: the refusal rule alone changes the
/// reviewer, and the pause lasts until the test moves the clock on.
fn slow_patience_and_long_quota_pause(rig: &Rig) {
    let path = rig.home.config_file();
    let config = std::fs::read_to_string(&path).unwrap();
    assert!(config.contains("[review]\n"), "{config}");
    let config = config.replace("[review]\n", "[review]\nreplace_after_minutes = 100000\n");
    std::fs::write(path, format!("{config}[limits]\nquota_pause_minutes = 600\n")).unwrap();
}

/// The reviews of the item in review of the only team of `state`; none when no item is in review.
fn reviews(state: &State) -> Vec<Review> {
    state
        .teams()
        .first()
        .and_then(|team| {
            team.items().iter().find_map(|item| match item.phase() {
                ItemPhase::InReview { reviews, .. } => Some(reviews.clone()),
                _ => None,
            })
        })
        .unwrap_or_default()
}

/// The names of the reviewers of the review pass in `state`, in the order asked.
fn reviewer_names(state: &State) -> Vec<String> {
    reviews(state)
        .iter()
        .map(|review| review.reviewer.as_str().to_owned())
        .collect()
}

/// The task of the review that `agent` gives in `state`, when it gives one.
fn review_task_of(state: &State, agent: &str) -> Option<TaskId> {
    reviews(state)
        .into_iter()
        .find(|review| review.reviewer.as_str() == agent)
        .map(|review| review.task)
}

/// Whether the reviewer `agent` of the only team of `state` is paused by its provider.
fn reviewer_paused(state: &State, agent: &str) -> bool {
    state.teams().first().is_some_and(|team| {
        team.reviewers()
            .iter()
            .any(|member| member.status() == TaskStatus::Paused && member.agent().name() == agent)
    })
}

/// Whether GitHub got a review of the pull request whose body names `agent` as its reviewer.
fn review_posted_by(rig: &Rig, agent: &str) -> bool {
    let needle = format!("### Review by `{agent}`");
    rig.github
        .requests()
        .iter()
        .any(|seen| seen.method == "POST" && seen.target.ends_with("/reviews") && seen.body.contains(&needle))
}

#[tokio::test]
async fn a_reviewer_refused_on_every_run_is_replaced_by_an_investigator_of_another_provider() {
    let mut rig = Rig::new(
        &[
            normal("claude-1", "work"),
            investigator("inv-1", "turn-limit-quota"),
            investigator("inv-2", "opencode-verdict:APPROVE"),
            investigator("inv-3", "done:VERDICT: APPROVE\\nSUMMARY: fine"),
            investigator("inv-4", "opencode-verdict:APPROVE"),
        ],
        1,
        false,
    )
    .await;
    with_opencode_investigators(&rig, &["inv-2", "inv-4"]);
    slow_patience_and_long_quota_pause(&rig);
    rig.start().await;
    rig.run(&["claude-1"], ONE, MergeMode::Autopilot, None).await;

    let paused = rig
        .state_where("the reviewer inv-1 paused by its quota", |state| {
            reviewer_paused(state, "inv-1")
        })
        .await;
    let refused = review_task_of(&paused, "inv-1").unwrap();
    assert_eq!(rig.runs(&refused), 1);

    // The pause ends with the clock: the retry is refused too, and the refusal rule changes the agent of the review.
    rig.advance_history_days(1);
    let replaced = rig
        .state_where("the review of inv-1 given to inv-4", |state| {
            let names = reviewer_names(state);
            names.iter().any(|name| name == "inv-4") && !names.iter().any(|name| name == "inv-1")
        })
        .await;
    assert_eq!(rig.runs(&refused), 2, "one retry, then the change of agent");
    assert!(!reviewer_names(&replaced).iter().any(|name| name == "inv-3"));
    let replacement = review_task_of(&replaced, "inv-4").unwrap();

    let merged = rig
        .item_where("1.1", "merged", |phase| matches!(phase, ItemPhase::Done { .. }))
        .await;
    rig.stop().await.unwrap();
    assert_eq!(rig.runs(&replacement), 1, "the replacement reviews once");
    assert!(
        merged.teams()[0]
            .items()
            .iter()
            .all(|item| matches!(item.phase(), ItemPhase::Done { .. }))
    );
}

#[tokio::test]
async fn a_reviewer_refused_once_keeps_its_review_after_a_good_retry() {
    let mut rig = Rig::new(
        &[
            normal("claude-1", "work"),
            investigator("inv-1", "turn-limit-quota,done:VERDICT: APPROVE\\nSUMMARY: fine"),
            investigator("inv-2", "opencode-verdict:APPROVE"),
            investigator("inv-3", "done:VERDICT: APPROVE\\nSUMMARY: fine"),
            investigator("inv-4", "opencode-verdict:APPROVE"),
        ],
        1,
        false,
    )
    .await;
    with_opencode_investigators(&rig, &["inv-2", "inv-4"]);
    slow_patience_and_long_quota_pause(&rig);
    rig.start().await;
    rig.run(&["claude-1"], ONE, MergeMode::Autopilot, None).await;

    let paused = rig
        .state_where("the reviewer inv-1 paused by its quota", |state| {
            reviewer_paused(state, "inv-1")
        })
        .await;
    assert!(
        reviewer_names(&paused).contains(&"inv-1".to_owned()),
        "inv-1 must be among the reviewers asked"
    );
    let task = review_task_of(&paused, "inv-1").unwrap();
    rig.advance_history_days(1);

    let merged = rig
        .item_where("1.1", "merged", |phase| matches!(phase, ItemPhase::Done { .. }))
        .await;
    rig.stop().await.unwrap();
    assert_eq!(rig.runs(&task), 2, "one refusal, then the good retry");
    assert!(
        review_posted_by(&rig, "inv-1"),
        "the retained reviewer's review is posted"
    );
    // An investigator of another provider is not asked because inv-1 keeps its review.
    let same_provider_names: Vec<_> = reviewer_names(&merged)
        .into_iter()
        .filter(|n| n != "inv-1")
        .collect();
    assert!(
        same_provider_names.iter().all(|n| !n.starts_with("inv-4")),
        "no investigator of another provider is asked"
    );
    assert!(
        merged.teams()[0]
            .items()
            .iter()
            .all(|item| matches!(item.phase(), ItemPhase::Done { .. }))
    );
}

#[tokio::test]
async fn a_refused_reviewer_waits_and_the_human_is_told_once_when_no_investigator_of_another_provider_is_free() {
    let mut rig = Rig::new(
        &[
            normal("claude-1", "work"),
            investigator("inv-1", "turn-limit-quota"),
            investigator("inv-2", "opencode-verdict:APPROVE"),
            investigator("inv-3", "done:VERDICT: APPROVE\\nSUMMARY: fine"),
        ],
        1,
        false,
    )
    .await;
    with_opencode_investigators(&rig, &["inv-2"]);
    slow_patience_and_long_quota_pause(&rig);
    rig.start().await;
    rig.run(&["claude-1"], ONE, MergeMode::Autopilot, None).await;

    let paused = rig
        .state_where("the reviewer inv-1 paused by its quota", |state| {
            reviewer_paused(state, "inv-1")
        })
        .await;
    let refused = review_task_of(&paused, "inv-1").unwrap();
    rig.advance_history_days(1);

    let told = rig.notification_with("inv-1 waits for its provider").await;
    assert!(told.contains("no investigator of another provider is free"), "{told}");
    let state = rig.state();
    assert!(
        reviewer_names(&state).contains(&"inv-1".to_owned()),
        "inv-1 must be among the reviewers"
    );
    assert!(
        !reviewer_names(&state).contains(&"inv-3".to_owned()),
        "inv-3 shares the provider, so it is not asked"
    );
    assert_eq!(rig.runs(&refused), 2, "one retry, then the wait");
    let tellings = rig
        .notifications()
        .iter()
        .filter(|text| text.contains("inv-1 waits for its provider"))
        .count();
    rig.stop().await.unwrap();
    assert_eq!(tellings, 1);
}

// Rule: the human is told once that a refused reviewer waits, also when the daemon restarts while it waits (review of
// #273, finding 2).
#[tokio::test]
async fn a_restart_of_the_daemon_does_not_tell_again_that_a_refused_reviewer_waits() {
    let mut rig = Rig::new(
        &[
            normal("claude-1", "work"),
            investigator("inv-1", "turn-limit-quota"),
            investigator("inv-2", "opencode-verdict:APPROVE"),
            investigator("inv-3", "done:VERDICT: APPROVE\\nSUMMARY: fine"),
        ],
        1,
        false,
    )
    .await;
    with_opencode_investigators(&rig, &["inv-2"]);
    slow_patience_and_long_quota_pause(&rig);
    rig.start().await;
    rig.run(&["claude-1"], ONE, MergeMode::Autopilot, None).await;
    let paused = rig
        .state_where("the reviewer inv-1 paused by its quota", |state| {
            reviewer_paused(state, "inv-1")
        })
        .await;
    let refused = review_task_of(&paused, "inv-1").unwrap();
    rig.advance_history_days(1);
    rig.notification_with("inv-1 waits for its provider").await;
    rig.stop().await.unwrap();

    rig.start().await;
    // The pause of the second refusal ends: the third run of inv-1 is refused as well, and the review still waits.
    rig.advance_history_days(1);
    rig.wait_for("the third run of inv-1", || rig.runs(&refused) >= 3).await;
    rig.state_where("the reviewer inv-1 paused again", |state| {
        reviewer_paused(state, "inv-1")
    })
    .await;
    rig.stop().await.unwrap();

    let tellings = rig
        .notifications()
        .iter()
        .filter(|text| text.contains("inv-1 waits for its provider"))
        .count();
    assert_eq!(tellings, 1);
}

#[tokio::test]
async fn the_human_is_told_once_that_a_refused_reviewer_was_replaced() {
    let mut rig = Rig::new(
        &[
            normal("claude-1", "work"),
            investigator("inv-1", "turn-limit-quota"),
            investigator("inv-2", "opencode-verdict:APPROVE"),
            investigator("inv-4", "opencode-verdict:APPROVE"),
        ],
        1,
        false,
    )
    .await;
    with_opencode_investigators(&rig, &["inv-2", "inv-4"]);
    slow_patience_and_long_quota_pause(&rig);
    rig.start().await;
    rig.run(&["claude-1"], ONE, MergeMode::Autopilot, None).await;
    rig.state_where("the reviewer inv-1 paused by its quota", |state| {
        reviewer_paused(state, "inv-1")
    })
    .await;
    rig.advance_history_days(1);

    let told = rig.notification_with("inv-4 reviews pull request").await;
    rig.item_where("1.1", "merged", |phase| matches!(phase, ItemPhase::Done { .. }))
        .await;
    let tellings = rig
        .notifications()
        .iter()
        .filter(|text| text.contains("inv-1 was refused by its provider"))
        .count();
    rig.stop().await.unwrap();
    assert!(told.contains("the provider claude refused inv-1"), "{told}");
    assert_eq!(tellings, 1);
}
