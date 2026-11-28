//! No agent of a paused account is picked: not an investigator for a review, not a replacement for a task that waits.
//! The pause of the account is written into the state of the home before the daemon starts.

use agent_core::config::AccountName;
use agent_core::decide::limit::Limit;
use agent_core::ids::{AgentName, TaskId};
use agent_core::state::{AccountPause, ItemPhase, MergeMode, State};
use agent_core::status::TaskStatus;
use rand::SeedableRng;

use super::rig::{ONE, Rig, investigator, normal};
use crate::daemon::owner::fixture::Daemon;

/// The account that is paused in these tests.
const PAUSED: &str = "inv-paused";

/// A moment so far ahead that no test reaches it.
const FAR: &str = "2100-01-01T00:00:00Z";

/// The state of a home in which `account` is paused until [`FAR`].
fn state_with_pause(account: &str) -> State {
    let mut state = State::default();
    let account: AccountName = account.parse().unwrap();
    let until = FAR.parse().unwrap();
    state.record_account_pause(
        account,
        Some(AccountPause {
            until,
            reason: Limit::Quota { reset_in: None },
        }),
    );
    state
}

/// The names of the reviewers of the item in review of the only team of `state`, in the order asked; none when no item
/// is in review.
fn reviewer_names(state: &State) -> Vec<String> {
    state
        .teams()
        .first()
        .and_then(|team| {
            team.items().iter().find_map(|item| match item.phase() {
                ItemPhase::InReview { reviews, .. } => Some(
                    reviews
                        .iter()
                        .map(|review| review.reviewer.as_str().to_owned())
                        .collect(),
                ),
                _ => None,
            })
        })
        .unwrap_or_default()
}

// Rule: the review goes to the investigators of accounts that are not paused: `inv-1` would be one of the first two by
// name, but its account is paused, so `inv-2` and `inv-3` review and `inv-1` never runs.
#[tokio::test]
async fn review_goes_to_another_account_while_one_is_paused() {
    let approve = "done:VERDICT: APPROVE\\nSUMMARY: fine";
    let mut rig = Rig::new(
        &[
            normal("claude-1", "work"),
            investigator("inv-1", approve),
            investigator("inv-2", approve),
            investigator("inv-3", approve),
        ],
        1,
        false,
    )
    .await;
    let path = rig.home.config_file();
    let text = std::fs::read_to_string(&path).unwrap();
    let header = "[investigators.inv-1]\n";
    assert!(text.contains(header), "{text}");
    let text = text.replace(header, &format!("{header}account = \"{PAUSED}\"\n"));
    std::fs::write(&path, format!("{text}\n[accounts.{PAUSED}]\nowner = \"inv\"\n")).unwrap();
    std::fs::write(rig.home.state_file(), state_with_pause(PAUSED).to_json().unwrap()).unwrap();
    rig.start_with_rng(rand::rngs::StdRng::seed_from_u64(0)).await;
    let (team, _) = rig.run(&["claude-1"], ONE, MergeMode::Autopilot, None).await;

    let state = rig
        .state_where("the reviewers are asked", |state| reviewer_names(state).len() == 2)
        .await;
    rig.stop().await.unwrap();

    assert_eq!(reviewer_names(&state), ["inv-2", "inv-3"]);
    let skipped = state
        .team(&team)
        .unwrap()
        .reviewers()
        .iter()
        .filter(|member| member.agent().name() == "inv-1")
        .count();
    assert_eq!(skipped, 0, "no task of the paused account's investigator was made");
}

/// A waiting ordinary member in a team, and `claude-2` alone in a team of its own that is done.
fn waiting_member_and_a_free_candidate() -> (State, TaskId) {
    use agent_core::state::{Crew, Member, TaskListText, Team};

    let mut state = crate::fixtures::team_state();
    let waiting = crate::fixtures::task(2);
    state.start_waiting(&waiting, crate::fixtures::at(0)).unwrap();
    let task = crate::fixtures::task(5);
    let team = crate::fixtures::team_id(1);
    state
        .insert_team(Team::new(
            team.clone(),
            crate::fixtures::repo(),
            crate::fixtures::at(0),
            MergeMode::NoMerge,
            TaskListText::new("Solo task.\n".to_owned()).unwrap(),
            Crew::solo(Member::new(task.clone(), crate::fixtures::normal("claude-2"))).unwrap(),
        ))
        .unwrap();
    let mut solo = state.team_mut(&team).unwrap();
    let (_, mut member) = solo.find_mut(&task).unwrap();
    member.transition(TaskStatus::Running).unwrap();
    member.transition(TaskStatus::Done).unwrap();
    (state, waiting)
}

/// Makes `claude-2` of the account `claude-paused`, which is paused until [`FAR`], and a replacement of the daemon.
fn pause_the_account_of_claude_2(daemon: &Daemon) {
    let path = daemon.home.config_file();
    let text = std::fs::read_to_string(&path).unwrap();
    let header = "[agents.claude-2]\ncli = \"claude\"\n";
    assert!(text.contains(header), "{text}");
    let text = text.replace(header, &format!("{header}account = \"claude-paused\"\n"));
    std::fs::write(
        &path,
        format!(
            "{text}\n[accounts.claude-paused]\nowner = \"claude\"\n[replacement]\nagents = [\"claude-2\"]\nbusy_after = 1\n"
        ),
    )
    .unwrap();
}

// Rule: a waiting task is not handed over to an agent whose account is paused, as it is not to one its provider refused.
#[tokio::test]
async fn an_agent_of_a_paused_account_takes_no_waiting_task_over() {
    let (mut state, waiting) = waiting_member_and_a_free_candidate();
    let paused: AccountName = "claude-paused".parse().unwrap();
    state.record_account_pause(
        paused,
        Some(AccountPause {
            until: FAR.parse().unwrap(),
            reason: Limit::RateLimit,
        }),
    );
    let daemon = Daemon::new(&state, None);
    pause_the_account_of_claude_2(&daemon);
    daemon.clock.advance(time::Duration::seconds(2));
    let (mut owner, _sinks) = daemon.bare_owner().await;

    let changed = owner.hand_over_busy_wait(&waiting).await.unwrap();

    assert!(!changed, "an agent of a paused account is not free");
    let holder: AgentName = owner
        .parts
        .state
        .find_task(&waiting)
        .unwrap()
        .member
        .agent()
        .name()
        .parse()
        .unwrap();
    assert_eq!(holder.as_str(), "claude-1");
}
