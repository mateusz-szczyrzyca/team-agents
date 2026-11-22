//! The section of the steps done by a worker in the description of a pull request, end to end: a solo agent's round
//! with a request for a worker that is done and names a step of its plan is published with the section, also when the
//! bus does not answer at the first try.

use std::os::unix::fs::PermissionsExt as _;
use std::path::Path;
use std::process::{Command, Stdio};
use std::str::FromStr;

use agent_core::config::{AbsolutePath, Program};
use agent_core::ids::TaskId;
use agent_core::state::{ItemPhase, MergeMode};
use agent_runtime::agentbus::{AgentBus, DelegationStatus, DelegationUpdate};

use super::fake_github::SLUG;
use super::rig::{ONE, Rig, investigator, normal};

/// A reviewer that approves at once.
const APPROVING: &str = "verify:VERDICT: APPROVE\\nSUMMARY: fine";

/// The line the section holds for the request of the test.
const STEP_LINE: &str = "- step 1.1 of claude-1's plan — done by glm-1 (H1)";

/// The script of the solo agent's run: it commits its work, says it is ready and waits until the test releases it.
const HOLD: &str = r#"set -eu
printf 'run 1 of claude-1\n' > work-1.txt
git add work-1.txt
git -c user.name=Fake -c user.email=fake@example.com -c commit.gpgsign=false commit -qm 'Work of run 1 of claude-1'
barrier="$(dirname "$0")/release"
mkfifo "$barrier"
: > "$barrier-ready"
read -r -t 30 release < "$barrier"
printf '%s\n' '.fake-*' >> "$(git rev-parse --git-path info/exclude)"
rm "$barrier" "$barrier-ready"
"#;

/// The stand-in of `agent _delegation-check` the request is checked with: it accepts the request for step 1.1.
const ACCEPTING_AGENT: &str = "#!/bin/sh\nprintf 'ACCEPTED\\nThe step of the requester'\"'\"'s plan:\\nStep 1.1 Write it\\nRequest files: work-1.txt\\nRequest check: exists work-1.txt\\nDo: Write it.\\n'\n";

/// A solo agent that holds its run until released, and the two reviewers.
async fn holding_rig() -> Rig {
    let rig = Rig::new(
        &[
            normal("claude-1", "script:hold"),
            investigator("inv-r1", APPROVING),
            investigator("inv-r2", APPROVING),
        ],
        5,
        false,
    )
    .await;
    let scripts = rig.home.root().join("fake-scripts");
    std::fs::create_dir_all(&scripts).unwrap();
    std::fs::write(scripts.join("hold.sh"), HOLD).unwrap();
    rig
}

/// The bus of the rig's home, as the daemon runs it.
fn bus(rig: &Rig) -> AgentBus {
    AgentBus::new(
        Program::from_str(crate::daemon::owner::fixture::agentbus_binary().to_str().unwrap()).unwrap(),
        AbsolutePath::from_str(rig.home.root().to_str().unwrap()).unwrap(),
    )
}

/// Asks for a worker as the agent of `task` does, through `agentbus serve` with a plan, for step 1.1; the request is
/// then given to the worker glm-1 and done, as when the worker returned its result.
async fn a_done_request_for_step_1_1(rig: &Rig, task: &TaskId) {
    let agent = rig.home.root().join("accepting-agent");
    std::fs::write(&agent, ACCEPTING_AGENT).unwrap();
    std::fs::set_permissions(&agent, std::fs::Permissions::from_mode(0o755)).unwrap();
    let worktree = rig.worktree(task);
    let plan = rig.home.root().join("plan.md");
    std::fs::write(&plan, "Do it.\n\n1. Work\n[>] 1.1 Write it\n").unwrap();
    let call = format!(
        "{{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"tools/call\",\"params\":{{\"name\":\"delegate\",\"arguments\":{{\"goal\":\"the work\",\"workdir\":\"{}\",\"steps\":\"write it\",\"returns\":\"the file\",\"for\":\"1.1\",\"files\":\"work-1.txt\",\"check\":\"exists work-1.txt\",\"do\":\"Write it.\"}}}}}}\n",
        worktree.display()
    );
    let mut serve = Command::new(crate::daemon::owner::fixture::agentbus_binary())
        .args(["serve", "--agent", "claude-1", "--task", task.as_str(), "--dir"])
        .arg(rig.home.root().join("bus"))
        .arg("--home")
        .arg(rig.home.root())
        .args(["--repo", "example", "--branch"])
        .arg(format!("agents/claude-1/{task}"))
        .arg("--worktree")
        .arg(&worktree)
        .arg("--plan")
        .arg(&plan)
        .arg("--agent-cmd")
        .arg(&agent)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    std::io::Write::write_all(&mut serve.stdin.take().unwrap(), call.as_bytes()).unwrap();
    let answer = String::from_utf8(serve.wait_with_output().unwrap().stdout).unwrap();
    assert!(answer.contains("Request H1 accepted"), "{answer}");
    let bus = bus(rig);
    let given = bus.helper_next(&"glm-1".parse().unwrap()).await.unwrap().unwrap();
    bus.helper_set(
        given.id,
        &DelegationUpdate {
            status: Some(DelegationStatus::Done),
            task: None,
            summary: Some("Result: written.".to_owned()),
        },
    )
    .await
    .unwrap();
}

/// Waits for the rig's solo run to hold, puts the done request in the bus, releases the run and returns the body of the
/// pull request the daemon opened.
async fn published_body(rig: &Rig, task: &TaskId) -> String {
    let release = rig.home.root().join("fake-scripts/release");
    rig.wait_for("the solo run to hold", || {
        Path::new(&format!("{}-ready", release.display())).exists()
    })
    .await;
    a_done_request_for_step_1_1(rig, task).await;
    std::fs::write(&release, "go\n").unwrap();
    rig.item_where("1.1", "in review", |phase| {
        matches!(
            phase,
            ItemPhase::InReview { .. } | ItemPhase::Approved { .. } | ItemPhase::Done { .. }
        )
    })
    .await;
    let opened = rig.github.calls("POST", &format!("/repos/{SLUG}/pulls"));
    assert_eq!(opened.len(), 1, "one pull request is opened");
    let request: serde_json::Value = serde_json::from_str(&opened[0].body).unwrap();
    request["body"].as_str().unwrap().to_owned()
}

#[tokio::test]
async fn a_round_with_a_done_request_for_a_step_is_published_with_the_steps_done_by_a_worker() {
    let mut rig = holding_rig().await;
    rig.start().await;
    let (_, tasks) = rig.run(&["claude-1"], ONE, MergeMode::Autopilot, None).await;

    let body = published_body(&rig, &tasks[0]).await;
    rig.stop().await.unwrap();

    assert!(body.contains("## Steps done by a worker"), "{body}");
    assert!(body.contains(STEP_LINE), "{body}");
}

#[tokio::test]
async fn a_bus_that_does_not_list_the_requests_at_the_first_try_delays_the_publication_and_the_section_is_kept() {
    let mut rig = holding_rig().await;
    // The daemon's bus refuses the first helper-dump, as a bus that is not there for a moment.
    let config = std::fs::read_to_string(rig.home.config_file()).unwrap();
    let real = crate::daemon::owner::fixture::agentbus_binary().display().to_string();
    let flaky = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/flaky-agentbus.sh");
    assert!(config.contains(&format!("agentbus = \"{real}\"")), "{config}");
    std::fs::write(
        rig.home.config_file(),
        config.replace(
            &format!("agentbus = \"{real}\""),
            &format!("agentbus = \"{}\"", flaky.display()),
        ),
    )
    .unwrap();
    std::fs::write(rig.home.root().join("real-agentbus"), &real).unwrap();
    std::fs::write(rig.home.root().join("flaky-bus-refuse-once"), "helper-dump").unwrap();
    rig.start().await;
    let (_, tasks) = rig.run(&["claude-1"], ONE, MergeMode::Autopilot, None).await;

    let body = published_body(&rig, &tasks[0]).await;
    rig.stop().await.unwrap();

    assert!(
        !rig.home.root().join("flaky-bus-refuse-once").exists(),
        "the first helper-dump was refused"
    );
    assert!(body.contains(STEP_LINE), "{body}");
    let told = rig.notifications();
    assert!(
        told.iter()
            .any(|text| text.contains("cannot list the requests for workers")
                && text.contains("The harness tries again by itself")),
        "the human is told that the harness tries again: {told:?}"
    );
}
