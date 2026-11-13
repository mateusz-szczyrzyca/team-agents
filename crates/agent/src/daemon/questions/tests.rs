use std::path::Path;

use agent_core::config::Language;
use agent_core::protocol::{AnswerInput, AnswerList, FailureKind};
use agent_core::questions::{AnswerProblem, Verdict};

use super::*;
use crate::fixtures::task;

/// A file with a question that has a recommendation and one that has none, both unanswered.
const ASKED: &str = "\
# Questions — fake

## 1. Which store?
a) memory
b) disk

**Recommended:** a) — simpler (fake)

**Verdict:**

## 2. Which name?

**Verdict:**
";

fn n(number: u32) -> NonZeroU32 {
    NonZeroU32::new(number).unwrap()
}

fn file() -> QuestionsFile {
    QuestionsFile::parse(ASKED).unwrap()
}

fn verdict(text: &str) -> Verdict {
    Verdict::from_text(text).unwrap().unwrap()
}

fn answers(given: &[(u32, &str)]) -> AnswerList {
    AnswerList::try_from(
        given
            .iter()
            .map(|(question, text)| AnswerInput {
                question: n(*question),
                verdict: verdict(text),
            })
            .collect::<Vec<_>>(),
    )
    .unwrap()
}

async fn texts() -> QuestionsPrompts {
    QuestionsPrompts::load(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("../../prompts"),
        &Language::default(),
    )
    .await
    .unwrap()
}

#[test]
fn the_answers_become_the_verdicts_of_the_file_and_it_is_marked_answered() {
    let answered = apply(&file(), &answers(&[(1, "ok"), (2, "Bile, please")])).unwrap();

    assert!(answered.is_answered());
    let verdicts: Vec<_> = answered
        .entries()
        .iter()
        .map(|entry| entry.verdict().cloned())
        .collect();
    assert_eq!(verdicts, [Some(verdict("ok")), Some(verdict("Bile, please"))]);
}

#[test]
fn applying_the_answers_leaves_the_file_it_was_given_as_it_was() {
    let asked = file();

    apply(&asked, &answers(&[(1, "ok"), (2, "x")])).unwrap();

    assert_eq!(asked, file());
}

#[test]
fn every_question_the_file_does_not_have_is_named_and_nothing_is_applied() {
    let refusal = apply(&file(), &answers(&[(1, "ok"), (7, "x"), (9, "y")])).unwrap_err();

    assert_eq!(
        refusal,
        Refusal::UnknownQuestions {
            numbers: vec![n(7), n(9)]
        }
    );
}

#[test]
fn a_question_left_without_a_verdict_is_listed_with_its_problem() {
    let refusal = apply(&file(), &answers(&[(1, "ok")])).unwrap_err();

    assert_eq!(
        refusal,
        Refusal::Incomplete {
            problems: vec![(n(2), AnswerProblem::MissingVerdict)]
        }
    );
}

#[test]
fn ok_for_a_question_without_a_recommendation_is_refused() {
    let refusal = apply(&file(), &answers(&[(1, "ok"), (2, "ok")])).unwrap_err();

    assert_eq!(
        refusal,
        Refusal::Incomplete {
            problems: vec![(n(2), AnswerProblem::AcceptWithoutRecommendation)]
        }
    );
}

#[test]
fn a_verdict_the_human_wrote_in_the_file_stays_when_the_answers_do_not_mention_the_question() {
    let written = ASKED.replacen("**Verdict:**\n", "**Verdict:** disk\n", 2);
    let written = QuestionsFile::parse(&written).unwrap();

    let answered = apply(&written, &answers(&[(1, "ok")])).unwrap();

    assert_eq!(answered.entries()[1].verdict(), Some(&verdict("disk")));
}

#[test]
fn the_failure_for_a_file_without_a_question_names_the_numbers() {
    let failure = Refusal::UnknownQuestions {
        numbers: vec![n(7), n(9)],
    }
    .failure(&file());

    assert_eq!(failure.kind, FailureKind::Invalid);
    assert!(
        failure.message.contains('7') && failure.message.contains('9'),
        "{}",
        failure.message
    );
}

#[test]
fn the_failure_for_incomplete_answers_lists_each_question_with_its_title_and_problem() {
    let failure = Refusal::Incomplete {
        problems: vec![
            (n(1), AnswerProblem::AcceptWithoutRecommendation),
            (n(2), AnswerProblem::MissingVerdict),
        ],
    }
    .failure(&file());

    assert_eq!(failure.kind, FailureKind::Invalid);
    assert_eq!(
        failure.message,
        "the answers are not complete:\n\
         - question 1 (Which store?): `ok` but no recommendation to accept\n\
         - question 2 (Which name?): no verdict"
    );
}

#[tokio::test]
async fn the_instructions_of_the_template_are_put_under_the_title_with_the_task() {
    let prepared = with_instructions(&file(), &texts().await, &task(1)).unwrap();

    assert!(
        prepared.instructions().contains(&format!("agent answer {}", task(1))),
        "{}",
        prepared.instructions()
    );
    assert_eq!(QuestionsFile::parse(&prepared.render()).unwrap(), prepared);
}

#[tokio::test]
async fn the_instructions_are_not_added_twice() {
    let texts = texts().await;
    let once = with_instructions(&file(), &texts, &task(1)).unwrap();

    let twice = with_instructions(&once, &texts, &task(1)).unwrap();

    assert_eq!(once, twice);
}

#[tokio::test]
async fn what_the_task_is_told_has_the_round_the_answers_and_the_words_of_an_accepted_recommendation() {
    let answered = apply(&file(), &answers(&[(1, "ok"), (2, "Bile, please")])).unwrap();

    let prompt = answers_prompt(&answered, n(3), &texts().await).unwrap();

    assert!(prompt.contains("round 3"), "{prompt}");
    assert!(prompt.contains("Bile, please"), "{prompt}");
    assert!(
        prompt.contains("the human accepts the recommendation: a) — simpler (fake)"),
        "{prompt}"
    );
    assert!(!prompt.contains("**ANSWERED**"), "{prompt}");
}

#[tokio::test]
async fn a_file_that_is_not_answered_is_not_handed_to_the_task() {
    let error = answers_prompt(&file(), n(1), &texts().await).unwrap_err();

    assert!(matches!(error, TextError::Expand(_)), "{error:?}");
}

fn marked(text: &str) -> QuestionsFile {
    let mut file = QuestionsFile::parse(text).unwrap();
    file.mark_answered();
    file
}

#[test]
fn a_file_without_the_marker_is_not_answered_yet_whatever_it_says() {
    let written = ASKED.replacen("**Verdict:**\n", "**Verdict:** ok\n", 1);

    assert_eq!(judge(&QuestionsFile::parse(&written).unwrap()), Judgement::NotYet);
}

#[test]
fn a_marked_file_with_a_verdict_for_every_question_is_answered() {
    let written =
        ASKED
            .replacen("**Verdict:**\n", "**Verdict:** ok\n", 1)
            .replacen("**Verdict:**\n", "**Verdict:** disk\n", 1);

    assert_eq!(judge(&marked(&written)), Judgement::Answered);
}

#[test]
fn a_marked_file_with_a_question_unanswered_lists_it() {
    assert_eq!(
        judge(&marked(ASKED)),
        Judgement::Incomplete {
            problems: vec![
                (n(1), AnswerProblem::MissingVerdict),
                (n(2), AnswerProblem::MissingVerdict)
            ]
        }
    );
}

#[test]
fn a_marked_file_that_accepts_a_recommendation_there_is_none_for_lists_it() {
    let written = ASKED.replacen("**Verdict:**\n", "**Verdict:** ok\n", 2);

    assert_eq!(
        judge(&marked(&written)),
        Judgement::Incomplete {
            problems: vec![(n(2), AnswerProblem::AcceptWithoutRecommendation)]
        }
    );
}

#[tokio::test]
async fn the_notification_of_incomplete_answers_names_the_agent_the_task_the_file_and_each_problem() {
    let file = marked(ASKED);
    let problems = [(n(2), AnswerProblem::MissingVerdict)];

    let note = incomplete_notification(
        &texts().await,
        "claude-1",
        &task(1),
        Path::new("/h/tasks/t/questions-1.md"),
        &describe(&file, &problems),
    )
    .unwrap();

    assert!(
        note.title().contains("claude-1") && note.title().contains(task(1).as_str()),
        "{}",
        note.title()
    );
    assert!(note.body().contains("/h/tasks/t/questions-1.md"), "{}", note.body());
    assert!(
        note.body().contains("question 2 (Which name?): no verdict"),
        "{}",
        note.body()
    );
}
