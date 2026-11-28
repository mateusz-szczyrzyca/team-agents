//! No worker of a paused account is given a request: not by the team's workers, not from the pool, on the first try, on
//! the second try or when a request is recovered. The pause is written into the state before the owner is built.

use std::os::unix::fs::PermissionsExt;
use std::path::Path;

use agent_core::config::AccountName;
use agent_core::decide::limit::Limit;
use agent_core::state::{AccountPause, Crew, Member, MergeMode, State, TaskListText, Team, TeamMembers};
use agent_core::status::TaskStatus;
use agent_runtime::agentbus::RequestId;
use agent_runtime::worker_request::Pending;

use super::Elsewhere;
use crate::daemon::owner::fixture::{Daemon, pair_state};
use crate::fixtures::{at, normal, repo, task, team_id, worker};

/// A moment so far ahead that no test reaches it.
const FAR: &str = "2100-01-01T00:00:00Z";

/// Pauses `account` in `state` until [`FAR`].
fn pause(state: &mut State, account: &str) {
    state.record_account_pause(
        account.parse::<AccountName>().unwrap(),
        Some(AccountPause {
            until: FAR.parse().unwrap(),
            reason: Limit::RateLimit,
        }),
    );
}

/// A bus whose queue holds request 1 of `claude-1`'s task 1, and that writes the arguments of every claim to `claims`
/// beside itself.
fn bus_with_one_request(dir: &Path) -> std::path::PathBuf {
    let bus = dir.join("bus");
    std::fs::write(
        &bus,
        r#"#!/bin/sh
case "$*" in
 *helper-list*) printf '%s\n' '[{"id":1,"from":"claude-1","from_task":"task-20261122-120000-aaaa","repo":"team-agents","branch":"main","status":"queued"}]' ;;
 *helper-next*) echo "$*" >> "$(dirname "$0")/claims" ;;
 *) exit 1 ;;
esac
"#,
    )
    .unwrap();
    std::fs::set_permissions(&bus, std::fs::Permissions::from_mode(0o755)).unwrap();
    bus
}

/// Adds `agents` (a table text) to the configuration of `daemon` before its `[tiers]`, and `workers` to the tiers.
fn add_pool(daemon: &Daemon, agents: &str, workers: &str) {
    let config = std::fs::read_to_string(daemon.home.config_file()).unwrap();
    std::fs::write(
        daemon.home.config_file(),
        config.replace("[tiers]", &format!("{agents}\n[tiers]\nworkers = {workers}")),
    )
    .unwrap();
}

/// The pool `glm-1` (account `glm-paused`) and `glm-2`, which have the account's owner or none.
const POOL: &str = "[agents.glm-1]\ncli = \"claude\"\naccount = \"glm-paused\"\n[agents.glm-2]\ncli = \"claude\"\n\
                    [accounts.glm-paused]\nowner = \"glm\"\n";

// Rule: the first try of a request goes to the pool worker of an account that is not paused.
#[tokio::test]
async fn the_first_try_of_a_request_skips_a_pool_worker_of_a_paused_account() {
    let scripts = tempfile::tempdir().unwrap();
    let bus = bus_with_one_request(scripts.path());
    let mut state = pair_state();
    pause(&mut state, "glm-paused");
    let daemon = Daemon::new(&state, Some(&bus));
    add_pool(&daemon, POOL, "[\"glm-1\", \"glm-2\"]");
    let (mut owner, _sinks) = daemon.bare_owner().await;

    owner.serve_pool_requests().await.unwrap();

    let claims = std::fs::read_to_string(scripts.path().join("claims")).unwrap();
    assert!(claims.contains("glm-2"), "{claims}");
    assert!(!claims.contains("glm-1"), "{claims}");
}

/// A request of `claude-1` that `worker` was given and returned for a second try.
fn pending_of(worker: &str) -> Pending {
    Pending {
        request: RequestId::new(std::num::NonZeroU64::MIN),
        team: team_id(0),
        requester: "claude-1".parse().unwrap(),
        requester_task: task(1),
        worker: worker.parse().unwrap(),
        worker_task: task(2),
        summary: "finished".to_owned(),
        run_seconds: 0,
        base: None,
        stage: agent_runtime::worker_request::Stage::Ended,
    }
}

// Rule: the second try of a pool request is not offered to a worker of another provider whose account is paused: the
// request goes again to the worker of the same provider, as when no other provider has a worker.
#[tokio::test]
async fn the_second_try_of_a_request_finds_no_pool_worker_of_a_paused_account() {
    let scripts = tempfile::tempdir().unwrap();
    let bus = bus_with_one_request(scripts.path());
    let mut state = pair_state();
    pause(&mut state, "glm-paused");
    let daemon = Daemon::new(&state, Some(&bus));
    add_pool(
        &daemon,
        "[agents.glm-1]\ncli = \"claude\"\n[agents.glm-2]\ncli = \"opencode\"\nmodel = \"openrouter/free\"\n\
         account = \"glm-paused\"\n[accounts.glm-paused]\nowner = \"glm\"\n",
        "[\"glm-1\", \"glm-2\"]",
    );
    let (owner, _sinks) = daemon.bare_owner().await;

    let elsewhere = owner.elsewhere_of(&pending_of("glm-1")).await;

    assert_eq!(elsewhere, Elsewhere::SameProvider);
}

/// A state with one team: `claude-1` coordinating the workers `w-1` (task 3) and `w-2` (task 4).
fn team_with_workers() -> State {
    let crew = Crew::team(TeamMembers {
        coordinator: Member::new(task(1), normal("claude-1")),
        members: vec![Member::new(task(2), normal("claude-2"))],
        workers: vec![Member::new(task(3), worker("w-1")), Member::new(task(4), worker("w-2"))],
    })
    .unwrap();
    let mut state = State::new();
    state
        .insert_team(Team::new(
            team_id(0),
            repo(),
            at(0),
            MergeMode::Autopilot,
            TaskListText::new("Fix the typo in the README.\n".to_owned()).unwrap(),
            crew,
        ))
        .unwrap();
    for worker_task in [task(3), task(4)] {
        let (_, mut member) = state.find_task_mut(&worker_task).unwrap();
        member.transition(TaskStatus::Running).unwrap();
        member.transition(TaskStatus::Waiting).unwrap();
    }
    state
}

/// The configuration of [`team_with_workers`]: the account `w-paused` of `w-2`.
const TEAM_WORKERS: &str = "[agents.w-1]\ncli = \"claude\"\n[agents.w-2]\ncli = \"opencode\"\nmodel = \"openrouter/free\"\naccount = \"w-paused\"\n\
                            [accounts.w-paused]\nowner = \"w\"\n";

// Rule: a worker of the team whose account is paused takes no request: it is busy for every choice, as a running one.
#[tokio::test]
async fn a_team_worker_of_a_paused_account_is_busy_for_requests() {
    let mut state = team_with_workers();
    pause(&mut state, "w-paused");
    let daemon = Daemon::new(&state, None);
    add_pool(&daemon, TEAM_WORKERS, "[\"w-1\", \"w-2\"]");
    let (owner, _sinks) = daemon.bare_owner().await;

    let workers = owner.workers_of(&team_id(0)).unwrap().unwrap();

    let busy: Vec<&str> = workers.busy.iter().map(agent_core::ids::AgentName::as_str).collect();
    assert_eq!(busy, ["w-2"]);
    assert_eq!(workers.in_order.len(), 2);
}

// Rule: the second try of a request of a team goes again to a worker of the same provider when the only worker of
// another provider has a paused account.
#[tokio::test]
async fn the_second_try_of_a_team_request_finds_no_worker_of_a_paused_account() {
    let scripts = tempfile::tempdir().unwrap();
    let bus = bus_with_one_request(scripts.path());
    let mut state = team_with_workers();
    pause(&mut state, "w-paused");
    let daemon = Daemon::new(&state, Some(&bus));
    add_pool(&daemon, TEAM_WORKERS, "[\"w-1\", \"w-2\"]");
    let (owner, _sinks) = daemon.bare_owner().await;
    let mut pending = pending_of("w-1");
    pending.worker_task = task(3);

    let elsewhere = owner.elsewhere_of(&pending).await;

    assert_eq!(elsewhere, Elsewhere::SameProvider);
}

// Rule: a request that a restart gave back to the queue, with the work of its worker kept, is not recovered on a
// worker whose account is paused, though that worker is the first one of the pool and has no run.
#[tokio::test]
async fn a_retained_request_is_not_recovered_on_a_worker_of_a_paused_account() {
    use agent_runtime::request_workspace::{RequestWorkspace, Stage, WorkspaceRecord, branch};
    use agent_runtime::worker_request::recovery::{SnapshotIdentity, Transition, advance, prepare};

    let scripts = tempfile::tempdir().unwrap();
    let bus = scripts.path().join("bus");
    let list = format!(
        r#"[{{"id":1,"from":"claude-1","from_task":"{}","repo":"team-agents","branch":"main","status":"queued"}}]"#,
        task(1)
    );
    std::fs::write(
        &bus,
        format!(
            "#!/bin/sh\ncase \"$*\" in\n *helper-list*) printf '%s\\n' '{list}' ;;\n *helper-next*) echo \"$*\" >> \"$(dirname \"$0\")/claims\" ;;\n *) exit 1 ;;\nesac\n"
        ),
    )
    .unwrap();
    std::fs::set_permissions(&bus, std::fs::Permissions::from_mode(0o755)).unwrap();
    let mut state = pair_state();
    pause(&mut state, "glm-paused");
    let daemon = Daemon::new(&state, Some(&bus));
    add_pool(&daemon, POOL, "[\"glm-1\", \"glm-2\"]");
    let first: agent_core::ids::AgentName = "glm-1".parse().unwrap();
    let workspace = RequestWorkspace::try_from(WorkspaceRecord {
        request: RequestId::new(std::num::NonZeroU64::MIN),
        requester: "claude-1".parse().unwrap(),
        requester_task: task(1),
        requester_round: "round-20261124-120000-cccc".parse().unwrap(),
        requester_repo: "team-agents".parse().unwrap(),
        worker: first.clone(),
        worker_task: task(2),
        branch: branch(&first, &task(2)).unwrap(),
        base: "0123456789abcdef0123456789abcdef01234567".parse().unwrap(),
        stage: Stage::Working,
    })
    .unwrap();
    let task_dir = daemon.home.task_dir(&task(1));
    std::fs::create_dir_all(&task_dir).unwrap();
    prepare(&task_dir, &workspace).await.unwrap();
    advance(
        &task_dir,
        workspace.record().request,
        Transition::RestartedQueued {
            snapshot: SnapshotIdentity {
                round: workspace.record().requester_round.clone(),
                task: task(2),
                number: std::num::NonZeroU32::MIN,
                commit: workspace.record().base.clone(),
                head: workspace.record().base.clone(),
            },
        },
    )
    .await
    .unwrap();
    let (mut owner, _sinks) = daemon.bare_owner().await;

    owner.serve_pool_requests().await.unwrap();

    let claims = std::fs::read_to_string(scripts.path().join("claims")).unwrap();
    assert!(claims.contains("glm-2"), "{claims}");
    assert!(!claims.contains("glm-1"), "{claims}");
}
