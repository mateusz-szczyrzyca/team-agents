use agent_core::decide::outcome::{Classified, ExitStatus, RunFinish, StoppedBy};
use agent_runtime::run::{FinishedRun, Progress};

use super::{pool_run_end, pool_run_end_time};
use crate::daemon::owner::{Outcome, RunEnd, RunVerdict};
use crate::fixtures::at;

/// A finished run that ended with `end` and said `final_message`.
fn finished(end: RunEnd, final_message: Option<&str>) -> Outcome {
    Outcome::Verdict(Box::new(RunVerdict::Finished(Box::new(FinishedRun {
        classified: Classified { end, pause: None },
        finish: RunFinish {
            exit: ExitStatus::Code(0),
            stopped_by: StoppedBy::Nobody,
        },
        session: None,
        final_message: final_message.map(str::to_owned),
        progress: Progress::Made,
        used_tool: true,
    }))))
}

/// A clock set back during the run must not end the run before its request started.
#[test]
fn a_pool_run_never_ends_before_its_request_started() {
    assert_eq!(pool_run_end_time(at(5), Some(at(9))), at(9));
}

/// A clock that went on gives its own time, and a request with no known start leaves the clock alone.
#[test]
fn a_pool_run_ends_when_the_clock_says_after_its_start() {
    assert_eq!(pool_run_end_time(at(9), Some(at(5))), at(9));
    assert_eq!(pool_run_end_time(at(3), None), at(3));
}

/// A job whose engine failed ended with no status and no exit code, and carries no final message: the engine's error
/// is not what the agent said.
#[test]
fn a_failed_pool_run_ends_with_no_status_and_no_final_message() {
    let outcome = Outcome::Failed("the engine could not start the CLI".to_owned());

    let (end, message) = pool_run_end(&outcome);

    assert_eq!(end, RunEnd::NoStatus { exit_code: None });
    assert_eq!(message, None);
}

/// A run that went to its end records its classified end and what the agent said last.
#[test]
fn a_finished_pool_run_keeps_its_end_and_final_message() {
    let outcome = finished(RunEnd::NoStatus { exit_code: Some(3) }, Some("all done"));

    let (end, message) = pool_run_end(&outcome);

    assert_eq!(end, RunEnd::NoStatus { exit_code: Some(3) });
    assert_eq!(message, Some("all done"));
}

/// A run cut by a shutdown of the daemon ended with no status and no exit code.
#[test]
fn an_interrupted_pool_run_ends_with_no_status() {
    let outcome = Outcome::Verdict(Box::new(RunVerdict::Interrupted { session: None }));

    let (end, message) = pool_run_end(&outcome);

    assert_eq!(end, RunEnd::NoStatus { exit_code: None });
    assert_eq!(message, None);
}
