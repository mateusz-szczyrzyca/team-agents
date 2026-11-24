//! The reminder of Serena of a run that ended, read back from the history of its team.

use agent_core::history::{HistoryEvent, Record, SequencedRecord};
use agent_core::ids::{RunId, TeamId};
use agent_core::kinds::CliKind;
use agent_runtime::history::{ReadError, Reader};

/// The reminder of Serena that `history` holds for the run `run` of `team` that has ended: the count of searches of code
/// without Serena it came at, and the model CLI of the run.
///
/// # Errors
/// [`ReadError`] when the history of the team cannot be read: no reminder is delivered then, and the caller says why.
pub(crate) fn reminder_of(history: &Reader, run: &RunId, team: &TeamId) -> Result<Option<(u64, CliKind)>, ReadError> {
    let records = history.team_records(team)?;
    Ok(within(&records, run))
}

/// The reminder of `run` among `records` of its team, as [`reminder_of`] describes it.
fn within(records: &[SequencedRecord], run: &RunId) -> Option<(u64, CliKind)> {
    reminder_at(records, run, 0)
}

/// The reminder of `run` among `records` written from `from` on: its last record, before the first start of another run of
/// the team after it. A task starts its next run only after this one ended, and the reminder of that next run comes after its
/// start.
fn reminder_at(records: &[SequencedRecord], run: &RunId, from: usize) -> Option<(u64, CliKind)> {
    let mut reminder = None;
    for stored in records.iter().skip(from) {
        match &stored.record {
            Record::Event(event) => {
                if let HistoryEvent::SerenaReminded {
                    run: told,
                    searches,
                    cli,
                    ..
                } = &event.event
                    && told == run
                {
                    reminder = Some((*searches, *cli));
                }
            }
            Record::Run(start) if start.run != *run && reminder.is_some() => return reminder,
            Record::Team(_)
            | Record::Round(_)
            | Record::Run(_)
            | Record::Launch(_)
            | Record::Step(_)
            | Record::RunEnd(_)
            | Record::Changes(_)
            | Record::Snapshot(_)
            | Record::Session(_) => {}
        }
    }
    reminder
}
