//! Persistence boundaries of merge completion and human delivery.

use std::path::Path;
use std::sync::Arc;

use agent_core::state::RoundLife;
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

#[tokio::test]
async fn merge_completes_delivered_round_of_same_team() {
    let mut rig = Rig::new(
        &[
            normal("claude-1", "work,sleep"),
            investigator("inv-r1", "done:VERDICT: APPROVE\\nSUMMARY: fine"),
            investigator("inv-r2", "done:VERDICT: APPROVE\\nSUMMARY: fine"),
        ],
        5,
        false,
    )
    .await;
    rig.start().await;
    let (team, _) = rig
        .run(
            &["claude-1"],
            crate::daemon::owner::rounds::rig::PARALLEL,
            MergeMode::NoMerge,
            None,
        )
        .await;
    let item2: agent_core::ids::ItemId = "2.1".parse().unwrap();
    let mut state = rig
        .state_where("round for 2.1 is open and 1.1 is approved", |state| {
            let Some(team) = state.team(&team) else { return false };
            let Some(item1) = team.item("1.1".parse().unwrap()) else {
                return false;
            };
            matches!(item1.phase(), ItemPhase::Approved { .. })
                && team
                    .rounds()
                    .iter()
                    .any(|r| r.item() == Some(item2) && matches!(r.life(), RoundLife::Open))
        })
        .await;
    rig.stop().await.unwrap();

    let (round2, pr, now) = {
        let mut team_mut = state.team_mut(&team).unwrap();
        let round = team_mut.rounds().iter().find(|r| r.item() == Some(item2)).unwrap();
        let round2 = round.id().clone();
        let now = Timestamp::from_utc(round.opened_at().as_utc() + time::Duration::seconds(10)).unwrap();
        let item1 = "1.1".parse().unwrap();
        let ItemPhase::Approved { pr, .. } = team_mut.item(item1).unwrap().phase().clone() else {
            panic!("expected approved");
        };
        team_mut.deliver_round(&round2, pr.number).unwrap();
        (round2, pr, now)
    };

    let daemon = Daemon::new(&state, None);
    let (mut owner, _sinks) = daemon.bare_owner().await;
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
    let lead = owner.lead_of(&team).unwrap();
    let about = About {
        site: Site {
            pulls: &pulls,
            team: &team,
            lead: &lead,
        },
        pr: &pr,
        url: "https://github.example.com/octo/example/pull/1",
        now,
    };

    let item1 = "1.1".parse().unwrap();
    owner.complete(&about, &[item1], &mut Look::default()).await.unwrap();

    let team_after = owner.parts.state.team(&team).unwrap();
    let round2_life = team_after.round(&round2).unwrap().life();
    assert!(
        matches!(
            round2_life,
            agent_core::state::RoundLife::Closed {
                how: RoundEnd::Delivered,
                ..
            }
        ),
        "round2 should be closed as Delivered, got: {round2_life:?}"
    );
    let item2_phase = team_after.item(item2).unwrap().phase();
    assert!(
        matches!(
            item2_phase,
            ItemPhase::Done {
                completion: Completion::Delivered { pr: delivered_pr, .. }
            } if *delivered_pr == pr.number
        ),
        "item2 should be Done with Completion::Delivered, got: {item2_phase:?}"
    );
}

#[tokio::test]
async fn pull_request_closed_without_merge_reopens_delivered_round_and_tells_lead() {
    let mut rig = Rig::new(
        &[
            normal("claude-1", "work,sleep"),
            investigator("inv-r1", "done:VERDICT: APPROVE\\nSUMMARY: fine"),
            investigator("inv-r2", "done:VERDICT: APPROVE\\nSUMMARY: fine"),
        ],
        5,
        false,
    )
    .await;
    rig.start().await;
    let (team, _) = rig
        .run(
            &["claude-1"],
            crate::daemon::owner::rounds::rig::PARALLEL,
            MergeMode::NoMerge,
            None,
        )
        .await;
    let item2: agent_core::ids::ItemId = "2.1".parse().unwrap();
    let mut state = rig
        .state_where("round for 2.1 is open and 1.1 is approved", |state| {
            let Some(team) = state.team(&team) else { return false };
            let Some(item1) = team.item("1.1".parse().unwrap()) else {
                return false;
            };
            matches!(item1.phase(), ItemPhase::Approved { .. })
                && team
                    .rounds()
                    .iter()
                    .any(|r| r.item() == Some(item2) && matches!(r.life(), RoundLife::Open))
        })
        .await;
    rig.stop().await.unwrap();

    let (round2, pr, now) = {
        let mut team_mut = state.team_mut(&team).unwrap();
        let round = team_mut.rounds().iter().find(|r| r.item() == Some(item2)).unwrap();
        let round2 = round.id().clone();
        let now = Timestamp::from_utc(round.opened_at().as_utc() + time::Duration::seconds(10)).unwrap();
        let item1 = "1.1".parse().unwrap();
        let ItemPhase::Approved { pr, .. } = team_mut.item(item1).unwrap().phase().clone() else {
            panic!("expected approved");
        };
        team_mut.deliver_round(&round2, pr.number).unwrap();
        (round2, pr, now)
    };

    let daemon = Daemon::new(&state, None);
    let (mut owner, _sinks) = daemon.bare_owner().await;
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
    let lead = owner.lead_of(&team).unwrap();
    let about = About {
        site: Site {
            pulls: &pulls,
            team: &team,
            lead: &lead,
        },
        pr: &pr,
        url: "https://github.example.com/octo/example/pull/1",
        now,
    };

    owner.reopen_delivered_rounds(&about);

    let team_after = owner.parts.state.team(&team).unwrap();
    let round2_life = team_after.round(&round2).unwrap().life();
    assert_eq!(round2_life, &agent_core::state::RoundLife::Open);

    assert!(
        owner.outbox.iter().any(|job| {
            job.team == team
                && matches!(&job.job, crate::daemon::team::BusJob::Message { to, text } if *to == lead.agent && text.contains(&pr.number.to_string()))
        }),
        "lead should be told about closed PR, outbox: {:?}",
        owner.outbox
    );
}
