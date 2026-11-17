//! Persistence boundaries of merge completion and human delivery.

use std::path::Path;
use std::sync::Arc;

use agent_core::state::{ItemPhase, MergeMode};
use agent_runtime::autopilot_prompts::AutopilotPrompts;
use agent_runtime::checks_prompts::ChecksPrompts;
use agent_runtime::settings::Settings;
use agent_runtime::store::StoreError;
use tokio::sync::mpsc;

use super::*;
use crate::daemon::owner::fixture::Daemon;
use crate::daemon::owner::rounds::rig::{ONE, Rig, investigator, normal};
use crate::fixtures::at;

#[tokio::test]
async fn merge_delivery_waits_for_durable_completion_and_restart_does_not_repeat_it() {
    let mut rig = Rig::new(
        &[
            normal("claude-1", "work"),
            investigator("inv-r1", "done:VERDICT: APPROVE\\nSUMMARY: fine"),
            investigator("inv-r2", "done:VERDICT: APPROVE\\nSUMMARY: fine"),
        ],
        5,
        false,
    )
    .await;
    rig.start().await;
    let (team, _) = rig.run(&["claude-1"], ONE, MergeMode::NoMerge, None).await;
    let state = rig
        .item_where("1.1", "approved", |phase| matches!(phase, ItemPhase::Approved { .. }))
        .await;
    rig.stop().await.unwrap();
    let settings = Arc::new(Settings::load(&rig.home).await.unwrap());
    let prompts = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../prompts");
    let language = settings.config.language();
    let pulls = Pulls::new(
        Arc::new(AutopilotPrompts::load(&prompts, language).await.unwrap()),
        Arc::new(ChecksPrompts::load(&prompts, language).await.unwrap()),
        &settings,
    )
    .unwrap()
    .unwrap();
    let daemon = Daemon::new(&state, None);
    let (mut owner, _sinks) = daemon.bare_owner().await;
    owner.parts.settings = Arc::clone(&settings);
    let (sender, mut deliveries) = mpsc::channel(8);
    owner.parts.notices = sender;
    let item = "1.1".parse().unwrap();
    let ItemPhase::Approved { pr, .. } = state.team(&team).unwrap().item(item).unwrap().phase() else {
        panic!("expected an approved pull request");
    };
    let lead = owner.lead_of(&team).unwrap();
    let about = About {
        site: Site {
            pulls: &pulls,
            team: &team,
            lead: &lead,
        },
        pr,
        url: "https://github.example.com/octo/example/pull/1",
        now: at(1000),
    };
    let path = daemon.home.state_file();
    let saved = std::fs::read(&path).unwrap();
    std::fs::remove_file(&path).unwrap();
    std::fs::create_dir(&path).unwrap();
    let result = async {
        owner.complete(&about, &[item], &mut Look::default()).await?;
        owner.save().await
    }
    .await;
    assert!(matches!(result, Err(OwnerError::Store(StoreError::Replace { path: found, .. })) if found == path));
    assert!(matches!(deliveries.try_recv(), Err(mpsc::error::TryRecvError::Empty)));
    drop(owner);
    std::fs::remove_dir(&path).unwrap();
    std::fs::write(&path, saved).unwrap();

    let (mut restarted, _sinks) = daemon.bare_owner().await;
    restarted.parts.settings = settings;
    let (sender, mut deliveries) = mpsc::channel(8);
    restarted.parts.notices = sender;
    restarted.complete(&about, &[item], &mut Look::default()).await.unwrap();
    let notice = deliveries.try_recv().unwrap();
    assert_eq!(notice.notification.title(), "🟣 example");
    let persisted = restarted.parts.store.load().await.unwrap();
    assert!(matches!(
        persisted.team(&team).unwrap().item(item).unwrap().phase(),
        ItemPhase::Done { .. }
    ));
    drop(restarted);
    let (mut restarted, _sinks) = daemon.bare_owner().await;
    let (sender, mut deliveries) = mpsc::channel(8);
    restarted.parts.notices = sender;
    restarted
        .merge_approved(&pulls, &team, &mut Look::default())
        .await
        .unwrap();
    assert!(matches!(deliveries.try_recv(), Err(mpsc::error::TryRecvError::Empty)));
}
