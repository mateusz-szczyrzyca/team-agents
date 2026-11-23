//! Removal through the socket against the existing fake GitHub and agent CLI.

use agent_core::protocol::{Client, Command, ControlRequest, RemoveRequest, Reply, StatusRequest, Target};
use agent_core::state::MergeMode;
use agent_core::status::TaskStatus;
use agent_runtime::history::Reader;

use super::rig::{ONE, Rig, git, investigator, normal};
use crate::client::Connection;
use crate::daemon::owner::start::work_branch;

#[tokio::test]
async fn removal_notifies_once_closes_the_pull_and_survives_restart_without_losing_history_or_remote_work() {
    let mut rig = Rig::new(
        &[
            normal("claude-1", "clean-sleep"),
            investigator("inv-r1", "done"),
            investigator("inv-r2", "done"),
        ],
        0,
        false,
    )
    .await;
    rig.start().await;
    let (team, tasks) = rig.run(&["claude-1"], ONE, MergeMode::NoMerge, None).await;
    let task = tasks.first().unwrap();
    rig.status_is(task, TaskStatus::Waiting).await;
    Connection::connect(&rig.home.socket_file())
        .await
        .unwrap()
        .request(
            Client::Cli,
            Command::Stop(ControlRequest {
                target: Target::Team(team.clone()),
            }),
        )
        .await
        .unwrap();
    rig.stop().await.unwrap();
    let before = Reader::open(&rig.home.root().join("history.db"))
        .unwrap()
        .runs_of_team(&team)
        .unwrap();
    assert!(!before.is_empty());
    let state = rig.state();
    let found = state.find_task(task).unwrap();
    let branch = work_branch(found.team, found.member).unwrap();
    let worktree = rig.worktree(task);
    std::fs::write(worktree.join("kept.txt"), "unmerged work\n").unwrap();
    git(&worktree, &["add", "kept.txt"]);
    git(&worktree, &["commit", "-q", "-m", "Keep this work"]);
    git(&worktree, &["push", "-q", "origin", branch.as_str()]);
    let remote = rig.remote_tip(branch.as_str());
    let pull = rig.github.foreign_pull(branch.as_str());
    let older_notifications = rig.notifications();
    rig.start().await;
    let reply = Connection::connect(&rig.home.socket_file())
        .await
        .unwrap()
        .request(
            Client::Cli,
            Command::Remove(RemoveRequest {
                team: team.clone(),
                force: false,
            }),
        )
        .await
        .unwrap();
    let Reply::Removed { removal } = reply else {
        panic!("expected removal, got {reply:?}")
    };
    assert_eq!(
        removal
            .closed_pull_requests
            .iter()
            .map(|pr| pr.get().get())
            .collect::<Vec<_>>(),
        vec![pull]
    );
    assert!(rig.github.pulls().iter().find(|pr| pr.number == pull).unwrap().closed);
    assert!(!rig.github.pulls().iter().find(|pr| pr.number == pull).unwrap().merged);
    let comments = rig.github.comments(pull);
    assert_eq!(comments.len(), 1);
    assert!(comments[0].contains(&team.to_string()));
    assert!(comments[0].contains("removed"));
    assert_eq!(rig.remote_tip(branch.as_str()), remote);
    let bundle = removal.bundle.as_ref().unwrap();
    git(
        &rig.repo,
        &["fetch", bundle.as_path().to_str().unwrap(), branch.as_str()],
    );
    assert_eq!(git(&rig.repo, &["rev-parse", "FETCH_HEAD"]), remote);
    assert!(!worktree.exists());
    assert!(!rig.home.task_dir(task).exists());
    let notices = rig.notifications();
    assert!(older_notifications.iter().all(|old| notices.contains(old)));
    let removal_notices: Vec<_> = notices.iter().filter(|text| text.contains("Team removed")).collect();
    assert_eq!(removal_notices.len(), 1);
    let notice = removal_notices[0];
    for text in [
        team.to_string(),
        "requested by the human".to_owned(),
        bundle.as_path().display().to_string(),
        removal.freed_bytes.to_string(),
    ] {
        assert!(notice.contains(&text), "missing {text:?} in {notice:?}");
    }
    for restart in [false, true] {
        if restart {
            rig.stop().await.unwrap();
            rig.start().await;
        }
        let status = Connection::connect(&rig.home.socket_file())
            .await
            .unwrap()
            .request(Client::Cli, Command::Status(StatusRequest { target: None }))
            .await
            .unwrap();
        assert_eq!(status, Reply::Teams { teams: vec![] });
        assert!(rig.state().find_task(task).is_none());
    }
    rig.stop().await.unwrap();
    let after = Reader::open(&rig.home.root().join("history.db"))
        .unwrap()
        .runs_of_team(&team)
        .unwrap();
    assert_eq!(after, before);
}
