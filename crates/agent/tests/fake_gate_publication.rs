//! Gate records from the fake CLI remain complete while their replacement is written.

use std::process::Command;

#[test]
fn the_fake_cli_keeps_the_previous_gate_record_until_the_replacement_is_complete() {
    let directory = tempfile::tempdir().unwrap();
    let worktree = directory.path().join("fixture-task");
    let task = directory.path().join("tasks/fixture-task");
    std::fs::create_dir(&worktree).unwrap();
    std::fs::create_dir_all(&task).unwrap();
    let previous = serde_json::json!({"tree": "clean", "previous": "complete record"});
    std::fs::write(task.join("gate.json"), serde_json::to_vec(&previous).unwrap()).unwrap();

    let fixture = include_str!("fixtures/team-cli.sh");
    let (_, body) = fixture.split_once("record_gate() {\n").unwrap();
    let (body, _) = body.split_once("\n}\n").unwrap();
    // A shell function runs after output redirection opens the destination. Inspecting the record here makes
    // an incomplete publication observable without a concurrent reader or timing assumptions.
    let script = format!(
        r#"
set -euo pipefail
home=$FIXTURE_HOME
git() {{ builtin printf 'fixture-head\n'; }}
printf() {{
  cp "$home/tasks/fixture-task/gate.json" "$home/observed.json"
  builtin printf "$@"
}}
record_gate() {{
{body}
}}
record_gate
"#
    );
    let output = Command::new("bash")
        .args(["-c", &script])
        .env("FIXTURE_HOME", directory.path())
        .current_dir(&worktree)
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");

    let observed = std::fs::read(directory.path().join("observed.json")).unwrap();
    assert_eq!(observed, serde_json::to_vec(&previous).unwrap());
    let saved: serde_json::Value = serde_json::from_slice(&std::fs::read(task.join("gate.json")).unwrap()).unwrap();
    assert_eq!(
        saved,
        serde_json::json!({
            "tree": "clean",
            "result": {
                "head": "fixture-head",
                "finished_at": "2026-11-13T10:00:00Z",
                "outcome": {"kind": "passed"},
                "log": format!("{}/gate-1.log", task.display())
            }
        })
    );
    let remaining: Vec<_> = std::fs::read_dir(&task)
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    assert_eq!(remaining, vec![std::ffi::OsString::from("gate.json")]);
}
