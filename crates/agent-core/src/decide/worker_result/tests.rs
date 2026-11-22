//! The table of [`super::decide`] and of [`super::worker_for_second_try`]: one row for each outcome of a worker's
//! result, and each free worker a second try may go to.

#![cfg(test)]

use super::*;

/// The facts of a worker whose check passed, that changed only the request's files and ended `DONE` in time: the
/// first try of a request that writes files, with both tries left.
fn passing() -> Facts {
    Facts {
        ending: WorkerEnding::Done,
        check: CheckOutcome::Passed,
        changed_outside_files: false,
        past_seconds: false,
        scope: Scope::Files,
        try_number: 1,
        tries: 2,
        elsewhere: Elsewhere::Free,
    }
}

/// A table row: what is changed in [`passing`], and what the harness does with it.
struct Row {
    /// What the row shows.
    what: &'static str,
    /// The facts of the row.
    facts: Facts,
    /// What the harness does with them.
    verdict: Verdict,
}

/// A change of one fact of [`passing`].
type Change = fn(&mut Facts);

/// Why a result does not serve the request.
fn give(wrong: Vec<Wrong>) -> Verdict {
    Verdict::GiveToAnother { wrong }
}

/// Why the second result, or a request with no retry left, does not serve it.
fn ret(wrong: Vec<Wrong>) -> Verdict {
    Verdict::Return { wrong }
}

/// A row of the table: `what` changes `passing` by `change` and expects `verdict`.
fn row(what: &'static str, change: Change, verdict: Verdict) -> Row {
    let mut facts = passing();
    change(&mut facts);
    Row { what, facts, verdict }
}

/// The rows whose result serves the request or ends its first try.
fn first_try_rows() -> Vec<Row> {
    let failed = |facts: &mut Facts| facts.check = CheckOutcome::Failed;
    vec![
        row("the check passed", |_| {}, Verdict::Accept),
        row("the check failed", failed, give(vec![Wrong::CheckFailed])),
        row(
            "the check ran past its limit",
            |facts| facts.check = CheckOutcome::TimedOut,
            give(vec![Wrong::CheckTimedOut]),
        ),
        row(
            "the run ended FAILED",
            |facts| facts.ending = WorkerEnding::Failed,
            give(vec![Wrong::EndedOtherwise(WorkerEnding::Failed)]),
        ),
        row(
            "the run ended without a status",
            |facts| facts.ending = WorkerEnding::NoStatus,
            give(vec![Wrong::EndedOtherwise(WorkerEnding::NoStatus)]),
        ),
        row(
            "the worker said it cannot",
            |facts| facts.ending = WorkerEnding::Cannot,
            give(vec![Wrong::EndedOtherwise(WorkerEnding::Cannot)]),
        ),
        row(
            "a file outside files",
            |facts| facts.changed_outside_files = true,
            give(vec![Wrong::OutsideFiles]),
        ),
        row(
            "the run passed [workers] seconds",
            |facts| facts.past_seconds = true,
            give(vec![Wrong::PastSeconds]),
        ),
        row(
            "a request that names files but has no check: nothing shows the commit right",
            |facts| facts.check = CheckOutcome::Absent,
            give(vec![Wrong::NoCheck]),
        ),
    ]
}

/// The rows of the second try, of the limits on it, of several reasons, and of requests for facts only.
fn more_rows() -> Vec<Row> {
    vec![
        row(
            "the second try's check failed",
            |facts| {
                facts.check = CheckOutcome::Failed;
                facts.try_number = 2;
            },
            ret(vec![Wrong::CheckFailed]),
        ),
        row("the second try passed", |facts| facts.try_number = 2, Verdict::Accept),
        row(
            "the check failed with tries = 1",
            |facts| {
                facts.check = CheckOutcome::Failed;
                facts.tries = 1;
            },
            ret(vec![Wrong::CheckFailed]),
        ),
        row(
            "the check failed with no other provider free",
            |facts| {
                facts.check = CheckOutcome::Failed;
                facts.elsewhere = Elsewhere::None;
            },
            ret(vec![Wrong::CheckFailed]),
        ),
        row(
            "the check failed, with a file outside files and a run past its seconds too",
            |facts| {
                facts.check = CheckOutcome::Failed;
                facts.changed_outside_files = true;
                facts.past_seconds = true;
            },
            give(vec![Wrong::OutsideFiles, Wrong::PastSeconds, Wrong::CheckFailed]),
        ),
        row(
            "the worker failed and a file is outside files",
            |facts| {
                facts.ending = WorkerEnding::Failed;
                facts.changed_outside_files = true;
            },
            give(vec![Wrong::EndedOtherwise(WorkerEnding::Failed), Wrong::OutsideFiles]),
        ),
        row(
            "every reason, on the second try",
            |facts| {
                facts.ending = WorkerEnding::Cannot;
                facts.check = CheckOutcome::TimedOut;
                facts.changed_outside_files = true;
                facts.past_seconds = true;
                facts.try_number = 2;
            },
            ret(vec![
                Wrong::EndedOtherwise(WorkerEnding::Cannot),
                Wrong::OutsideFiles,
                Wrong::PastSeconds,
                Wrong::CheckTimedOut,
            ]),
        ),
        row(
            "a facts-only request whose check passed",
            |facts| {
                facts.scope = Scope::Facts;
                facts.check = CheckOutcome::Passed;
            },
            Verdict::Accept,
        ),
        row(
            "a facts-only request whose check failed",
            |facts| {
                facts.scope = Scope::Facts;
                facts.check = CheckOutcome::Failed;
            },
            give(vec![Wrong::CheckFailed]),
        ),
        row(
            "a facts-only request with no check",
            |facts| {
                facts.scope = Scope::Facts;
                facts.check = CheckOutcome::Absent;
            },
            Verdict::Accept,
        ),
    ]
}

/// Every row of the table.
fn rows() -> Vec<Row> {
    let mut rows = first_try_rows();
    rows.extend(more_rows());
    rows
}

#[test]
fn the_decision_of_each_row_matches_its_verdict() {
    for row in rows() {
        assert_eq!(decide(&row.facts), row.verdict, "{}", row.what);
    }
}

#[test]
fn every_wrong_of_a_request_has_its_row() {
    let covered: Vec<Wrong> = rows()
        .iter()
        .flat_map(|row| match &row.verdict {
            Verdict::GiveToAnother { wrong } | Verdict::Return { wrong } => wrong.clone(),
            Verdict::Accept => Vec::new(),
        })
        .collect();
    let endings = [WorkerEnding::Failed, WorkerEnding::NoStatus, WorkerEnding::Cannot];

    let mut expected: Vec<Wrong> = endings.iter().map(|ending| Wrong::EndedOtherwise(*ending)).collect();
    expected.extend([
        Wrong::CheckFailed,
        Wrong::CheckTimedOut,
        Wrong::OutsideFiles,
        Wrong::PastSeconds,
    ]);
    for wrong in expected {
        assert!(covered.contains(&wrong), "no row has {wrong:?}");
    }
}

#[test]
fn a_retry_goes_to_a_same_provider_worker_when_no_other_is_free() {
    let same_provider = |tries: u32, try_number: u32| Facts {
        check: CheckOutcome::Failed,
        elsewhere: Elsewhere::SameProvider,
        tries,
        try_number,
        ..passing()
    };
    let table = [
        (
            "first try, two tries",
            same_provider(2, 1),
            give(vec![Wrong::CheckFailed]),
        ),
        (
            "first try, tries = 1",
            same_provider(1, 1),
            ret(vec![Wrong::CheckFailed]),
        ),
        ("second try", same_provider(2, 2), ret(vec![Wrong::CheckFailed])),
    ];
    for (what, facts, verdict) in table {
        assert_eq!(decide(&facts), verdict, "{what}");
    }
}

#[test]
fn a_retry_with_no_free_worker_returns_the_step_at_once() {
    let facts = Facts {
        check: CheckOutcome::Failed,
        elsewhere: Elsewhere::None,
        ..passing()
    };

    assert_eq!(decide(&facts), ret(vec![Wrong::CheckFailed]));
}

#[test]
fn a_request_for_files_without_a_check_is_never_accepted_and_one_for_facts_is() {
    let no_check = |scope: Scope, try_number: u32| Facts {
        check: CheckOutcome::Absent,
        scope,
        try_number,
        ..passing()
    };
    let table = [
        (
            "files, first try",
            no_check(Scope::Files, 1),
            give(vec![Wrong::NoCheck]),
        ),
        (
            "files, second try",
            no_check(Scope::Files, 2),
            ret(vec![Wrong::NoCheck]),
        ),
        ("facts only", no_check(Scope::Facts, 1), Verdict::Accept),
    ];
    for (what, facts, verdict) in table {
        assert_eq!(decide(&facts), verdict, "{what}");
    }
}

/// The name of a worker of the second table.
fn worker(name: &str) -> AgentName {
    name.parse().unwrap_or_else(|error| panic!("{name}: {error}"))
}

#[test]
fn the_second_try_of_a_table_goes_to_the_worker_its_row_names() {
    let alpha = worker("alpha");
    let beta = worker("beta");
    let gamma = worker("gamma");
    let delta = worker("delta");
    let listed = [
        (&alpha, CliKind::Claude),
        (&beta, CliKind::Claude),
        (&gamma, CliKind::Codex),
        (&delta, CliKind::Opencode),
    ];

    let table: [(&str, CliKind, [&str; 4], Option<&AgentName>); 5] = [
        (
            "the first free worker of another provider",
            CliKind::Claude,
            ["", "", "", ""],
            Some(&gamma),
        ),
        (
            "another provider's worker busy: the next provider",
            CliKind::Claude,
            ["", "", "gamma", ""],
            Some(&delta),
        ),
        (
            "no free worker of another provider, another one of the same: its",
            CliKind::Claude,
            ["alpha", "", "gamma", "delta"],
            Some(&beta),
        ),
        (
            "the first free worker of another provider, in the configured order",
            CliKind::Opencode,
            ["", "", "", ""],
            Some(&alpha),
        ),
        (
            "nobody free",
            CliKind::Claude,
            ["alpha", "beta", "gamma", "delta"],
            None,
        ),
    ];
    for (what, failed, busy, expected) in table {
        let busy: Vec<AgentName> = busy
            .iter()
            .filter(|name| !name.is_empty())
            .map(|name| worker(name))
            .collect();
        assert_eq!(worker_for_second_try(&listed, failed, &busy), expected, "{what}");
    }
}
