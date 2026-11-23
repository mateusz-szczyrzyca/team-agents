//! Gate completion while the conflict tip is being read must use the completed gate's saved record.

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use agent_core::protocol::Reply;
use agent_core::state::{GateOutcome, GateRecord, GateResult, GateTree};
use rustix::fs::{CWD, Mode, OFlags};
use tokio::io::AsyncWriteExt;
use tokio::sync::oneshot;

use super::{Judgement, Lead};
use crate::daemon::owner::fixture::Daemon;
use crate::daemon::owner::rounds::publish::OwnGate;
use crate::daemon::owner::rounds::rig::git;
use crate::fixtures::{at, repo, task, team_state};

/// Connects only when Git has opened HEAD for reading, without a timing-based delay.
async fn head_reader(path: &Path) -> std::fs::File {
    loop {
        match rustix::fs::open(path, OFlags::WRONLY | OFlags::NONBLOCK, Mode::empty()) {
            Ok(fd) => return fd.into(),
            Err(rustix::io::Errno::NXIO) => tokio::task::yield_now().await,
            Err(error) => panic!("cannot connect to Git's HEAD read: {error}"),
        }
    }
}

#[tokio::test]
async fn a_gate_completed_during_the_git_read_uses_its_new_record() {
    let daemon = Daemon::new(&team_state(), None);
    let (mut owner, _sinks) = daemon.bare_owner().await;
    let lead = Lead {
        repo: repo(),
        task: task(1),
        agent: "claude-1".parse().unwrap(),
        done: true,
    };
    let worktree = daemon.home.worktree_dir(&lead.repo, &lead.task);
    std::fs::create_dir_all(&worktree).unwrap();
    git(&worktree, &["init", "-q", "-b", "main"]);
    git(&worktree, &["commit", "--allow-empty", "-q", "-m", "Approved tip"]);
    let approved = git(&worktree, &["rev-parse", "HEAD"]).parse().unwrap();
    git(&worktree, &["update-ref", "refs/remotes/origin/main", "HEAD"]);
    git(&worktree, &["commit", "--allow-empty", "-q", "-m", "Resolved tip"]);
    let tip = git(&worktree, &["rev-parse", "HEAD"]).parse().unwrap();
    let record = GateRecord {
        tree: GateTree::Clean,
        result: GateResult {
            head: tip,
            finished_at: at(2),
            outcome: GateOutcome::Passed,
            log: daemon.home.root().join("gate.log").to_str().unwrap().parse().unwrap(),
        },
    };
    let (completed, answer) = oneshot::channel();
    owner.watch.own_gates.insert(
        lead.task.clone(),
        OwnGate::Running {
            tip: record.result.head.clone(),
            retry: false,
            answer,
        },
    );
    let store = Arc::clone(&owner.parts.store);
    let head_path = worktree.join(".git/HEAD");
    let head = std::fs::read(&head_path).unwrap();
    std::fs::remove_file(&head_path).unwrap();
    rustix::fs::mkfifoat(CWD, &head_path, Mode::RUSR | Mode::WUSR).unwrap();

    let completion = async {
        let mut reader = tokio::fs::File::from_std(head_reader(&head_path).await);
        // The gate service saves the record before replying. Git stays blocked until both effects are visible.
        store.save_gate_record(&lead.task, &record).await.unwrap();
        completed
            .send(Ok(Reply::GateRecorded { record: record.clone() }))
            .unwrap();
        let replacement = worktree.join(".git/HEAD.ready");
        tokio::fs::write(&replacement, &head).await.unwrap();
        tokio::fs::rename(&replacement, &head_path).await.unwrap();
        reader.write_all(&head).await.unwrap();
        reader.flush().await.unwrap();
    };
    let (judgement, ()) = tokio::time::timeout(Duration::from_secs(30), async {
        tokio::join!(owner.judge_tip(&lead, &approved), completion)
    })
    .await
    .expect("the gate completion and Git read must finish");

    assert!(matches!(judgement.unwrap(), Judgement::Tip(head) if head == record.result.head));
}
