//! Reduced debug information must retain source locations in failing-test backtraces.

use std::process::Command;

#[test]
fn panic_backtrace_retains_file_and_line() {
    assert!(
        std::env::var_os("AGENT_BACKTRACE_PROBE").is_none(),
        "intentional backtrace probe"
    );
    let output = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "panic_backtrace_retains_file_and_line", "--nocapture"])
        .env("AGENT_BACKTRACE_PROBE", "1")
        .env("RUST_BACKTRACE", "1")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(101));
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("stack backtrace:"), "{stderr}");
    let location = format!(
        "/{}:",
        std::path::Path::new(file!()).file_name().unwrap().to_str().unwrap()
    );
    let source_line = stderr.lines().find_map(|line| {
        line.trim()
            .strip_prefix("at ")
            .and_then(|frame| frame.split_once(&location))
            .map(|(_, rest)| rest.split(':').next().unwrap().parse::<u32>().unwrap())
    });
    assert!(source_line.is_some_and(|line| line > 0), "{stderr}");
}
