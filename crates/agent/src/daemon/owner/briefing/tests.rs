//! The briefing the daemon renders from its facts: the note on unread messages left out follows the bus's reader.

use std::num::NonZeroU64;
use std::path::Path;

use agent_core::config::Language;
use agent_core::questions::Verdict;
use agent_core::state::RunEnd;
use agent_core::status::RunOutcome;

use super::*;

/// The facts of a briefing whose unread messages are `completeness`.
fn facts(completeness: Completeness) -> BriefingFacts {
    BriefingFacts {
        reason: Trouble::NoStatus,
        base: "main".parse().unwrap(),
        branch: "agents/claude-1/task-20261120-144101-r7q2".parse().unwrap(),
        commits: Vec::new(),
        uncommitted: Vec::new(),
        plan: None,
        messages: vec![UnreadMessage {
            id: NonZeroU64::new(1508).unwrap(),
            from: "inv-opus".to_owned(),
            text: "Work on item 1.1 only.".to_owned(),
        }],
        completeness,
        snapshot: "refs/agents/snapshots/round-1/task-20261120-144101-r7q2/1".to_owned(),
    }
}

/// The briefing of `facts`, rendered through the catalog of the repository.
async fn render(facts: &BriefingFacts) -> String {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../prompts");
    let texts = HarnessPrompts::load(&dir, &Language::default()).await.unwrap();
    facts
        .render(&texts, &"task-20261120-144101-r7q2".parse().unwrap(), "Fix the parser.")
        .unwrap()
}

/// The note of the catalog on unread messages left out.
const NOTE: &str = "Older unread messages are left out of this list: read them all with `read_messages`";

#[tokio::test]
async fn a_briefing_whose_reader_gave_only_the_newest_messages_says_older_ones_are_left_out() {
    let text = render(&facts(Completeness::Newest)).await;

    assert!(text.contains(NOTE), "{text}");
    assert!(text.contains("- #1508 inv-opus:"), "{text}");
}

#[tokio::test]
async fn a_briefing_with_every_unread_message_has_no_note() {
    let text = render(&facts(Completeness::All)).await;

    assert!(!text.contains(NOTE), "{text}");
    assert!(text.contains("- #1508 inv-opus:"), "{text}");
}

#[test]
fn a_run_whose_end_was_never_recorded_is_told_as_cut() {
    assert_eq!(run_end_word(None), "cut before its end was recorded");
}

#[test]
fn each_way_a_run_ends_has_its_word_in_the_briefing() {
    let cases = [
        (
            RunEnd::Reported {
                outcome: RunOutcome::Done,
            },
            "DONE",
        ),
        (
            RunEnd::Reported {
                outcome: RunOutcome::Questions,
            },
            "QUESTIONS",
        ),
        (
            RunEnd::Reported {
                outcome: RunOutcome::Failed,
            },
            "FAILED",
        ),
        (RunEnd::NoStatus { exit_code: Some(1) }, "no status"),
        (RunEnd::Hung, "hang"),
        (RunEnd::TurnLimit, "turn limit"),
        (RunEnd::Stopped, "stopped"),
        (RunEnd::RateLimited, "failed"),
        (RunEnd::QuotaExhausted { resets_at: None }, "failed"),
    ];
    for (end, word) in cases {
        assert_eq!(run_end_word(Some(&end)), word, "{end:?}");
    }
}

#[test]
fn only_the_newest_items_are_shown_and_a_flag_says_older_ones_are_left_out() {
    assert_eq!(newest_shown(vec![1, 2, 3], 10), (vec![1, 2, 3], false));
    assert_eq!(newest_shown(vec![1, 2, 3, 4], 2), (vec![3, 4], true));
    assert_eq!(newest_shown(Vec::<u8>::new(), 2), (Vec::new(), false));
}

#[test]
fn an_accepted_verdict_is_ok_and_an_own_answer_is_its_text() {
    assert_eq!(verdict_word(&Verdict::Accept), "ok");
    let own = Verdict::from_text("No, drop it.").unwrap().unwrap();
    assert_eq!(verdict_word(&own), "No, drop it.");
}

/// A question file of the task with one answered question, numbered `number`.
fn question_file(number: u32, title: &str, verdict: &str) -> String {
    format!(
        "# Questions — task-20261120-144101-r7q2\n\n## {number}. {title}\na) Keep it — the format stays\nb) Drop it — simpler\n\n**Recommended:** a) — the format stays (agent)\n\n**Verdict:** {verdict}\n"
    )
}

#[tokio::test]
async fn the_answers_are_read_from_the_question_files_of_the_task_oldest_first() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("questions-2.md"),
        question_file(2, "Keep the old format?", "No, drop it."),
    )
    .unwrap();
    std::fs::write(
        dir.path().join("questions-1.md"),
        question_file(1, "Which parser?", "ok"),
    )
    .unwrap();
    std::fs::write(dir.path().join("notes.md"), "not a question file").unwrap();
    let task: TaskId = "task-20261120-144101-r7q2".parse().unwrap();

    let answers = read_answers(dir.path(), &task).await.unwrap();

    assert_eq!(
        answers,
        vec![
            AnswerLine {
                number: "1".to_owned(),
                title: "Which parser?".to_owned(),
                verdict: "ok".to_owned(),
            },
            AnswerLine {
                number: "2".to_owned(),
                title: "Keep the old format?".to_owned(),
                verdict: "No, drop it.".to_owned(),
            },
        ]
    );
}

#[tokio::test]
async fn a_task_without_a_folder_has_no_answers() {
    let dir = tempfile::tempdir().unwrap();
    let task: TaskId = "task-20261120-144101-r7q2".parse().unwrap();

    let answers = read_answers(&dir.path().join("missing"), &task).await.unwrap();

    assert_eq!(answers, Vec::new());
}

#[test]
fn every_reason_has_its_own_word_in_the_briefing() {
    for (reason, word) in [
        (Trouble::NoStatus, "no_status"),
        (Trouble::Hang, "hang"),
        (Trouble::TurnLimit, "turn_limit"),
        (Trouble::Busy, "busy"),
        (Trouble::DaemonDown, "daemon_down"),
    ] {
        assert_eq!(reason_word(reason), word);
    }
}

#[tokio::test]
async fn a_briefing_after_the_daemon_went_down_says_so_and_that_it_was_not_the_agents_fault() {
    let mut cut = facts(Completeness::All);
    cut.reason = Trouble::DaemonDown;

    let text = render(&cut).await;

    assert!(text.contains("`daemon_down`"), "{text}");
    assert!(
        text.contains("the daemon went down while the run was going. The run was cut from outside"),
        "{text}"
    );
}

// ---- the judgements that still hold ------------------------------------------------------------------------------

/// The plan of `text`.
fn plan_of(text: &str) -> Plan {
    parse_plan(text).unwrap()
}

/// The run `n` of these tests.
fn run_n(n: u32) -> RunId {
    format!("run-20261120-14410{n}-aaaa").parse().unwrap()
}

/// The judgement of step `step`, made at the end of run `run` (outside a run for `None`).
fn judgement(step: &str, state: JudgementState, run: Option<u32>) -> (ItemId, JudgementRecord) {
    let step: ItemId = step.parse().unwrap();
    let record = JudgementRecord {
        task: "task-20261120-144101-r7q2".parse().unwrap(),
        team: "team-20261120-144101-rjna".parse().unwrap(),
        run: run.map(run_n),
        at: "2026-11-20T20:00:00Z".parse().unwrap(),
        step,
        state,
        check: None,
        reason: None,
        duration_ms: 0,
        output_path: None,
    };
    (step, record)
}

/// What stands of `judged` for `current`, when the plan of run 1 was `at_run_1`.
fn standing(current: &str, at_run_1: &str, judged: Vec<(ItemId, JudgementRecord)>) -> Vec<(String, JudgementState)> {
    let plans = HashMap::from([(run_n(1), plan_of(at_run_1))]);
    standing_judgements(&plan_of(current), &judged.into_iter().collect(), &plans)
}

const WRITTEN: &str =
    "Do it.\n\n1. Work\n[x] 1.1 Write it\n    Check: exists a.txt\n[x] 1.2 Test it\n    Check: cargo test\n";

#[test]
fn a_judgement_stands_while_its_step_is_as_it_was_when_judged() {
    let judged = vec![
        judgement("1.1", JudgementState::Confirmed, Some(1)),
        judgement("1.2", JudgementState::Mismatch, Some(1)),
    ];

    assert_eq!(
        standing(WRITTEN, WRITTEN, judged),
        [
            ("1.1".to_owned(), JudgementState::Confirmed),
            ("1.2".to_owned(), JudgementState::Mismatch)
        ]
    );
}

#[test]
fn a_judgement_of_a_step_whose_check_or_title_changed_since_is_left_out() {
    let edited_check = WRITTEN.replace("cargo test", "cargo test -p agent");
    let edited_title = WRITTEN.replace("Write it", "Write it again");
    for current in [edited_check, edited_title] {
        let judged = vec![
            judgement("1.1", JudgementState::Confirmed, Some(1)),
            judgement("1.2", JudgementState::Confirmed, Some(1)),
        ];

        let kept = standing(&current, WRITTEN, judged);

        // Only the step that was not edited keeps its judgement.
        assert_eq!(kept.len(), 1, "{current}: {kept:?}");
    }
}

#[test]
fn a_judgement_whose_plan_was_not_kept_or_was_made_outside_a_run_is_left_out() {
    let judged = vec![
        judgement("1.1", JudgementState::Confirmed, Some(2)),
        judgement("1.2", JudgementState::Confirmed, None),
    ];

    assert_eq!(standing(WRITTEN, WRITTEN, judged), []);
}

#[test]
fn a_judgement_of_a_step_the_plan_no_longer_has_is_left_out() {
    let current = "Do it.\n\n1. Work\n[x] 1.1 Write it\n    Check: exists a.txt\n";
    let judged = vec![judgement("1.2", JudgementState::Confirmed, Some(1))];

    assert_eq!(standing(current, WRITTEN, judged), []);
}

#[test]
fn a_run_the_daemon_cut_is_not_told_as_an_agent_that_ended_without_a_status() {
    // Recovery records a run the daemon's stop or crash cut as ended with no status and no exit code.
    assert_eq!(
        run_end_word(Some(&RunEnd::NoStatus { exit_code: None })),
        "cut by a stop of the daemon"
    );
    assert_eq!(
        run_end_word(Some(&RunEnd::NoStatus { exit_code: Some(0) })),
        "no status"
    );
}

#[tokio::test]
async fn a_question_file_that_cannot_be_read_names_the_task_and_the_file() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("questions-1.md"),
        question_file(1, "Which parser?", "ok"),
    )
    .unwrap();
    std::fs::write(dir.path().join("questions-2.md"), [0xff, 0xfe, 0x00, 0x80]).unwrap();
    let task: TaskId = "task-20261120-144101-r7q2".parse().unwrap();

    let refused = read_answers(dir.path(), &task).await.unwrap_err();

    let BriefingProblem::Answers {
        task: named,
        path,
        source: _,
    } = refused
    else {
        panic!("expected the file to be refused: {refused:?}")
    };
    assert_eq!(named, task);
    assert_eq!(path, dir.path().join("questions-2.md"));
}

#[tokio::test]
async fn a_folder_that_cannot_be_listed_names_the_task_and_the_folder() {
    let dir = tempfile::tempdir().unwrap();
    let not_a_folder = dir.path().join("task");
    std::fs::write(&not_a_folder, "a file where the folder should be").unwrap();
    let task: TaskId = "task-20261120-144101-r7q2".parse().unwrap();

    let refused = read_answers(&not_a_folder, &task).await.unwrap_err();

    let BriefingProblem::AnswersDir {
        task: named,
        path,
        source: _,
    } = refused
    else {
        panic!("expected the folder to be refused: {refused:?}")
    };
    assert_eq!(named, task);
    assert_eq!(path, not_a_folder);
}

// ---- the whole briefing, with what the history gave ---------------------------------------------------------------

/// A draft whose history has two runs, two answers and a judgement of each kind, with the notes that older runs and
/// answers are left out.
fn draft_with_history() -> BriefingDraft {
    let mut facts = facts(Completeness::All);
    facts.plan = Some(vec![
        ("1.1".to_owned(), "Read the code".to_owned(), "[x]"),
        ("1.2".to_owned(), "Run the check".to_owned(), "[x]"),
        ("1.3".to_owned(), "Write the notes".to_owned(), "[x]"),
        ("1.4".to_owned(), "Write the test".to_owned(), "[ ]"),
    ]);
    BriefingDraft {
        facts,
        history: BriefingHistory {
            runs: vec![
                RunLine {
                    started: "2026-11-20 14:41".to_owned(),
                    agent: "claude-1".to_owned(),
                    end: "no status",
                },
                RunLine {
                    started: "2026-11-20 15:02".to_owned(),
                    agent: "claude-2".to_owned(),
                    end: "cut by a stop of the daemon",
                },
            ],
            runs_left_out: true,
            answers: vec![
                AnswerLine {
                    number: "1".to_owned(),
                    title: "Which parser?".to_owned(),
                    verdict: "ok".to_owned(),
                },
                AnswerLine {
                    number: "2".to_owned(),
                    title: "Keep the old format?".to_owned(),
                    verdict: "No, drop it.".to_owned(),
                },
            ],
            answers_left_out: true,
            judgements: vec![
                ("1.1".to_owned(), JudgementState::Confirmed),
                ("1.2".to_owned(), JudgementState::Mismatch),
            ],
        },
    }
}

/// The texts of the catalog.
async fn texts() -> HarnessPrompts {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../prompts");
    HarnessPrompts::load(&dir, &Language::default()).await.unwrap()
}

/// The lines that tell the briefing's parts, in the order the briefing gives them.
const WHAT_THE_HISTORY_GAVE: [&str; 10] = [
    "Older runs of the task are left out of this list",
    "- 2026-11-20 14:41 claude-1: no status",
    "- 2026-11-20 15:02 claude-2: cut by a stop of the daemon",
    "Confirmed (the check passed):\n- 1.1 Read the code",
    "Declared (the agent says so; no check ran, or it waits for a reviewer):\n- 1.3 Write the notes",
    "Mismatch (the agent says so and the check failed):\n- 1.2 Run the check",
    "Not done:\n- [ ] 1.4 Write the test",
    "Older answers of the human are left out of this list",
    "- Question 1, Which parser?: ok",
    "- Question 2, Keep the old format?: No, drop it.",
];

#[tokio::test]
async fn the_briefing_tells_the_runs_the_judged_steps_and_the_answers_the_history_gave_in_their_order() {
    let task: TaskId = "task-20261120-144101-r7q2".parse().unwrap();

    let text = draft_with_history()
        .render(&texts().await, &task, "Fix the parser.")
        .unwrap();

    let mut at = 0;
    for part in WHAT_THE_HISTORY_GAVE {
        let found = text[at..]
            .find(part)
            .unwrap_or_else(|| panic!("{part:?} is missing or out of order:\n{text}"));
        at += found + part.len();
    }
}

#[tokio::test]
async fn the_briefing_of_a_lead_tells_the_runs_the_judged_steps_and_the_answers_too() {
    let task: TaskId = "task-20261120-144101-r7q2".parse().unwrap();

    let text = draft_with_history().render_for_lead(&texts().await, &task).unwrap();

    for part in WHAT_THE_HISTORY_GAVE {
        assert!(text.contains(part), "{part:?} is missing:\n{text}");
    }
}
