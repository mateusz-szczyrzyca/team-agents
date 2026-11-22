//! A request from an agent with a plan, end to end: the daemon starts the run from this repository's `agent`, which reads
//! the run's plan at the moment of the request; the stand-in CLI asks for a worker through the agentbus MCP server of its
//! own MCP configuration, and the bus answers as `delegate` does.
//!
//! The daemon's own end-to-end tests of a team live in `crates/agent/src/daemon/owner/teams/tests.rs`; this file runs the
//! same wire with the binaries of the build: a bus of a run that keeps a plan, and the request of its stand-in.

#[cfg(test)]
mod delegation_process {
    use std::io::Write as _;
    use std::path::{Path, PathBuf};
    use std::process::{Command, Stdio};

    /// The `agentbus` built from `agentbus/`, once for all of this file.
    fn agentbus_binary() -> &'static Path {
        static BINARY: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
        BINARY.get_or_init(|| {
            let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../agentbus");
            let binary = Path::new(env!("CARGO_TARGET_TMPDIR")).join("agentbus-under-test-of-delegation");
            let built = Command::new("go")
                .arg("build")
                .arg("-o")
                .arg(&binary)
                .arg(".")
                .current_dir(&source)
                .output()
                .unwrap();
            assert!(
                built.status.success(),
                "go build of {source:?} failed: {}",
                String::from_utf8_lossy(&built.stderr)
            );
            binary
        })
    }

    /// A home with a plan and a bus started as the daemon starts the bus of a run that keeps a plan, and the answer to
    /// one `delegate` call through it.
    fn answer(plan: &str, arguments: &str) -> String {
        let home = tempfile::tempdir().unwrap();
        std::fs::write(
            home.path().join("agents.toml"),
            include_str!("../../../config/agents.example.toml"),
        )
        .unwrap();
        let plan_file = home.path().join("plan.md");
        std::fs::write(&plan_file, plan).unwrap();
        let mut bus = Command::new(agentbus_binary())
            .args([
                "serve",
                "--agent",
                "member-1",
                "--dir",
                home.path().join("bus").to_str().unwrap(),
                "--plan",
                plan_file.to_str().unwrap(),
                "--agent-cmd",
                env!("CARGO_BIN_EXE_agent"),
                "--home",
                home.path().to_str().unwrap(),
            ])
            .env("AGENTS_HOME", home.path())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let call = format!(
            "{{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"tools/call\",\"params\":{{\"name\":\"delegate\",\"arguments\":{arguments}}}}}\n"
        );
        bus.stdin.take().unwrap().write_all(call.as_bytes()).unwrap();
        let out = bus.wait_with_output().unwrap();
        String::from_utf8_lossy(&out.stdout).to_string()
    }

    /// A plan with a step to do, a step in hand and a step done.
    const PLAN: &str = "\
Publish the guide.

1. Draft
[ ] 1.1 List the questions a reader asks
[>] 1.2 Write the walkthrough
    Files: docs/guide.md
    One section per question.
    Check: exists docs/guide.md
[x] 1.3 Check the spelling
";

    /// A complete request for step 1.2 of [`PLAN`], with `for` and `check` as given.
    fn arguments(step: &str, check: &str) -> String {
        format!(
            "{{\"goal\":\"the guide\",\"workdir\":\"/tmp\",\"steps\":\"write it\",\"returns\":\"the file\",\"for\":\"{step}\",\"files\":\"docs/guide.md\",\"check\":\"{check}\",\"do\":\"Write it.\"}}"
        )
    }

    #[test]
    fn a_complete_request_is_accepted_and_says_what_the_worker_gets() {
        let out = answer(PLAN, &arguments("1.2", "exists docs/guide.md"));

        assert!(out.contains("Request H1 accepted"), "{out}");
    }

    #[test]
    fn a_request_without_a_step_is_refused_with_its_cause() {
        let out = answer(PLAN, &arguments("", "exists docs/guide.md"));

        assert!(!out.contains("Request H"), "{out}");
        assert!(out.contains("step of your plan"), "{out}");
    }

    #[test]
    fn a_request_for_a_step_the_plan_does_not_have_or_that_is_done_is_refused() {
        for step in ["9.9", "1.3"] {
            let out = answer(PLAN, &arguments(step, "exists docs/guide.md"));
            assert!(!out.contains("Request H"), "{step}: {out}");
            assert!(
                out.contains("no step of your plan") || out.contains("is done"),
                "{step}: {out}"
            );
        }
    }

    #[test]
    fn a_request_whose_check_is_the_gate_is_refused() {
        let out = answer(PLAN, &arguments("1.2", "make gate"));

        assert!(!out.contains("Request H"), "{out}");
        assert!(out.contains("cannot show a worker"), "{out}");
    }

    #[test]
    fn a_plan_that_does_not_read_refuses_the_request() {
        let out = answer(
            "1. Draft\n[?] 1.1 Not a mark\n",
            &arguments("1.1", "exists docs/guide.md"),
        );

        assert!(!out.contains("Request H"), "{out}");
        assert!(out.contains("does not read"), "{out}");
    }
}
