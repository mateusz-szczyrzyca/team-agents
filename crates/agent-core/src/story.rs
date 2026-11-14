//! The story of a round or a team: the records of the history in, the lines a person reads out.
//!
//! A story is a pure function of the records it is given ([`render`]); `--json` of `agent log` prints the same records,
//! each once ([`json_lines`]). Both choose them with [`selected`], so the text and the JSON cannot disagree about what
//! belongs to a round. Records come in the order they were written in ([`SequencedRecord::sequence`]) and are told in the
//! order of their time, a tie going to the one written first.

mod json;
mod text;

use crate::history::{Record, SequencedRecord};
use crate::ids::{RoundId, TeamId};

/// How many lines of a prompt a story prints unless it is asked for the whole prompt.
pub const PREVIEW_LINES: usize = 5;

/// What a story is about.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Selection {
    /// One round: the records linked to it, and the start of its team.
    Round(RoundId),
    /// A team: all of its records; its rounds are told one after the other.
    Team(TeamId),
}

/// How much of a prompt a story prints.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PromptView {
    /// The first [`PREVIEW_LINES`] lines and the size.
    Preview,
    /// The whole prompt, as it was stored.
    Whole,
}

/// Whether a story lists the steps of the runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StepView {
    /// Every step, numbered, with its kind and time.
    Listed,
    /// Only how many there were.
    LeftOut,
}

/// What the human text of a story shows; the JSON is always whole.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StoryOptions {
    /// How much of a prompt.
    pub prompts: PromptView,
    /// Whether the steps.
    pub steps: StepView,
}

/// The records `selection` is about, each once, in the order of their time and, for the same time, of their writing.
#[must_use]
pub fn selected<'a>(records: &'a [SequencedRecord], selection: &Selection) -> Vec<&'a SequencedRecord> {
    let mut chosen: Vec<&SequencedRecord> = match selection {
        Selection::Team(team) => records.iter().filter(|record| &record.team == team).collect(),
        Selection::Round(round) => {
            let Some(owner) = records.iter().find(|record| record.rounds.contains(round)) else {
                return Vec::new();
            };
            records
                .iter()
                .filter(|record| {
                    record.rounds.contains(round)
                        || (matches!(record.record, Record::Team(_)) && record.team == owner.team)
                })
                .collect()
        }
    };
    chosen.sort_by_key(|record| (record.at, record.sequence));
    chosen.dedup_by_key(|record| record.sequence);
    chosen
}

/// The story of `selection` as text, ending with a newline; empty when no record belongs to it.
#[must_use]
pub fn render(records: &[SequencedRecord], selection: &Selection, options: StoryOptions) -> String {
    let chosen = selected(records, selection);
    if chosen.is_empty() {
        return String::new();
    }
    text::render(&chosen, selection, options)
}

/// The records of `selection`, each as one line of JSON, in the order of the story.
#[must_use]
pub fn json_lines(records: &[SequencedRecord], selection: &Selection) -> Vec<String> {
    selected(records, selection)
        .into_iter()
        .map(|record| json::record_json(record).to_string())
        .collect()
}
