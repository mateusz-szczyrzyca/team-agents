//! The values of the briefing's variables for the runs, the steps as the harness judged them, and the answers of the
//! human, one table of cases per part.

use agent_core::plan::judge::JudgementState;
use agent_core::prompt::Variables;

use super::*;

/// The facts of a briefing of one task: nothing but the reason, the task and the snapshot, so that only `notes` decide
/// the parts under test.
fn facts(steps: Option<&'static [BriefingStep<'static>]>) -> Briefing<'static> {
    Briefing {
        reason: "no_status",
        task: "task-20261120-144101-r7q2",
        prompt: "Fix the parser.",
        base: "main",
        branch: "agents/claude-1/task-20261120-144101-r7q2-1.1",
        commits: &[],
        uncommitted: &[],
        plan: steps,
        messages: &[],
        left_out: None,
        snapshot: "refs/agents/snapshots/round-1/task-20261120-144101-r7q2/1",
    }
}

/// The headings of the groups of steps, as the catalog words them.
const HEADINGS: PlanHeadings<'static> = PlanHeadings {
    confirmed: "Confirmed (the check passed):",
    declared: "Declared (the agent says so; no check ran, or it waits for a reviewer):",
    mismatch: "Mismatch (the agent says so and the check failed):",
    not_done: "Not done:",
};

/// The value of the variable `name` in `values`, or `<missing>` when it has none.
fn value(values: &Variables, name: &str) -> String {
    let name = name.parse().expect("a variable name");
    values.get(&name).unwrap_or("<missing>").to_owned()
}

/// The variables of the briefing of `steps` with `notes`.
fn variables(steps: Option<&'static [BriefingStep<'static>]>, notes: &BriefingNotes<'_>) -> Variables {
    briefing_variables_with(&facts(steps), notes).expect("the variables of a briefing")
}

#[test]
fn runs_are_listed_oldest_first_with_their_agent_and_end() {
    let runs = [
        BriefingRun {
            started: "2026-11-20 14:41",
            agent: "claude-1",
            end: "no status",
        },
        BriefingRun {
            started: "2026-11-20 15:02",
            agent: "claude-2",
            end: "DONE",
        },
    ];
    let notes = BriefingNotes {
        runs: &runs,
        ..BriefingNotes::default()
    };

    assert_eq!(
        value(&variables(None, &notes), "runs"),
        "- 2026-11-20 14:41 claude-1: no status\n- 2026-11-20 15:02 claude-2: DONE"
    );
}

#[test]
fn a_task_with_no_runs_has_an_empty_runs_list() {
    let values = variables(None, &BriefingNotes::default());

    assert_eq!(value(&values, "runs"), "");
    assert_eq!(value(&values, "runs_left_out"), "");
}

#[test]
fn the_note_on_runs_left_out_is_passed_through() {
    let runs = [BriefingRun {
        started: "2026-11-20 15:02",
        agent: "claude-2",
        end: "hang",
    }];
    let notes = BriefingNotes {
        runs: &runs,
        runs_left_out: Some("Older runs are left out of this list.\n"),
        ..BriefingNotes::default()
    };

    assert_eq!(
        value(&variables(None, &notes), "runs_left_out"),
        "Older runs are left out of this list."
    );
}

#[test]
fn steps_are_grouped_as_confirmed_declared_mismatch_then_not_done() {
    static STEPS: [BriefingStep<'static>; 6] = [
        BriefingStep {
            id: "1.1",
            title: "Read the code",
            state: "[x]",
        },
        BriefingStep {
            id: "1.2",
            title: "Run the check",
            state: "[x]",
        },
        BriefingStep {
            id: "1.3",
            title: "Write the notes",
            state: "[x]",
        },
        BriefingStep {
            id: "1.4",
            title: "Write the test",
            state: "[ ]",
        },
        BriefingStep {
            id: "1.5",
            title: "Write the parser",
            state: "[>]",
        },
        BriefingStep {
            id: "1.6",
            title: "Drop the old parser",
            state: "[-]",
        },
    ];
    let judgements = [("1.1", JudgementState::Confirmed), ("1.2", JudgementState::Mismatch)];
    let notes = BriefingNotes {
        judgements: &judgements,
        headings: HEADINGS,
        ..BriefingNotes::default()
    };

    assert_eq!(
        value(&variables(Some(&STEPS), &notes), "plan"),
        "Confirmed (the check passed):\n- 1.1 Read the code\n\
         Declared (the agent says so; no check ran, or it waits for a reviewer):\n- 1.3 Write the notes\n\
         Mismatch (the agent says so and the check failed):\n- 1.2 Run the check\n\
         Not done:\n- [ ] 1.4 Write the test\n- [>] 1.5 Write the parser\n- [-] 1.6 Drop the old parser"
    );
}

#[test]
fn a_plan_with_only_done_steps_has_no_empty_group_and_no_not_done_part() {
    static STEPS: [BriefingStep<'static>; 1] = [BriefingStep {
        id: "1.1",
        title: "Read the code",
        state: "[x]",
    }];

    let notes = BriefingNotes {
        headings: HEADINGS,
        ..BriefingNotes::default()
    };

    assert_eq!(
        value(&variables(Some(&STEPS), &notes), "plan"),
        "Declared (the agent says so; no check ran, or it waits for a reviewer):\n- 1.1 Read the code"
    );
}

#[test]
fn a_task_without_plan_has_an_empty_plan() {
    assert_eq!(value(&variables(None, &BriefingNotes::default()), "plan"), "");
}

#[test]
fn the_human_answers_are_listed_with_their_verdicts() {
    let answers = [
        BriefingAnswer {
            number: "1",
            title: "Which parser to use?",
            verdict: "ok",
        },
        BriefingAnswer {
            number: "2",
            title: "Keep the old format?",
            verdict: "No, drop it.",
        },
    ];
    let notes = BriefingNotes {
        answers: &answers,
        ..BriefingNotes::default()
    };

    assert_eq!(
        value(&variables(None, &notes), "answers"),
        "- Question 1, Which parser to use?: ok\n- Question 2, Keep the old format?: No, drop it."
    );
}

#[test]
fn a_task_without_answers_has_an_empty_answers_list() {
    let values = variables(None, &BriefingNotes::default());

    assert_eq!(value(&values, "answers"), "");
    assert_eq!(value(&values, "answers_left_out"), "");
}

#[test]
fn the_note_on_answers_left_out_is_passed_through() {
    let answers = [BriefingAnswer {
        number: "7",
        title: "Which parser?",
        verdict: "ok",
    }];
    let notes = BriefingNotes {
        answers: &answers,
        answers_left_out: Some("Older answers are left out of this list.\n"),
        ..BriefingNotes::default()
    };

    assert_eq!(
        value(&variables(None, &notes), "answers_left_out"),
        "Older answers are left out of this list."
    );
}

#[test]
fn a_step_the_harness_judged_declared_is_listed_as_declared() {
    static STEPS: [BriefingStep<'static>; 1] = [BriefingStep {
        id: "1.1",
        title: "Ask the reviewer",
        state: "[x]",
    }];
    let judgements = [("1.1", JudgementState::Declared)];
    let notes = BriefingNotes {
        judgements: &judgements,
        headings: HEADINGS,
        ..BriefingNotes::default()
    };

    assert_eq!(
        value(&variables(Some(&STEPS), &notes), "plan"),
        "Declared (the agent says so; no check ran, or it waits for a reviewer):\n- 1.1 Ask the reviewer"
    );
}
