//! The checks before a review in a daemon: a round that the checks hold back starts no review, the team is given the
//! reasons or the human is asked, and the answer starts the review or sends the round back — against a fake GitHub, a bare
//! `origin` and stand-ins for the model CLIs that do what a script says.

use std::num::NonZeroU32;

use agent_core::decide::checks::Subject;
use agent_core::ids::TaskId;
use agent_core::protocol::{AnswerInput, AnswerList, AnswerRequest, Client, Command, Reply};
use agent_core::questions::{QuestionsFile, Verdict};
use agent_core::state::{ChecksRecord, ItemPhase, MergeMode, PhaseKind};
use agent_core::status::TaskStatus;
use agent_runtime::store::load_checks_record;

use super::fake_github::SLUG;
use super::rig::{ONE, Rig, git, investigator, normal};

/// Reviewers that never give a verdict: they end their turn and wait.
const SILENT: &str = "sleep";

/// A reviewer that approves.
const APPROVE: &str = "done:VERDICT: APPROVE\\nSUMMARY: fine";

/// A reviewer that asks for changes the first time and approves afterwards.
const CHANGES_ONCE: &str = "done:VERDICT: REQUEST_CHANGES\\nBLOCKERS\\n1. [TDD-2] src/lib.rs:7 — the case is untested,done:VERDICT: APPROVE\\nSUMMARY: fixed";

/// What every script starts with: git commits as a stand-in without signing. A script adds the files it changed by name:
/// the stand-in CLI leaves files of its own (`.fake-*`) in the worktree, which a commit must not carry to the reviewers.
const PRELUDE: &str = "set -eu\ncommit() { git -c user.name=Fake -c user.email=fake@example.com -c commit.gpgsign=false commit -q \"$@\"; }\n";

/// A rig with `lead` as the solo agent and two investigators with the scenarios `first` and `second`; the repository of
/// the team has the checks table, `check_passes` of them, and a test, a protected file and a file of code in its history.
async fn checking_rig(lead: &'static str, check_passes: u32, first: &'static str, second: &'static str) -> Rig {
    let rig = Rig::new(
        &[
            normal("claude-1", lead),
            investigator("inv-r1", first),
            investigator("inv-r2", second),
        ],
        5,
        false,
    )
    .await;
    let text = std::fs::read_to_string(rig.home.config_file()).unwrap();
    let text = text.replace(
        "fix_passes = 5",
        &format!("fix_passes = 5\ncheck_passes = {check_passes}"),
    );
    std::fs::write(
        rig.home.config_file(),
        format!(
            "{text}[repos.example.checks]\ntest_paths = [\"tests/\"]\ncode_paths = [\"src/\"]\nprotected_paths = [\"ci/\"]\n"
        ),
    )
    .unwrap();
    for (name, text) in [
        ("tests/existing.txt", "a\nb\nc\n"),
        ("src/code.txt", "code\n"),
        ("ci/gate.sh", "true\n"),
    ] {
        let file = rig.repo.join(name);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(file, text).unwrap();
    }
    git(&rig.repo, &["add", "-A"]);
    git(
        &rig.repo,
        &["commit", "-q", "-m", "Seed the tests, the code and the gate"],
    );
    git(&rig.repo, &["push", "-q", "origin", "main"]);
    rig
}

/// Writes the script `name` of the stand-in CLI: `body` after the prelude.
fn script(rig: &Rig, name: &str, body: &str) {
    let dir = rig.home.root().join("fake-scripts");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join(format!("{name}.sh")), format!("{PRELUDE}{body}")).unwrap();
}

/// The scripts most tests use.
fn scripts(rig: &Rig) {
    script(
        rig,
        "edit-test",
        "sed -i 2d tests/existing.txt\ngit add tests/existing.txt\ncommit -m 'Change the existing test'\n",
    );
    script(
        rig,
        "restore-test",
        "git checkout main -- tests/existing.txt\ngit add tests/existing.txt\ncommit -m 'Restore the test'\n",
    );
    script(
        rig,
        "code-only",
        "echo 'fn a() {}' > src/a.txt\ngit add src/a.txt\ncommit -m 'Add code'\n",
    );
    script(
        rig,
        "add-test",
        "echo case > tests/new.txt\ngit add tests/new.txt\ncommit -m 'Add a test'\n",
    );
    script(
        rig,
        "code-and-test",
        "echo 'fn a() {}' > src/a.txt\necho case > tests/new.txt\ngit add src/a.txt tests/new.txt\ncommit -m 'Add code and its test'\n",
    );
    script(
        rig,
        "protected",
        "echo 'exit 0' >> ci/gate.sh\ngit add ci/gate.sh\ncommit -m 'Change the gate'\n",
    );
}

/// Answers questions number `number` of `task` with the verdict `text`.
async fn answer(rig: &Rig, task: &TaskId, number: u32, text: &str) {
    let answers = AnswerList::try_from(vec![AnswerInput {
        question: NonZeroU32::MIN,
        verdict: Verdict::from_text(text).unwrap().unwrap(),
    }])
    .unwrap();
    let reply = crate::client::Connection::connect(&rig.home.socket_file())
        .await
        .unwrap()
        .request(
            Client::Cli,
            Command::Answer(AnswerRequest {
                task: task.clone(),
                number: NonZeroU32::new(number).unwrap(),
                answers,
            }),
        )
        .await
        .unwrap();
    assert!(matches!(reply, Reply::Applied { .. }), "{reply:?}");
}

/// Replaces the description of pull request `number` on the fake GitHub, as the human or the team does on GitHub.
async fn edit_description(rig: &Rig, number: u32, body: &str) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let url = rig.github.url();
    let address = url.strip_prefix("http://").unwrap();
    let mut stream = tokio::net::TcpStream::connect(address).await.unwrap();
    let payload = serde_json::json!({ "body": body }).to_string();
    let request = format!(
        "PATCH /repos/{SLUG}/pulls/{number} HTTP/1.1\r\nHost: {address}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{payload}",
        payload.len()
    );
    stream.write_all(request.as_bytes()).await.unwrap();
    let mut response = String::new();
    stream.read_to_string(&mut response).await.unwrap();
    assert!(response.starts_with("HTTP/1.1 200"), "{response}");
}

/// The record of the checks of `task`.
async fn record(rig: &Rig, task: &TaskId) -> ChecksRecord {
    load_checks_record(&rig.home, task).await.unwrap().unwrap_or_default()
}

/// The phase of item `1.1` of the only team.
fn phase(rig: &Rig) -> ItemPhase {
    rig.state().teams()[0]
        .item("1.1".parse().unwrap())
        .unwrap()
        .phase()
        .clone()
}

/// Waits until item `1.1` has its two reviews asked.
async fn reviews_asked(rig: &Rig) {
    rig.item_where(
        "1.1",
        "asked for reviews",
        |phase| matches!(phase, ItemPhase::InReview { reviews, .. } if reviews.len() == 2),
    )
    .await;
}

/// The questions file number `number` of `task`, read.
fn questions_of(rig: &Rig, task: &TaskId, number: u32) -> QuestionsFile {
    let path = rig.home.questions_file(task, NonZeroU32::new(number).unwrap());
    QuestionsFile::parse(&std::fs::read_to_string(path).unwrap()).unwrap()
}

// ---- what only the human may allow ------------------------------------------------------------------------------------

#[tokio::test]
async fn a_round_that_edits_an_existing_test_asks_the_human_by_name_and_starts_no_review_until_they_allow_it() {
    let mut rig = checking_rig("script:edit-test,questions,done", 2, SILENT, SILENT).await;
    scripts(&rig);
    rig.start().await;

    let (team, tasks) = rig.run(&["claude-1"], ONE, MergeMode::Autopilot, None).await;
    rig.status_is(&tasks[0], TaskStatus::Questions).await;

    // Held: the pull request is a draft, no review is asked, the item is where the team left it.
    assert_eq!(phase(&rig).kind(), PhaseKind::InProgress);
    assert_eq!(rig.state().team(&team).unwrap().reviewers(), []);
    assert!(rig.github.pulls()[0].draft);
    assert_eq!(rig.github.calls("POST", "/graphql"), []);
    // The question names the file and gives the human the words to answer with.
    let file = questions_of(&rig, &tasks[0], 1);
    assert_eq!(file.entries().len(), 1);
    assert!(
        file.entries()[0].title().contains("`tests/existing.txt`"),
        "{}",
        file.entries()[0].title()
    );
    assert!(
        file.entries()[0].body().contains("`allow`"),
        "{}",
        file.entries()[0].body()
    );
    assert!(file.entries()[0].recommendation().is_some());

    answer(&rig, &tasks[0], 1, "allow").await;
    reviews_asked(&rig).await;
    rig.stop().await.unwrap();

    // The review started: the pull request is ready, and the allowance is recorded for the item.
    assert!(!rig.github.pulls()[0].draft);
    assert_eq!(rig.github.calls("POST", "/graphql").len(), 1);
    let kept = record(&rig, &tasks[0]).await;
    assert_eq!(
        kept.allowed_of(&["1.1".parse().unwrap()]),
        [Subject::Path("tests/existing.txt".parse().unwrap())]
    );
    assert_eq!(kept.asked, None);
    assert_eq!(rig.runs(&tasks[0]), 3);
    // The lead that put the question was told to do nothing else, and told what the answer was for afterwards.
    assert!(rig.prompt(&tasks[0], 2).await.contains("STATUS: QUESTIONS"));
    assert!(
        rig.prompt(&tasks[0], 3)
            .await
            .contains("THE HUMAN ANSWERED THE QUESTIONS OF THE ROUND CHECKS")
    );
}

#[tokio::test]
async fn what_the_human_refuses_goes_back_to_the_team_with_the_refusal_and_is_reviewed_once_it_is_undone() {
    let mut rig = checking_rig("script:edit-test,questions,done,script:restore-test", 2, SILENT, SILENT).await;
    scripts(&rig);
    rig.start().await;

    let (_, tasks) = rig.run(&["claude-1"], ONE, MergeMode::Autopilot, None).await;
    rig.status_is(&tasks[0], TaskStatus::Questions).await;
    answer(&rig, &tasks[0], 1, "refuse").await;
    reviews_asked(&rig).await;
    let fix = rig.prompt(&tasks[0], 4).await;
    rig.stop().await.unwrap();

    assert!(fix.contains("THE ROUND IS NOT READY FOR ITS REVIEW"), "{fix}");
    assert!(fix.contains("check 1"), "{fix}");
    assert!(fix.contains("refused to allow"), "{fix}");
    assert!(fix.contains("`tests/existing.txt`"), "{fix}");
    // Nothing was allowed: the test was put back.
    assert_eq!(record(&rig, &tasks[0]).await.allowed_of(&["1.1".parse().unwrap()]), []);
}

#[tokio::test]
async fn a_round_that_changes_a_protected_file_waits_for_the_human() {
    let mut rig = checking_rig("script:protected,questions,done", 2, SILENT, SILENT).await;
    scripts(&rig);
    rig.start().await;

    let (_, tasks) = rig.run(&["claude-1"], ONE, MergeMode::Autopilot, None).await;
    rig.status_is(&tasks[0], TaskStatus::Questions).await;
    let file = questions_of(&rig, &tasks[0], 1);
    answer(&rig, &tasks[0], 1, "allow").await;
    reviews_asked(&rig).await;
    rig.stop().await.unwrap();

    assert!(
        file.entries()[0].title().contains("`ci/gate.sh`"),
        "{}",
        file.entries()[0].title()
    );
    assert!(
        file.entries()[0].body().contains("judges a round"),
        "{}",
        file.entries()[0].body()
    );
}

// ---- what the team can fix ------------------------------------------------------------------------------------------

#[tokio::test]
async fn a_round_that_changes_code_without_a_test_goes_back_to_the_team_with_the_reason_and_starts_no_review() {
    let mut rig = checking_rig("script:code-only,script:add-test", 2, SILENT, SILENT).await;
    scripts(&rig);
    rig.start().await;

    let (team, tasks) = rig.run(&["claude-1"], ONE, MergeMode::Autopilot, None).await;
    let back = rig.prompt(&tasks[0], 2).await;
    reviews_asked(&rig).await;
    rig.stop().await.unwrap();

    assert!(
        back.contains("THE ROUND IS NOT READY FOR ITS REVIEW (pull request #1, check 1)"),
        "{back}"
    );
    assert!(back.contains("Test-First: not possible"), "{back}");
    assert!(back.contains("agent gate"), "{back}");
    // The human was not asked, and the review waited for the fixed head: one pull request, made ready once.
    assert_eq!(
        rig.state().team(&team).unwrap().find(&tasks[0]).unwrap().1.questions(),
        []
    );
    assert_eq!(rig.github.calls("POST", &format!("/repos/{SLUG}/pulls")).len(), 1);
    assert_eq!(rig.github.calls("POST", "/graphql").len(), 1);
    assert_eq!(record(&rig, &tasks[0]).await.passes_of(&["1.1".parse().unwrap()]), 1);
}

#[tokio::test]
async fn after_its_passes_the_human_is_asked_and_their_answer_starts_the_review() {
    let mut rig = checking_rig("script:code-only,done,questions,done", 1, SILENT, SILENT).await;
    scripts(&rig);
    rig.start().await;

    let (_, tasks) = rig.run(&["claude-1"], ONE, MergeMode::Autopilot, None).await;
    rig.status_is(&tasks[0], TaskStatus::Questions).await;
    let file = questions_of(&rig, &tasks[0], 1);
    // The team was sent back once and did nothing about it.
    assert!(rig.prompt(&tasks[0], 2).await.contains("check 1"));
    assert_eq!(phase(&rig).kind(), PhaseKind::InProgress);
    answer(&rig, &tasks[0], 1, "allow").await;
    reviews_asked(&rig).await;
    rig.stop().await.unwrap();

    assert!(
        file.entries()[0].title().contains("production code"),
        "{}",
        file.entries()[0].title()
    );
    assert!(
        file.entries()[0].body().contains("1 time(s)"),
        "{}",
        file.entries()[0].body()
    );
    let kept = record(&rig, &tasks[0]).await;
    assert_eq!(kept.allowed_of(&["1.1".parse().unwrap()]), [Subject::NoTests]);
    assert_eq!(kept.passes_of(&["1.1".parse().unwrap()]), 0);
}

// ---- a test that was frozen -------------------------------------------------------------------------------------------

#[tokio::test]
async fn a_green_commit_that_edits_the_test_of_its_red_commit_goes_back_to_the_team_with_the_commit_and_the_file() {
    let mut rig = checking_rig("script:red-green,done", 2, SILENT, SILENT).await;
    script(
        &rig,
        "red-green",
        "printf 'x\\ny\\n' > tests/new.txt\ngit add tests/new.txt\ncommit -m 'RED: a test'\necho 'fn a() {}' > src/a.txt\nsed -i 1d tests/new.txt\ngit add src/a.txt tests/new.txt\ncommit -m 'GREEN: code a'\n",
    );
    rig.start().await;

    let (team, tasks) = rig.run(&["claude-1"], ONE, MergeMode::Autopilot, None).await;
    let back = rig.prompt(&tasks[0], 2).await;
    // A commit is named by its first seven digits.
    let short = |grep: &str| -> String {
        git(&rig.worktree(&tasks[0]), &["log", "-1", "--format=%H", grep])
            .chars()
            .take(7)
            .collect()
    };
    let (green, red) = (short("--grep=^GREEN"), short("--grep=^RED"));
    rig.stop().await.unwrap();

    assert!(back.contains("`tests/new.txt`"), "{back}");
    assert!(back.contains(&green), "{back}");
    assert!(back.contains(&red), "{back}");
    assert!(back.contains("TEST-FIX:"), "{back}");
    assert_eq!(rig.state().team(&team).unwrap().reviewers(), []);
    assert_eq!(rig.github.calls("POST", "/graphql"), []);
}

#[tokio::test]
async fn the_same_change_as_a_test_fix_that_the_description_names_goes_to_review() {
    let mut rig = checking_rig("script:red-testfix,slowdone", 5, SILENT, SILENT).await;
    script(
        &rig,
        "red-testfix",
        "printf 'x\\ny\\n' > tests/new.txt\ngit add tests/new.txt\ncommit -m 'RED: a test'\nprintf 'x\\nz\\n' > tests/new.txt\ngit add tests/new.txt\ncommit -m 'TEST-FIX: the second line was wrong'\necho 'fn a() {}' > src/a.txt\ngit add src/a.txt\ncommit -m 'GREEN: code a'\n",
    );
    rig.start().await;

    let (_, tasks) = rig.run(&["claude-1"], ONE, MergeMode::Autopilot, None).await;
    // The first check does not find the test fix named: the team is sent back, and somebody names it in the description.
    rig.wait_for("the pull request", || !rig.github.pulls().is_empty())
        .await;
    let fix = git(
        &rig.worktree(&tasks[0]),
        &["log", "-1", "--format=%H", "--grep=^TEST-FIX"],
    );
    edit_description(
        &rig,
        1,
        &format!("## Tests changed after RED\n\n- {fix}: the second line was wrong\n"),
    )
    .await;
    reviews_asked(&rig).await;
    let back = rig.prompt(&tasks[0], 2).await;
    rig.stop().await.unwrap();

    assert!(
        back.contains("TEST-FIX:") && back.contains("## Tests changed after RED"),
        "{back}"
    );
    assert!(!rig.github.pulls()[0].draft);
    assert_eq!(record(&rig, &tasks[0]).await.passes_of(&["1.1".parse().unwrap()]), 1);
}

// ---- after a fix pass -------------------------------------------------------------------------------------------------

#[tokio::test]
async fn the_checks_run_again_on_the_head_of_a_fix_pass_and_a_head_they_hold_back_is_not_pushed() {
    let mut rig = checking_rig(
        "script:code-and-test,script:edit-test,questions",
        2,
        CHANGES_ONCE,
        APPROVE,
    )
    .await;
    scripts(&rig);
    rig.start().await;

    let (_, tasks) = rig.run(&["claude-1"], ONE, MergeMode::Autopilot, None).await;
    // The first review pass is clean and is reviewed; the fix pass then edits an existing test.
    rig.status_is(&tasks[0], TaskStatus::Questions).await;
    let branch = format!("agents/claude-1/{}-1.1", tasks[0]);
    let file = questions_of(&rig, &tasks[0], 1);
    rig.stop().await.unwrap();

    assert!(
        file.entries()[0].title().contains("`tests/existing.txt`"),
        "{}",
        file.entries()[0].title()
    );
    assert!(matches!(phase(&rig), ItemPhase::Fixing { .. }));
    // The held head stayed on the worktree's branch: the remote still has the head that was reviewed.
    assert_ne!(rig.remote_tip(&branch), git(&rig.repo, &["rev-parse", &branch]));
    assert_eq!(rig.github.comments(1), Vec::<String>::new());
}

// ---- no table ---------------------------------------------------------------------------------------------------------

#[tokio::test]
async fn a_repository_without_the_checks_table_goes_straight_to_review_whatever_the_round_does() {
    let mut rig = Rig::new(
        &[
            normal("claude-1", "script:edit-test"),
            investigator("inv-r1", SILENT),
            investigator("inv-r2", SILENT),
        ],
        5,
        false,
    )
    .await;
    std::fs::create_dir_all(rig.repo.join("tests")).unwrap();
    std::fs::write(rig.repo.join("tests/existing.txt"), "a\nb\nc\n").unwrap();
    git(&rig.repo, &["add", "-A"]);
    git(&rig.repo, &["commit", "-q", "-m", "Seed a test"]);
    git(&rig.repo, &["push", "-q", "origin", "main"]);
    scripts(&rig);
    rig.start().await;

    let (_, tasks) = rig.run(&["claude-1"], ONE, MergeMode::Autopilot, None).await;
    reviews_asked(&rig).await;
    rig.stop().await.unwrap();

    assert!(!rig.github.pulls()[0].draft);
    assert!(!rig.home.checks_file(&tasks[0]).exists());
    assert_eq!(rig.runs(&tasks[0]), 1);
}

// ---- the question and its messenger ---------------------------------------------------------------------------------

/// Waits until the daemon has looked at its teams since now: its answer to a request comes between two turns of its loop,
/// and each turn starts with a look at the teams that something happened to.
async fn looked_again(rig: &Rig) {
    for _ in 0..3 {
        let reply = crate::client::Connection::connect(&rig.home.socket_file())
            .await
            .unwrap()
            .request(
                Client::Cli,
                Command::Status(agent_core::protocol::StatusRequest { target: None }),
            )
            .await
            .unwrap();
        assert!(matches!(reply, Reply::Teams { .. }), "{reply:?}");
    }
}

#[tokio::test]
async fn a_lead_that_does_not_put_the_question_is_woken_twice_and_then_the_human_is_told_once() {
    // The lead edits a test and then ends its turn with `done` whenever it is asked to put the question.
    let mut rig = checking_rig("script:edit-test,done", 2, SILENT, SILENT).await;
    scripts(&rig);
    rig.start().await;

    let (team, tasks) = rig.run(&["claude-1"], ONE, MergeMode::Autopilot, None).await;
    let notice = rig.notification_with("no question was put to you").await;
    looked_again(&rig).await;
    rig.stop().await.unwrap();

    assert!(notice.contains(tasks[0].as_str()), "{notice}");
    assert!(notice.contains(team.as_str()), "{notice}");
    // The run that edited the test, and the two that were asked to put the question and did not, and the one after them.
    assert_eq!(rig.runs(&tasks[0]), 4);
    assert_eq!(
        rig.notifications()
            .iter()
            .filter(|text| text.contains("no question was put to you"))
            .count(),
        1
    );
    assert_eq!(record(&rig, &tasks[0]).await.asked.map(|asked| asked.relays), Some(2));
    assert_eq!(phase(&rig).kind(), PhaseKind::InProgress);
    assert_eq!(
        rig.state().team(&team).unwrap().find(&tasks[0]).unwrap().1.questions(),
        []
    );
}

#[tokio::test]
async fn while_the_human_has_not_answered_nothing_is_asked_again_and_the_lead_is_not_woken() {
    let mut rig = checking_rig("script:edit-test,questions,done", 2, SILENT, SILENT).await;
    scripts(&rig);
    rig.start().await;

    let (_, tasks) = rig.run(&["claude-1"], ONE, MergeMode::Autopilot, None).await;
    rig.status_is(&tasks[0], TaskStatus::Questions).await;
    looked_again(&rig).await;
    let questions = std::fs::read_to_string(rig.home.questions_file(&tasks[0], NonZeroU32::MIN)).unwrap();
    looked_again(&rig).await;
    rig.stop().await.unwrap();

    assert_eq!(rig.runs(&tasks[0]), 2);
    assert_eq!(
        std::fs::read_to_string(rig.home.questions_file(&tasks[0], NonZeroU32::MIN)).unwrap(),
        questions
    );
    assert_eq!(rig.github.calls("POST", &format!("/repos/{SLUG}/pulls")).len(), 1);
}

#[tokio::test]
async fn the_answer_of_the_human_is_kept_across_a_restart_of_the_daemon() {
    let mut rig = checking_rig("script:edit-test,questions,done", 2, SILENT, SILENT).await;
    scripts(&rig);
    rig.start().await;

    let (_, tasks) = rig.run(&["claude-1"], ONE, MergeMode::Autopilot, None).await;
    rig.status_is(&tasks[0], TaskStatus::Questions).await;
    rig.stop().await.unwrap();
    rig.start().await;
    answer(&rig, &tasks[0], 1, "allow").await;
    reviews_asked(&rig).await;
    rig.stop().await.unwrap();

    assert!(!rig.github.pulls()[0].draft);
    assert_eq!(
        record(&rig, &tasks[0]).await.allowed_of(&["1.1".parse().unwrap()]),
        [Subject::Path("tests/existing.txt".parse().unwrap())]
    );
}

#[tokio::test]
async fn each_question_is_answered_for_its_own_file_and_only_what_was_refused_goes_back_to_the_team() {
    let mut rig = checking_rig("script:edit-two,questions,done,script:restore-other", 2, SILENT, SILENT).await;
    scripts(&rig);
    script(
        &rig,
        "edit-two",
        "sed -i 2d tests/existing.txt\nsed -i 2d tests/other.txt\ngit add tests/existing.txt tests/other.txt\ncommit -m 'Change two tests'\n",
    );
    script(
        &rig,
        "restore-other",
        "git checkout main -- tests/other.txt\ngit add tests/other.txt\ncommit -m 'Restore the other test'\n",
    );
    std::fs::write(rig.repo.join("tests/other.txt"), "x\ny\nz\n").unwrap();
    git(&rig.repo, &["add", "tests/other.txt"]);
    git(&rig.repo, &["commit", "-q", "-m", "Seed another test"]);
    git(&rig.repo, &["push", "-q", "origin", "main"]);
    rig.start().await;

    let (_, tasks) = rig.run(&["claude-1"], ONE, MergeMode::Autopilot, None).await;
    rig.status_is(&tasks[0], TaskStatus::Questions).await;
    let file = questions_of(&rig, &tasks[0], 1);
    let answers = AnswerList::try_from(vec![
        AnswerInput {
            question: NonZeroU32::MIN,
            verdict: Verdict::from_text("allow").unwrap().unwrap(),
        },
        AnswerInput {
            question: NonZeroU32::new(2).unwrap(),
            verdict: Verdict::from_text("refuse").unwrap().unwrap(),
        },
    ])
    .unwrap();
    let reply = crate::client::Connection::connect(&rig.home.socket_file())
        .await
        .unwrap()
        .request(
            Client::Cli,
            Command::Answer(AnswerRequest {
                task: tasks[0].clone(),
                number: NonZeroU32::MIN,
                answers,
            }),
        )
        .await
        .unwrap();
    assert!(matches!(reply, Reply::Applied { .. }), "{reply:?}");
    reviews_asked(&rig).await;
    let back = rig.prompt(&tasks[0], 4).await;
    rig.stop().await.unwrap();

    let titles: Vec<&str> = file.entries().iter().map(agent_core::questions::Entry::title).collect();
    assert!(
        titles[0].contains("`tests/existing.txt`") && titles[1].contains("`tests/other.txt`"),
        "{titles:?}"
    );
    assert!(
        back.contains("refused to allow") && back.contains("- `tests/other.txt`"),
        "{back}"
    );
    assert!(!back.contains("tests/existing.txt"), "{back}");
    assert_eq!(
        record(&rig, &tasks[0]).await.allowed_of(&["1.1".parse().unwrap()]),
        [Subject::Path("tests/existing.txt".parse().unwrap())]
    );
}

// ---- what the description names ---------------------------------------------------------------------------------------

/// A list whose item names the file the scripts put a lint suppression in.
const NAMES_A_FILE: &str = "\
Build the thing.

1. First
[ ] 1.1 The first item
    Add a function to src/lib.rs.
    Expected result:
    - the function works
";

#[tokio::test]
async fn a_file_that_only_the_text_of_the_item_names_is_not_disclosed_to_the_checks() {
    let mut rig = checking_rig("script:allow-lint,done", 2, SILENT, SILENT).await;
    scripts(&rig);
    script(
        &rig,
        "allow-lint",
        "printf '#[allow(dead_code)]\\nfn a() {}\\n' > src/lib.rs\necho case > tests/new.txt\ngit add src/lib.rs tests/new.txt\ncommit -m 'Add a function and its test'\n",
    );
    rig.start().await;

    let (_, tasks) = rig.run(&["claude-1"], NAMES_A_FILE, MergeMode::Autopilot, None).await;
    let back = rig.prompt(&tasks[0], 2).await;
    rig.stop().await.unwrap();

    assert!(
        back.contains("`src/lib.rs`") && back.contains("suppress a lint"),
        "{back}"
    );
    assert_eq!(rig.github.calls("POST", "/graphql"), []);
}

#[tokio::test]
async fn a_file_that_somebody_names_in_the_description_after_the_check_is_disclosed() {
    let mut rig = checking_rig("script:allow-lint,slowdone", 5, SILENT, SILENT).await;
    scripts(&rig);
    script(
        &rig,
        "allow-lint",
        "printf '#[allow(dead_code)]\\nfn a() {}\\n' > src/lib.rs\necho case > tests/new.txt\ngit add src/lib.rs tests/new.txt\ncommit -m 'Add a function and its test'\n",
    );
    rig.start().await;

    let (_, tasks) = rig.run(&["claude-1"], NAMES_A_FILE, MergeMode::Autopilot, None).await;
    rig.wait_for("the pull request", || !rig.github.pulls().is_empty())
        .await;
    let described = rig.github.pulls()[0].body.clone();
    edit_description(&rig, 1, &format!("{described}\n\n## Suppressions\n\n- `src/lib.rs`: the allow stays until the next item wires the function.\n")).await;
    reviews_asked(&rig).await;
    rig.stop().await.unwrap();

    assert!(!rig.github.pulls()[0].draft);
    assert_eq!(record(&rig, &tasks[0]).await.passes_of(&["1.1".parse().unwrap()]), 1);
}

// ---- an attribute the round only edits -----------------------------------------------------------------------------

#[tokio::test]
async fn a_round_that_edits_one_line_of_a_wrapped_attribute_to_a_suppression_goes_back_to_the_team() {
    let mut rig = checking_rig("script:edit-attribute,done", 2, SILENT, SILENT).await;
    scripts(&rig);
    std::fs::write(
        rig.repo.join("src/wired.rs"),
        "#[cfg_attr(\n    not(test),\n    derive(Debug),\n)]\nstruct Wired;\n",
    )
    .unwrap();
    git(&rig.repo, &["add", "-A"]);
    git(&rig.repo, &["commit", "-q", "-m", "Add a wrapped attribute"]);
    git(&rig.repo, &["push", "-q", "origin", "main"]);
    script(
        &rig,
        "edit-attribute",
        "printf '#[cfg_attr(\\n    not(test),\\n    expect(dead_code, reason = \"later\"),\\n)]\\nstruct Wired;\\n' > src/wired.rs\necho case > tests/new.txt\ngit add src/wired.rs tests/new.txt\ncommit -m 'Edit the attribute and add its test'\n",
    );
    rig.start().await;

    let (_, tasks) = rig.run(&["claude-1"], ONE, MergeMode::Autopilot, None).await;
    let back = rig.prompt(&tasks[0], 2).await;
    rig.stop().await.unwrap();

    assert!(
        back.contains("`src/wired.rs`") && back.contains("suppress a lint"),
        "{back}"
    );
    assert_eq!(rig.github.calls("POST", "/graphql"), []);
}

#[tokio::test]
async fn a_round_that_edits_a_line_far_below_the_opener_of_its_attribute_goes_back_to_the_team() {
    let mut rig = checking_rig("script:edit-attribute,done", 2, SILENT, SILENT).await;
    scripts(&rig);
    let attribute = |applied: &str| {
        let fillers = (0..14)
            .map(|n| format!("    cfg_attr_filler_{n}(true),"))
            .collect::<Vec<_>>()
            .join("\\n");
        format!("#[cfg_attr(\\n    not(test),\\n{fillers}\\n    {applied},\\n)]\\nstruct Wired;\\n")
    };
    std::fs::write(
        rig.repo.join("src/wired.rs"),
        attribute("derive(Debug)").replace("\\n", "\n"),
    )
    .unwrap();
    git(&rig.repo, &["add", "-A"]);
    git(&rig.repo, &["commit", "-q", "-m", "Add a long wrapped attribute"]);
    git(&rig.repo, &["push", "-q", "origin", "main"]);
    script(
        &rig,
        "edit-attribute",
        &format!(
            "printf '{}' > src/wired.rs\necho case > tests/new.txt\ngit add src/wired.rs tests/new.txt\ncommit -m 'Edit the attribute and add its test'\n",
            attribute("allow(dead_code)")
        ),
    );
    rig.start().await;

    let (_, tasks) = rig.run(&["claude-1"], ONE, MergeMode::Autopilot, None).await;
    let back = rig.prompt(&tasks[0], 2).await;
    rig.stop().await.unwrap();

    assert!(
        back.contains("`src/wired.rs`") && back.contains("suppress a lint"),
        "{back}"
    );
    assert_eq!(rig.github.calls("POST", "/graphql"), []);
}

// ---- what the history records of the checks -----------------------------------------------------------------------------

/// The `Checked` events of `team`, in the order they were written.
fn checked(rig: &Rig, team: &agent_core::ids::TeamId) -> Vec<agent_core::history::HistoryEvent> {
    agent_runtime::history::Reader::open(&rig.home.history_file())
        .unwrap()
        .team_records(team)
        .unwrap()
        .into_iter()
        .filter_map(|stored| match stored.record {
            agent_core::history::Record::Event(event) => Some(event.event),
            _ => None,
        })
        .filter(|event| event.kind() == agent_core::history::EventKind::Checked)
        .collect()
}

#[tokio::test]
async fn the_checks_are_recorded_with_each_issue_and_what_followed_sent_back_asked_of_the_human_and_allowed() {
    use agent_core::decide::round::AskReason;
    use agent_core::history::{CheckOutcome, HistoryEvent};

    let mut rig = checking_rig("script:code-only,done,questions,done", 1, SILENT, SILENT).await;
    scripts(&rig);
    rig.start().await;

    let (team, tasks) = rig.run(&["claude-1"], ONE, MergeMode::Autopilot, None).await;
    rig.status_is(&tasks[0], TaskStatus::Questions).await;
    answer(&rig, &tasks[0], 1, "allow").await;
    reviews_asked(&rig).await;
    rig.stop().await.unwrap();

    let events = checked(&rig, &team);
    let outcomes: Vec<CheckOutcome> = events
        .iter()
        .map(|event| match event {
            HistoryEvent::Checked { outcome, .. } => *outcome,
            other => panic!("not a check: {other:?}"),
        })
        .collect();
    assert_eq!(
        outcomes,
        [
            CheckOutcome::SentBack {
                pass: NonZeroU32::MIN,
                of: 1
            },
            CheckOutcome::AskedHuman {
                reason: AskReason::PassesUsed,
                question: NonZeroU32::MIN
            },
            CheckOutcome::Allowed,
        ]
    );
    let HistoryEvent::Checked { pr, head, issues, .. } = &events[0] else {
        panic!("not a check")
    };
    assert_eq!(pr.map(|pr| pr.get().get()), Some(1));
    assert_eq!(head.as_str(), rig.remote_tip(&rig.github.pulls()[0].head));
    assert_eq!(issues.len(), 1);
    assert!(issues[0].contains("production code"), "{}", issues[0]);
    let HistoryEvent::Checked { issues: allowed, .. } = &events[2] else {
        panic!("not a check")
    };
    assert_eq!(allowed.len(), 1);
    assert!(allowed[0].contains("production code"), "{}", allowed[0]);
}

#[tokio::test]
async fn what_the_human_refuses_is_recorded_as_a_round_sent_back_with_the_refused_issue() {
    use agent_core::history::{CheckOutcome, HistoryEvent};

    let mut rig = checking_rig("script:edit-test,questions,done,script:restore-test", 2, SILENT, SILENT).await;
    scripts(&rig);
    rig.start().await;

    let (team, tasks) = rig.run(&["claude-1"], ONE, MergeMode::Autopilot, None).await;
    rig.status_is(&tasks[0], TaskStatus::Questions).await;
    answer(&rig, &tasks[0], 1, "refuse").await;
    reviews_asked(&rig).await;
    rig.stop().await.unwrap();

    let events = checked(&rig, &team);
    assert_ne!(events.len(), 0, "no check was recorded");
    let HistoryEvent::Checked { issues, outcome, .. } = &events[events.len() - 1] else {
        panic!("not a check")
    };
    assert!(matches!(outcome, CheckOutcome::SentBack { .. }), "{outcome:?}");
    assert!(
        issues.iter().any(|issue| issue.contains("tests/existing.txt")),
        "{issues:?}"
    );
}

// ---- the story of a round the checks held back --------------------------------------------------------------------------

#[tokio::test]
async fn a_round_the_checks_send_back_once_and_then_ask_the_human_about_is_told_with_both_and_the_allowance() {
    use super::history_stories_tests::{Normalized, check_story, golden};

    let mut rig = checking_rig("script:code-only,done,questions,done", 1, SILENT, SILENT).await;
    scripts(&rig);
    rig.start_with_rng(rand::SeedableRng::seed_from_u64(7)).await;

    let (team, tasks) = rig.run(&["claude-1"], ONE, MergeMode::Autopilot, None).await;
    rig.status_is(&tasks[0], TaskStatus::Questions).await;
    answer(&rig, &tasks[0], 1, "allow").await;
    reviews_asked(&rig).await;
    rig.state_where("both reviewers to have run", |state| {
        let reviewers = state.team(&team).unwrap().reviewers();
        reviewers.len() == 2
            && reviewers.iter().all(|member| {
                !member.runs().is_empty()
                    && member
                        .runs()
                        .iter()
                        .all(|run| matches!(run.state(), agent_core::state::RunState::Ended { .. }))
            })
    })
    .await;
    rig.stop().await.unwrap();

    let (text, json) = check_story(&rig, &team);

    let sent_back = text.find("→ sent back to the team (pass 1 of 1)").expect(&text);
    let asked = text.find("→ the human is asked in questions 1").expect(&text);
    let allowed = text.find("→ allowed by the human: the reviews go on").expect(&text);
    let reviews = text.find("review pass 1 of #1 asked of").expect(&text);
    assert!(sent_back < asked && asked < allowed && allowed < reviews, "{text}");
    golden("story-checks.txt", &Normalized::of(&rig, &text, &json));
}
