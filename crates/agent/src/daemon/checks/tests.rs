//! The texts of the checks and what the answers of the human mean.

use std::num::NonZeroU32;
use std::path::Path;

use agent_core::config::Language;
use agent_core::decide::checks::markers::Marker;
use agent_core::decide::checks::{Issue, Subject, TestChange, Touch};
use agent_core::decide::round::AskReason;
use agent_core::questions::Verdict;
use agent_core::repo_path::RepoPath;
use agent_core::state::{CommitId, PrNumber};

use super::*;

async fn texts() -> ChecksPrompts {
    ChecksPrompts::load(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("../../prompts"),
        &Language::default(),
    )
    .await
    .unwrap()
}

fn path(text: &str) -> RepoPath {
    text.parse().unwrap()
}

/// A commit ID of forty copies of `digit`.
fn id(digit: char) -> CommitId {
    digit.to_string().repeat(40).parse().unwrap()
}

fn pr(number: u32) -> PrNumber {
    PrNumber::new(NonZeroU32::new(number).unwrap())
}

fn number(n: u32) -> NonZeroU32 {
    NonZeroU32::new(n).unwrap()
}

fn file(text: &str) -> Subject {
    Subject::Path(path(text))
}

fn team() -> TeamId {
    "team-20261113-100000-ab12".parse().unwrap()
}

fn task() -> TaskId {
    "task-20261113-100000-cd34".parse().unwrap()
}

fn edited(name: &str, removed: u32) -> Issue {
    Issue::TestChanged {
        path: path(name),
        how: TestChange::Edited { removed },
    }
}

fn marker(name: &str, marker: Marker) -> Issue {
    Issue::Marker {
        path: path(name),
        marker,
        line: 7,
        count: 2,
    }
}

// ---- reasons ------------------------------------------------------------------------------------------------------

// Rule: the reason of an issue names what it is about — the file, the commits, the numbers — and what to do about it.
#[tokio::test]
async fn the_reason_of_each_kind_of_issue_names_what_it_is_about() {
    let texts = texts().await;
    let cases = [
        (
            Issue::TestChanged {
                path: path("tests/a.rs"),
                how: TestChange::Deleted,
            },
            vec!["`tests/a.rs`", "deletes"],
        ),
        (
            Issue::TestChanged {
                path: path("tests/a.rs"),
                how: TestChange::Renamed { to: path("tests/b.rs") },
            },
            vec!["`tests/a.rs`", "`tests/b.rs`", "renames"],
        ),
        (edited("tests/a.rs", 3), vec!["`tests/a.rs`", "3 line(s)"]),
        (
            Issue::TestChanged {
                path: path("tests/a.bin"),
                how: TestChange::EditedBinary,
            },
            vec!["`tests/a.bin`", "not text"],
        ),
        (
            Issue::ProtectedChanged {
                path: path("AGENTS.md"),
                change: Touch::Renamed,
            },
            vec!["`AGENTS.md`", "renamed it"],
        ),
        (
            Issue::RedTestEdited {
                path: path("tests/a.rs"),
                red: id('1'),
                edit: id('2'),
            },
            vec!["`tests/a.rs`", "1111111", "2222222", "TEST-FIX:"],
        ),
        (Issue::NoTests, vec!["Test-First: not possible"]),
        (
            marker("src/a.rs", Marker::SkipsTest),
            vec!["`src/a.rs`", "2 added line(s)", "line 7", "skip a test"],
        ),
        (
            marker("src/a.rs", Marker::SuppressesLint),
            vec!["`src/a.rs`", "suppress a lint"],
        ),
        (
            marker("src/a.rs", Marker::ChecksUnderTest),
            vec!["`src/a.rs`", "under test"],
        ),
    ];
    for (issue, needles) in cases {
        let text = reason(&texts, &issue).unwrap();

        for needle in needles {
            assert!(text.contains(needle), "{issue:?} lacks {needle:?}: {text}");
        }
        assert!(!text.contains("{{"), "{issue:?}: {text}");
    }
}

// Rule: a test file whose mode alone changed is told as such: its lines are the same, and only the human may allow it.
#[tokio::test]
async fn the_reason_of_a_test_file_whose_mode_changed_names_the_file_and_its_mode() {
    let texts = texts().await;

    let text = reason(
        &texts,
        &Issue::TestChanged {
            path: path("tests/run.sh"),
            how: TestChange::ModeChanged,
        },
    )
    .unwrap();

    for needle in ["`tests/run.sh`", "mode", "only the human"] {
        assert!(text.contains(needle), "lacks {needle:?}: {text}");
    }
    assert!(!text.contains("not text"), "{text}");
}

#[tokio::test]
async fn a_protected_file_is_told_as_added_modified_deleted_or_renamed() {
    let texts = texts().await;

    for (touch, word) in [
        (Touch::Added, "added it"),
        (Touch::Modified, "modified it"),
        (Touch::Deleted, "deleted it"),
        (Touch::Renamed, "renamed it"),
    ] {
        let text = reason(
            &texts,
            &Issue::ProtectedChanged {
                path: path("Makefile"),
                change: touch,
            },
        )
        .unwrap();

        assert!(text.contains(word), "{word}: {text}");
    }
}

// ---- the message to the team --------------------------------------------------------------------------------------

#[tokio::test]
async fn the_team_is_told_the_pull_request_the_pass_and_every_reason() {
    let texts = texts().await;
    let issues = [Issue::NoTests, marker("src/a.rs", Marker::SkipsTest)];

    let message = team_message(&texts, pr(7), number(2), 3, &issues, &[]).unwrap();

    assert!(message.contains("pull request #7"), "{message}");
    assert!(message.contains("check 2"), "{message}");
    assert!(message.contains("3 such returns"), "{message}");
    for issue in &issues {
        assert!(message.contains(&reason(&texts, issue).unwrap()), "{message}");
    }
    assert!(!message.contains("refused"), "{message}");
    assert!(!message.contains("{{"), "{message}");
}

#[tokio::test]
async fn what_the_human_refused_is_told_to_the_team_with_each_subject() {
    let texts = texts().await;

    let message = team_message(
        &texts,
        pr(7),
        number(1),
        2,
        &[edited("tests/a.rs", 1)],
        &[file("tests/a.rs"), Subject::NoTests],
    )
    .unwrap();

    assert!(message.contains("refused to allow"), "{message}");
    assert!(message.contains("- `tests/a.rs`"), "{message}");
    assert!(message.contains("without a test of its own"), "{message}");
}

// ---- the questions ------------------------------------------------------------------------------------------------

#[tokio::test]
async fn a_question_is_asked_for_each_subject_in_the_order_the_issues_name_them() {
    let texts = texts().await;
    let issues = [
        edited("tests/b.rs", 1),
        Issue::NoTests,
        Issue::ProtectedChanged {
            path: path("AGENTS.md"),
            change: Touch::Modified,
        },
    ];

    let asked = questions(&texts, &team(), &task(), &issues, AskReason::OnlyTheHumanMayAllow, 0).unwrap();

    assert_eq!(
        asked.subjects,
        [file("tests/b.rs"), Subject::NoTests, file("AGENTS.md")]
    );
    let entries = asked.file.entries();
    assert_eq!(entries.len(), 3);
    for (position, entry) in entries.iter().enumerate() {
        assert_eq!(entry.number().get() as usize, position + 1);
        assert!(entry.verdict().is_none());
    }
    assert!(entries[0].title().contains("`tests/b.rs`"), "{}", entries[0].title());
    assert!(entries[1].title().contains("production code"), "{}", entries[1].title());
    assert!(entries[2].title().contains("`AGENTS.md`"), "{}", entries[2].title());
}

#[tokio::test]
async fn a_question_holds_the_reasons_for_its_subject_and_the_words_that_answer_it() {
    let texts = texts().await;
    let issue = edited("tests/a.rs", 4);

    let asked = questions(
        &texts,
        &team(),
        &task(),
        std::slice::from_ref(&issue),
        AskReason::OnlyTheHumanMayAllow,
        0,
    )
    .unwrap();

    let entry = &asked.file.entries()[0];
    assert!(
        entry.body().contains(&reason(&texts, &issue).unwrap()),
        "{}",
        entry.body()
    );
    assert!(entry.body().contains(&format!("`{ALLOW_WORD}`")), "{}", entry.body());
    assert!(entry.body().contains(&format!("`{REFUSE_WORD}`")), "{}", entry.body());
    assert!(entry.recommendation().unwrap().contains(REFUSE_WORD));
    assert_eq!(asked.file.missing_recommendations(), []);
}

#[tokio::test]
async fn several_issues_of_one_file_are_one_question_with_all_their_reasons() {
    let texts = texts().await;
    let issues = [
        marker("src/a.rs", Marker::SkipsTest),
        marker("src/a.rs", Marker::SuppressesLint),
    ];

    let asked = questions(&texts, &team(), &task(), &issues, AskReason::PassesUsed, 2).unwrap();

    assert_eq!(asked.subjects, [file("src/a.rs")]);
    let body = asked.file.entries()[0].body();
    for issue in &issues {
        assert!(body.contains(&reason(&texts, issue).unwrap()), "{body}");
    }
}

#[tokio::test]
async fn the_human_is_told_why_they_are_asked() {
    let texts = texts().await;
    let issues = [edited("tests/a.rs", 1)];

    let only = questions(&texts, &team(), &task(), &issues, AskReason::OnlyTheHumanMayAllow, 0).unwrap();
    let used = questions(&texts, &team(), &task(), &issues, AskReason::PassesUsed, 2).unwrap();

    assert!(only.file.entries()[0].body().contains("Only you may allow"));
    assert!(used.file.entries()[0].body().contains("2 time(s)"));
    assert!(!used.file.entries()[0].body().contains("Only you may allow"));
}

#[tokio::test]
async fn the_questions_file_names_the_team_and_the_task_and_reads_back_as_it_is_written() {
    let texts = texts().await;

    let asked = questions(
        &texts,
        &team(),
        &task(),
        &[Issue::NoTests, edited("tests/a.rs", 1)],
        AskReason::PassesUsed,
        1,
    )
    .unwrap();

    let title = asked.file.title().unwrap().to_owned();
    assert!(
        title.contains(team().as_str()) && title.contains(task().as_str()),
        "{title}"
    );
    let rendered = asked.file.render();
    assert_eq!(
        agent_core::questions::QuestionsFile::parse(&rendered).unwrap(),
        asked.file
    );
}

// ---- what an answer means -----------------------------------------------------------------------------------------

fn file_answered(subjects: &[Subject], verdicts: &[Option<&str>]) -> QuestionsFile {
    let entries = (1..=subjects.len())
        .map(|n| {
            agent_core::questions::Entry::new(number(u32::try_from(n).unwrap()), &format!("question {n}")).unwrap()
        })
        .collect();
    let mut file = QuestionsFile::new("Questions", entries).unwrap();
    for (position, verdict) in verdicts.iter().enumerate() {
        let verdict = verdict.map(|text| Verdict::from_text(text).unwrap().unwrap());
        file.set_verdict(number(u32::try_from(position).unwrap() + 1), verdict)
            .unwrap();
    }
    file
}

// Rule: only an explicit word of the human allows; anything else, `ok` and no answer included, refuses.
#[test]
fn only_the_word_allow_allows_what_a_question_names() {
    let subjects = [
        file("a.rs"),
        file("b.rs"),
        file("c.rs"),
        file("d.rs"),
        file("e.rs"),
        file("f.rs"),
        file("g.rs"),
    ];
    let file = file_answered(
        &subjects,
        &[
            Some("allow"),
            Some("ALLOW it, the test was wrong"),
            Some("Allow."),
            Some("refuse"),
            Some("ok"),
            Some("not yet, no"),
            Some("disallow"),
        ],
    );

    let decided = decisions(&subjects, &file);

    assert_eq!(
        decided.allowed,
        [file_subject("a.rs"), file_subject("b.rs"), file_subject("c.rs")]
    );
    assert_eq!(
        decided.refused,
        [
            file_subject("d.rs"),
            file_subject("e.rs"),
            file_subject("f.rs"),
            file_subject("g.rs")
        ]
    );
}

fn file_subject(text: &str) -> Subject {
    file(text)
}

#[test]
fn a_question_without_a_verdict_or_missing_from_the_file_is_refused() {
    let subjects = [file("a.rs"), file("b.rs"), file("c.rs")];
    // the file has two questions: the first is answered, the second is not, the third is gone
    let file = file_answered(&subjects[..2], &[Some("allow"), None]);

    let decided = decisions(&subjects, &file);

    assert_eq!(decided.allowed, [file_subject("a.rs")]);
    assert_eq!(decided.refused, [file_subject("b.rs"), file_subject("c.rs")]);
}

#[test]
fn a_question_of_the_file_that_names_no_subject_is_ignored() {
    let subjects = [file("a.rs")];
    let file = file_answered(
        &[file_subject("a.rs"), file_subject("b.rs")],
        &[Some("allow"), Some("allow")],
    );

    assert_eq!(decisions(&subjects, &file).allowed, [file_subject("a.rs")]);
}

/// A questions file with the questions `numbers`, in that order, each answered with its verdict.
fn file_numbered(answers: &[(u32, &str)]) -> QuestionsFile {
    let entries = answers
        .iter()
        .map(|(n, _)| agent_core::questions::Entry::new(number(*n), &format!("question {n}")).unwrap())
        .collect();
    let mut file = QuestionsFile::new("Questions", entries).unwrap();
    for (n, verdict) in answers {
        file.set_verdict(number(*n), Some(Verdict::from_text(verdict).unwrap().unwrap()))
            .unwrap();
    }
    file
}

// Rule: the answer to a subject is the verdict of the question with its number, wherever the question is in the file: a
// question the lead or the human removed passes its permission to no other subject.
#[test]
fn a_removed_question_is_refused_and_passes_its_permission_to_no_other_subject() {
    let subjects = [file("a.rs"), file("b.rs")];
    let file = file_numbered(&[(2, "allow")]);

    let decided = decisions(&subjects, &file);

    assert_eq!(decided.allowed, [file_subject("b.rs")]);
    assert_eq!(decided.refused, [file_subject("a.rs")]);
}

#[test]
fn questions_in_another_order_are_matched_by_their_numbers() {
    let subjects = [file("a.rs"), file("b.rs")];
    let file = file_numbered(&[(2, "allow"), (1, "refuse")]);

    let decided = decisions(&subjects, &file);

    assert_eq!(decided.allowed, [file_subject("b.rs")]);
    assert_eq!(decided.refused, [file_subject("a.rs")]);
}

#[test]
fn the_lack_of_tests_is_allowed_like_a_file() {
    let subjects = [Subject::NoTests];
    let file = file_answered(&subjects, &[Some("allow")]);

    assert_eq!(decisions(&subjects, &file).allowed, [Subject::NoTests]);
}

// ---- what the lead is told ----------------------------------------------------------------------------------------

#[tokio::test]
async fn the_lead_is_told_which_file_holds_the_questions_and_to_end_with_questions() {
    let texts = texts().await;

    let text = relay(&texts, &task(), number(3), Path::new("/home/tasks/questions-3.md")).unwrap();

    assert!(text.contains("`/home/tasks/questions-3.md`"), "{text}");
    assert!(text.contains("round 3"), "{text}");
    assert!(text.contains(task().as_str()), "{text}");
    assert!(text.contains("STATUS: QUESTIONS"), "{text}");
}

#[tokio::test]
async fn the_lead_is_told_after_the_answers_to_end_with_done() {
    let texts = texts().await;

    let text = answered(&texts).unwrap();

    assert!(text.contains("STATUS: DONE"), "{text}");
}

// ---- what the team wrote in the description -------------------------------------------------------------------------

// Rule: the lines the harness wrote are not the team's: what is left is what somebody added.
#[test]
fn what_the_harness_wrote_in_a_description_is_not_what_the_team_wrote() {
    let harness = "Work of the team.\n\n## 1.1 — The item\n\nChange src/lib.rs.\n\n## Reviews\n";
    let description = "Work of the team.\n\n## 1.1 — The item\n\nChange src/lib.rs.\n\n## Reviews\n\n## Tests changed after RED\n- abc1234\n";

    assert_eq!(
        authored(description, harness),
        "\n## Tests changed after RED\n- abc1234"
    );
}

#[test]
fn a_description_that_is_the_harnesss_own_has_nothing_the_team_wrote() {
    let harness = "Work of the team.\n\nChange src/lib.rs.\n";

    assert_eq!(authored(harness, harness).trim(), "");
    assert_eq!(authored("", harness), "");
}

#[test]
fn lines_are_compared_without_their_indentation_and_a_changed_line_is_the_teams() {
    let harness = "Change src/lib.rs.\nExpected result\n";
    let description = "  Change src/lib.rs.  \nChange src/lib.rs and src/other.rs.\n";

    assert_eq!(authored(description, harness), "Change src/lib.rs and src/other.rs.");
}

#[test]
fn a_description_without_the_harnesss_text_is_all_the_teams() {
    assert_eq!(authored("Only mine.\nTwo lines.", ""), "Only mine.\nTwo lines.");
}
