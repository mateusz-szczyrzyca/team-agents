//! Provider cooldowns apply to the newest run of an investigator across tasks.

use std::collections::BTreeSet;

use agent_core::config::Config;
use agent_core::decide::investigator::free_for_request;
use agent_core::state::{RunEnd, State, Timestamp};
use agent_core::status::TaskStatus;
use anyhow::Context;

fn at(minute: u8) -> anyhow::Result<Timestamp> {
    Ok(format!("2026-11-11T15:{minute:02}:00Z").parse()?)
}

fn after(end: RunEnd) -> anyhow::Result<State> {
    let mut state = State::from_json(include_str!("fixtures/state-before-investigation-records.json"))?;
    let task = "task-20261111-153002-k3x9".parse()?;
    let (_, mut member) = state.find_task_mut(&task).context("fixture task")?;
    member.transition(TaskStatus::Queued)?;
    member.transition(TaskStatus::Running)?;
    member.begin_run("run-20261111-153100-a1b2".parse()?, at(31)?, None)?;
    member.end_run(at(32)?, end)?;
    member.transition(TaskStatus::Failed)?;
    Ok(state)
}

fn config() -> anyhow::Result<Config> {
    Ok(Config::from_toml_str(
        "[investigators.inv-codex]\ncli = \"codex\"\n[limits]\nquota_pause_minutes = 10\nrate_limit_pause_minutes = 10",
    )?)
}

#[test]
fn an_investigator_whose_quota_resets_in_the_future_is_not_free() -> anyhow::Result<()> {
    let state = after(RunEnd::QuotaExhausted {
        resets_at: Some(at(45)?),
    })?;
    assert_eq!(free_for_request(&state, &config()?, at(44)?), BTreeSet::new());
    Ok(())
}

#[test]
fn an_investigator_is_free_again_after_its_quota_reset() -> anyhow::Result<()> {
    let state = after(RunEnd::QuotaExhausted {
        resets_at: Some(at(45)?),
    })?;
    assert_eq!(
        free_for_request(&state, &config()?, at(45)?),
        ["inv-codex".parse().unwrap()].into()
    );
    Ok(())
}

#[test]
fn unknown_quota_reset_and_rate_limit_use_the_configured_pause() -> anyhow::Result<()> {
    for end in [RunEnd::QuotaExhausted { resets_at: None }, RunEnd::RateLimited] {
        let state = after(end)?;
        assert_eq!(free_for_request(&state, &config()?, at(41)?), BTreeSet::new());
        assert_eq!(
            free_for_request(&state, &config()?, at(42)?),
            ["inv-codex".parse().unwrap()].into()
        );
    }
    Ok(())
}
