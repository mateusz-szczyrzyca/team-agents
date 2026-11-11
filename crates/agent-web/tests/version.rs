//! The built `agent-web` binary answers `--version`, the check of plan item 1.1.

use std::process::Command;

#[test]
fn the_binary_prints_its_version_and_exits_0() {
    let output = Command::new(env!("CARGO_BIN_EXE_agent-web"))
        .arg("--version")
        .output()
        .unwrap();

    assert!(output.status.success(), "{output:?}");
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        format!("agent-web {}\n", env!("CARGO_PKG_VERSION"))
    );
    assert_eq!(String::from_utf8(output.stderr).unwrap(), "");
}
