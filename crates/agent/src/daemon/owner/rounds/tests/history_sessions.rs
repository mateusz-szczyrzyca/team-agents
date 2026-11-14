//! Session IDs first reported by a CLI survive in the history of a merged daemon round.

use agent_core::state::{ItemPhase, MergeMode};
use agent_runtime::history::Reader;

use super::rig::{ONE, Rig, investigator, normal};

#[tokio::test]
async fn a_merged_opencode_round_records_session_changes_once_in_observation_order() {
    let mut rig = Rig::new(
        &[
            normal("opencode-1", "work"),
            investigator("inv-r1", "done:VERDICT: APPROVE\\nSUMMARY: fine"),
            investigator("inv-r2", "done:VERDICT: APPROVE\\nSUMMARY: fine"),
        ],
        5,
        false,
    )
    .await;
    let path = rig.home.config_file();
    let config = std::fs::read_to_string(&path).unwrap();
    let config = config.replace(
        "[agents.opencode-1]\ncli = \"claude\"",
        "[agents.opencode-1]\ncli = \"opencode\"",
    );
    let script = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/history-sessions.sh");
    let config = config.replace(
        "[binaries]\n",
        &format!("[binaries]\nopencode = \"{}\"\n", script.display()),
    );
    std::fs::write(path, config).unwrap();
    rig.start().await;
    let (team, tasks) = rig.run(&["opencode-1"], ONE, MergeMode::Autopilot, None).await;
    let state = rig
        .item_where("1.1", "merged", |phase| matches!(phase, ItemPhase::Done { .. }))
        .await;
    rig.stop().await.unwrap();
    let reader = Reader::open(&rig.home.history_file()).unwrap();
    let runs = reader.runs_of_team(&team).unwrap();
    let run = runs.iter().find(|row| row.start.value.task == tasks[0]).unwrap();
    assert!(
        run.start.value.session.is_none(),
        "new OpenCode sessions are assigned by the CLI"
    );
    let sessions = reader.sessions(&run.start.value.run).unwrap();
    assert_eq!(
        sessions
            .iter()
            .map(|s| (s.number.get(), s.session.as_str()))
            .collect::<Vec<_>>(),
        [(1, "ses-first"), (2, "ses-second")]
    );
    assert!(sessions.windows(2).all(|pair| pair[0].at <= pair[1].at));
    let ended = run.end.as_ref().unwrap().value.at;
    assert!(sessions.iter().all(|s| s.at >= run.start.value.at && s.at <= ended));
    assert_eq!(
        state
            .find_task(&tasks[0])
            .unwrap()
            .member
            .last_session()
            .unwrap()
            .as_str(),
        "ses-second"
    );
    assert!(reader.changes(&run.start.value.run).unwrap().is_some());
}
