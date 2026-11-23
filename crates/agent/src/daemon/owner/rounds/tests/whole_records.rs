//! The records the fake CLI keeps are whole: a reader that reads `.fake-prompt-N` while its write is still held open
//! sees the whole prompt or no record, never a torn one.

use super::rig::{ONE, Rig, investigator, normal};
use agent_core::state::{ItemPhase, MergeMode};

#[tokio::test]
async fn a_record_read_while_its_write_is_held_open_is_whole() {
    let mut rig = Rig::new(
        &[
            normal("lead", "integrate"),
            investigator("inv-member", "work,held-write,answer:1"),
            investigator(
                "inv-r1",
                "verify:VERDICT: REQUEST_CHANGES\\nFINDINGS\\n1. [MAJOR] TDD-2 src/lib.rs:7 — the case is untested → add coverage\\n   CHECK: read src/lib.rs:7,verify:VERDICT: APPROVE\\nSUMMARY: fixed",
            ),
            investigator("inv-r2", "verify:VERDICT: APPROVE"),
        ],
        5,
        true,
    )
    .await;
    rig.start().await;
    let (id, tasks) = rig
        .run(
            &["lead", "inv-member"],
            &format!("{ONE}\nTier: investigator\n"),
            MergeMode::Autopilot,
            Some("lead"),
        )
        .await;
    // The reviewer asks for a change, so the member's run 2 is the fix pass; `held-write` holds the write of the
    // run's own prompt record open (`.fake-held-2`) until the test creates `.fake-release-2`.
    let worktree = rig.worktree(&tasks[1]);
    rig.wait_for("the member's fix pass to hold its prompt write open", || {
        worktree.join(".fake-held-2").exists()
    })
    .await;
    // Read the record raw while the write window is open: it must be the whole prompt (the one mirrored out of the
    // worktree before the fixture performs its action) or absent — never a torn file. Reading through `prompt` would
    // wait for content and hide the tear.
    let whole = rig.kept_prompt(&tasks[1], 2);
    match std::fs::read_to_string(worktree.join(".fake-prompt-2")) {
        Ok(text) => assert_eq!(text, whole, "the record read while its write was held open is torn"),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => panic!("reading the held record: {error}"),
    }
    std::fs::write(worktree.join(".fake-release-2"), "").unwrap();
    // Once the window closes the fix pass ends; the re-review approves and the item merges.
    let state = rig
        .item_where("1.1", "merge after the held write of the record", |phase| {
            matches!(phase, ItemPhase::Done { .. })
        })
        .await;
    assert_eq!(state.team(&id).unwrap().rounds().len(), 1);
    assert_eq!(state.team(&id).unwrap().items()[0].fix_passes().len(), 1);
    rig.stop().await.unwrap();
}