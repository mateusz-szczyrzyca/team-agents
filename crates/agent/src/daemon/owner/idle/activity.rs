//! Reconstructs activity without depending on the daemon's startup time.
use super::super::removal::{absolute, branches_of, exists, internal, tasks_of};
use super::{Hold, IdleWatch, Owner, Team, Timestamp, recorded_activity};
use agent_core::ids::AgentName;
use agent_core::protocol::Failure;
use agent_runtime::git::Revision;
use std::collections::BTreeMap;
use std::str::FromStr;

impl Owner {
    /// All local task branches and worktree HEADs contribute to the durable observation.
    pub(super) async fn observe_idle(&self, team: &Team, now: Timestamp) -> Result<(IdleWatch, Hold), Failure> {
        let hold = self.idle_hold(team, now)?;
        let mut recorded = if hold == Hold::Running {
            now
        } else {
            recorded_activity(team)
        };
        let mut heads = BTreeMap::new();
        let tasks = tasks_of(team);
        let mut branches = branches_of(team)?;
        for member in team
            .crew()
            .roster()
            .into_iter()
            .map(|(_, member)| member)
            .chain(team.reviewers())
        {
            let path = self.parts.store.home().worktree_dir(team.repo(), member.task());
            if !exists(&path.join(".git")).await? {
                continue;
            }
            let agent = AgentName::from_str(member.agent().name()).map_err(|e| internal("invalid Git identity", &e))?;
            let git = self.git_at(&absolute(&path)?, &agent);
            let head = git
                .rev_parse(&Revision::Head)
                .await
                .map_err(|e| internal("cannot read worktree HEAD", &e))?;
            recorded = recorded.max(
                git.commit_time(&Revision::Commit(head.clone()))
                    .await
                    .map_err(|e| internal("cannot read worktree commit time", &e))?,
            );
            heads.insert(format!("worktree/{}", member.task()), head);
            for branch in git
                .local_branches()
                .await
                .map_err(|e| internal("cannot list activity branches", &e))?
            {
                if tasks.iter().any(|task| branch.as_str().contains(task.as_str())) {
                    branches.insert(branch);
                }
            }
            for branch in &branches {
                if let Some(head) = git
                    .branch_tip(branch)
                    .await
                    .map_err(|e| internal("cannot read branch activity", &e))?
                {
                    recorded = recorded.max(
                        git.commit_time(&Revision::Commit(head.clone()))
                            .await
                            .map_err(|e| internal("cannot read branch commit time", &e))?,
                    );
                    heads.insert(format!("branch/{branch}"), head);
                }
            }
        }
        Ok((IdleWatch::observe(team.idle_watch(), recorded, now, heads), hold))
    }
}
