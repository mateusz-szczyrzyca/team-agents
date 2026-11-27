//! What the owner records when a lead's final message returns work, on states the fake-CLI tests cannot reach: an item the
//! member holds after another before its first run, a hand-over returned again by a later lead run, and a part that is
//! not the open one. The owner is not running; `read_returns` is called as it is when a lead's run ends.

use std::num::NonZeroU32;

use agent_core::ids::{ItemId, RoundId};
use agent_core::state::split::{PartDefinition, SplitPlan};
use agent_core::state::{ItemPhase, NewRun, RunEnd, State};
use agent_core::status::RunOutcome;

use crate::daemon::owner::fixture::Daemon;
use crate::fixtures::{at, run, task, team_id, team_state};

/// The round of item `n` of these tests.
fn round(n: u32) -> RoundId {
    format!("round-20261111-1530{n:02}-q1w2").parse().unwrap()
}

/// The item `text`.
fn item(text: &str) -> ItemId {
    text.parse().unwrap()
}

/// A team whose member `claude-1` (task 2) holds items 1.1 (round 1) and 1.2 (round 2) in progress, and whose lead (task 1)
/// has a run that began at second `lead_started`. The member's runs are `runs`: for each, its number, its round, when it
/// began and when it ended `DONE`.
fn state_with(runs: &[(u32, u32, u32, u32)], lead_started: u32) -> State {
    let mut state = team_state();
    {
        let mut team = state.team_mut(&team_id(0)).unwrap();
        for (number, text) in [(1, "1.1"), (2, "1.2")] {
            team.open_round(round(number), Some(item(text)), at(1)).unwrap();
            team.item_mut(item(text))
                .unwrap()
                .advance(ItemPhase::InProgress {
                    assignee: "claude-1".parse().unwrap(),
                    branch: format!("agents/claude-1/item-{number}").parse().unwrap(),
                })
                .unwrap();
        }
    }
    for (number, round_number, started, ended) in runs {
        state
            .begin_run(
                &task(2),
                NewRun {
                    id: run(*number),
                    at: at(*started),
                    session: None,
                    rounds: vec![round(*round_number)],
                },
            )
            .unwrap();
        state
            .find_task_mut(&task(2))
            .unwrap()
            .1
            .end_run(
                at(*ended),
                RunEnd::Reported {
                    outcome: RunOutcome::Done,
                },
            )
            .unwrap();
    }
    state
        .begin_run(
            &task(1),
            NewRun {
                id: run(50),
                at: at(lead_started),
                session: None,
                rounds: Vec::new(),
            },
        )
        .unwrap();
    state
}

/// Runs the owner's reading of the final message `message` of the lead of `state`'s team.
async fn returned_by_the_lead(state: &State, message: &str) -> State {
    let daemon = Daemon::new(state, None);
    let (mut owner, _sinks) = daemon.bare_owner().await;
    let located = owner.locate(&task(1)).unwrap();

    owner
        .read_returns(&task(1), &located, RunOutcome::Sleeping, message)
        .await
        .unwrap();

    owner.parts.state.clone()
}

/// How many rejections the state keeps for `text`.
fn recorded(state: &State, text: &str) -> usize {
    state
        .team(&team_id(0))
        .unwrap()
        .item(item(text))
        .unwrap()
        .rejections()
        .map_or(0, <[agent_core::state::Rejection]>::len)
}

#[tokio::test]
async fn an_item_the_member_holds_after_another_is_not_returned_before_its_own_first_run() {
    // Item 1.1 was handed over in round 1; item 1.2 was given to the same member in round 2, which has no run yet.
    let state = state_with(&[(1, 1, 10, 11)], 30);

    let after = returned_by_the_lead(&state, "RETURNED: 1.2\nRETURNED: 1.1\nSTATUS: SLEEPING\n").await;

    assert_eq!(recorded(&after, "1.2"), 0, "no work was handed over for item 1.2");
    assert_eq!(recorded(&after, "1.1"), 1, "the work of item 1.1 was");
}

#[tokio::test]
async fn a_hand_over_returned_again_by_a_later_lead_run_counts_once() {
    // Done at 11, sent back at 15 and counted when the member began again at 20, which ended DONE at 21; a lead run that
    // began at 16 repeats its line.
    let mut state = state_with(&[(1, 1, 10, 11), (2, 1, 20, 21)], 16);
    {
        let mut team = state.team_mut(&team_id(0)).unwrap();
        let mut progress = team.item_mut(item("1.1")).unwrap();
        progress
            .record_rejection_of("fix it".to_owned(), at(15), None, Some(run(1)))
            .unwrap();
        progress.next_run_started(NonZeroU32::new(2).unwrap(), at(20)).unwrap();
    }

    let after = returned_by_the_lead(&state, "RETURNED: 1.1\nSTATUS: SLEEPING\n").await;

    assert_eq!(recorded(&after, "1.1"), 1, "the same hand-over is one rejection");
}

#[tokio::test]
async fn a_line_that_names_a_part_that_is_not_open_counts_nothing() {
    let mut state = state_with(&[(1, 1, 10, 11)], 30);
    let plan = SplitPlan::new(
        item("1.1"),
        ["First", "Second"]
            .into_iter()
            .map(|title| PartDefinition {
                title: title.into(),
                delivers: "A document".into(),
                checks: "Read it".into(),
                depends_on: Vec::new(),
            })
            .collect(),
    )
    .unwrap();
    state.team_mut(&team_id(0)).unwrap().activate_split(plan).unwrap();

    let after = returned_by_the_lead(&state, "RETURNED: 1.1b\nSTATUS: SLEEPING\n").await;
    assert_eq!(recorded(&after, "1.1"), 0, "part b is not the part that is open");

    let after = returned_by_the_lead(&state, "RETURNED: 1.1a\nSTATUS: SLEEPING\n").await;
    assert_eq!(recorded(&after, "1.1"), 1, "part a is");
}

/// A team whose current coordinator and member have replaced their original agents.
fn team_replace_state() -> State {
    let mut state = state_with(&[(1, 1, 10, 11)], 30);
    state
        .find_task_mut(&task(1))
        .unwrap()
        .1
        .end_run(
            at(31),
            RunEnd::Reported {
                outcome: RunOutcome::Sleeping,
            },
        )
        .unwrap();
    state
        .hand_over_task(
            &task(1),
            crate::fixtures::investigator("inv-new"),
            agent_core::state::Choice::Harness,
            at(32),
        )
        .unwrap();
    state
        .hand_over_task(
            &task(2),
            crate::fixtures::normal("claude-2"),
            agent_core::state::Choice::Harness,
            at(32),
        )
        .unwrap();
    state
        .team_mut(&team_id(0))
        .unwrap()
        .item_mut(item("1.1"))
        .unwrap()
        .reassign("claude-2".parse().unwrap(), "agents/claude-1/item-1".parse().unwrap())
        .unwrap();
    state
        .begin_run(
            &task(1),
            NewRun {
                id: run(51),
                at: at(40),
                session: None,
                rounds: Vec::new(),
            },
        )
        .unwrap();
    state
}

/// Installs the roster after both replacements have completed on the real bus.
async fn team_replace_bus(owner: &super::super::Owner) {
    use agent_runtime::agentbus::{BusTier, MemberState, NewMember, NewTeam};
    let members = [
        ("inv-new", task(1), BusTier::Normal),
        ("claude-2", task(2), BusTier::Normal),
    ]
    .into_iter()
    .map(|(name, task, tier)| {
        (
            name.parse().unwrap(),
            NewMember {
                branch: "agents/claude-1/item-1".parse().unwrap(),
                worktree: owner.parts.store.home().root().to_str().unwrap().parse().unwrap(),
                task,
                tier,
                state: MemberState::Sleeping,
                provider: None,
            },
        )
    })
    .collect();
    owner
        .parts
        .bus
        .team_create(&NewTeam {
            id: team_id(0),
            repo: crate::fixtures::repo(),
            lead: Some("inv-new".parse().unwrap()),
            members,
        })
        .await
        .unwrap();
}

#[tokio::test]
async fn team_replace_member_end_notice_reaches_the_replacement_coordinator() {
    use crate::daemon::team::{BusJob, notice_for};
    use agent_core::kinds::Role;
    use agent_runtime::agentbus::Reader;
    let state = team_replace_state();
    let daemon = Daemon::new(&state, None);
    let (owner, _sinks) = daemon.bare_owner().await;
    team_replace_bus(&owner).await;
    let found = state.find_task(&task(2)).unwrap();
    let told = notice_for(
        state.team(&team_id(0)).unwrap(),
        Role::Member,
        found.member,
        &RunEnd::Reported {
            outcome: RunOutcome::Done,
        },
        None,
    )
    .unwrap()
    .unwrap();
    owner
        .carry_out(&super::super::teams::Outbound {
            team: team_id(0),
            job: BusJob::Tell {
                to: told.to,
                notice: told.notice,
            },
        })
        .await
        .unwrap()
        .unwrap();
    let lead = "inv-new".parse().unwrap();
    let team = team_id(0);
    let lead_task = task(1);
    let messages = owner
        .parts
        .bus
        .said_after(
            Reader {
                agent: &lead,
                team: &team,
                task: &lead_task,
            },
            "agentbus",
            None,
        )
        .await
        .unwrap();
    assert_eq!(messages.len(), 1);
    assert!(messages[0].text.contains("claude-2"), "{messages:?}");
}

#[tokio::test]
async fn team_replace_return_reason_reads_the_current_lead_and_member_names() {
    use agent_core::kinds::Role;
    use agent_runtime::agentbus::{Outgoing, Recipient};
    let state = team_replace_state();
    let daemon = Daemon::new(&state, None);
    let (mut owner, _sinks) = daemon.bare_owner().await;
    team_replace_bus(&owner).await;
    let from = "inv-new".parse().unwrap();
    let to = Recipient::Member {
        team: team_id(0),
        agent: "claude-2".parse().unwrap(),
    };
    owner
        .parts
        .bus
        .send(
            &from,
            &to,
            &Outgoing::Chat {
                text: "Earlier message".to_owned(),
            },
        )
        .await
        .unwrap();
    owner.mark_lead_run(&task(1), Role::Coordinator).await;
    let reason = "Please repair the café entry";
    owner
        .parts
        .bus
        .send(
            &from,
            &to,
            &Outgoing::Chat {
                text: reason.to_owned(),
            },
        )
        .await
        .unwrap();
    let located = owner.locate(&task(1)).unwrap();
    owner
        .read_returns(
            &task(1),
            &located,
            RunOutcome::Sleeping,
            "RETURNED: 1.1\nSTATUS: SLEEPING\n",
        )
        .await
        .unwrap();
    let state = &owner.parts.state;
    let rejections = state
        .team(&team_id(0))
        .unwrap()
        .item(item("1.1"))
        .unwrap()
        .rejections()
        .unwrap();
    assert_eq!(rejections.len(), 1);
    assert_eq!(rejections[0].reason(), reason);
}
