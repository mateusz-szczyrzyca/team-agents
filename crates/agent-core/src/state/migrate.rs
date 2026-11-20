//! Reading schemas 1 through 11 and reconstructing their investigator activity history.
//!
//! Version 1 also gains rounds; versions before 10 preserve `Allow:` lines as brief text. The latest retained
//! task evidence supplies deterministic activity timestamps when older opinion records lack exact times.

use std::collections::HashSet;

use serde::Deserialize;
use serde_json::Value;

use super::team::{LegacyError, TeamV1};
use super::{InvalidState, SchemaVersion, State, StateError, StateWire, Team};
use crate::ids::RoundId;
use crate::task_list::{AllowLines, TaskList};

/// The schema version this module reads.
pub(super) const LEGACY_VERSION: u32 = 1;

/// A state as version 1 wrote it.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct StateV1 {
    /// Always [`LEGACY_VERSION`]: the caller has read it.
    #[expect(
        dead_code,
        reason = "read so that a state without it is refused; its value was checked by the caller"
    )]
    schema_version: u32,
    /// The teams, oldest first.
    teams: Vec<TeamV1>,
}

/// Reads schema version 1 into the current state, including reconstructed rounds and activities.
///
/// # Errors
/// [`StateError`]: the text is not a state of version 1 (a field missing, unknown or not valid, or a team whose parts
/// do not fit together), or it has no free ID for a round.
pub(super) fn from_v1(text: &str) -> Result<State, StateError> {
    let mut value: Value = serde_json::from_str(text).map_err(|source| StateError::Malformed { source })?;
    adapt_members(&mut value).map_err(|source| StateError::Malformed { source })?;
    keep_allow_lines_in_briefs(&mut value);
    let old: StateV1 = serde_json::from_value(value).map_err(|source| StateError::Malformed { source })?;
    let mut used: HashSet<RoundId> = HashSet::new();
    let mut teams = Vec::with_capacity(old.teams.len());
    for team in old.teams {
        teams.push(Team::from_v1(team, &mut used).map_err(StateError::from)?);
    }
    rebuild_activities(&mut teams);
    Ok(State::try_from(StateWire {
        schema_version: SchemaVersion,
        teams,
        wake_deliveries: std::collections::BTreeMap::new(),
    })?)
}

/// Marks a task list of an earlier version that starts with a line `Allow:` as one whose lines are brief: those
/// versions did not read the line as a permission, so it must neither refuse the state nor grant anything now, and
/// its text stays as it was.
fn keep_allow_lines_in_briefs(value: &mut Value) {
    let Some(teams) = value.get_mut("teams").and_then(Value::as_array_mut) else {
        return;
    };
    for team in teams {
        let Some(list) = team.get_mut("task_list") else {
            continue;
        };
        let Some(text) = list.as_str() else {
            continue;
        };
        if TaskList::has_allow_lines(text) {
            *list = serde_json::json!({ "text": text, "allow_lines": AllowLines::Brief });
        }
    }
}

/// Makes the members of the state `value` fit version 2 where the text of version 1 does not: every question gets the
/// key `number`, which version 1 called `round`, and a run that names rounds, which only version 2 has, is refused as
/// the unknown field it is for version 1.
fn adapt_members(value: &mut Value) -> Result<(), serde_json::Error> {
    if let Some(teams) = value.get_mut("teams").and_then(Value::as_array_mut) {
        for team in teams {
            adapt_team(team)?;
        }
    }
    Ok(())
}

/// Adapts the members of the team `team`.
fn adapt_team(team: &mut Value) -> Result<(), serde_json::Error> {
    if let Some(Value::Object(crew)) = team.get_mut("crew") {
        for (key, entry) in crew.iter_mut() {
            match (key.as_str(), entry) {
                ("member" | "coordinator", member) => adapt_member(member)?,
                ("members" | "workers", Value::Array(list)) => {
                    for member in list {
                        adapt_member(member)?;
                    }
                }
                _ => {}
            }
        }
    }
    if let Some(Value::Array(reviewers)) = team.get_mut("reviewers") {
        for member in reviewers {
            adapt_member(member)?;
        }
    }
    Ok(())
}

/// Adapts the questions and the runs of the member `member`. A question that has a `number` already is left alone:
/// reading it then refuses the key `round` as one a question does not have.
fn adapt_member(member: &mut Value) -> Result<(), serde_json::Error> {
    if let Some(Value::Array(runs)) = member.get("runs")
        && runs.iter().any(|run| run.get("rounds").is_some())
    {
        return Err(serde::de::Error::custom(
            "unknown field `rounds` in a run of schema version 1",
        ));
    }
    let Some(Value::Array(questions)) = member.get_mut("questions") else {
        return Ok(());
    };
    for question in questions {
        let Some(question) = question.as_object_mut() else {
            continue;
        };
        if !question.contains_key("number")
            && let Some(number) = question.remove("round")
        {
            question.insert("number".to_owned(), number);
        }
    }
    Ok(())
}

impl From<LegacyError> for StateError {
    fn from(error: LegacyError) -> Self {
        match error {
            LegacyError::NoFreeId { team, item } => Self::Invalid(InvalidState::NoFreeRoundId { team, item }),
            other => Self::Malformed {
                source: serde::de::Error::custom(other),
            },
        }
    }
}

/// Reads versions 2 through 10; only versions before 10 keep allowances as brief text.
pub(super) fn from_v2(text: &str) -> Result<State, StateError> {
    let mut value: Value = serde_json::from_str(text).map_err(|source| StateError::Malformed { source })?;
    if value
        .get("schema_version")
        .and_then(Value::as_u64)
        .is_some_and(|version| version < 10)
    {
        keep_allow_lines_in_briefs(&mut value);
    }
    if let Some(version) = value.get_mut("schema_version") {
        *version = Value::from(super::SCHEMA_VERSION);
    }
    let mut wire: StateWire = serde_json::from_value(value).map_err(|source| StateError::Malformed { source })?;
    rebuild_activities(&mut wire.teams);
    Ok(State::try_from(wire)?)
}

/// Uses the latest retained run evidence when old records lack an ending timestamp.
fn rebuild_activities(teams: &mut [Team]) {
    for team in teams {
        let at = team
            .crew()
            .roster()
            .into_iter()
            .map(|(_, member)| member)
            .chain(team.reviewers())
            .flat_map(super::Member::runs)
            .map(|run| match run.state() {
                super::RunState::Running => run.started_at(),
                super::RunState::Ended { at, .. } => *at,
            })
            .max()
            .unwrap_or(team.created_at());
        team.record_activities(at);
    }
}

/// Promotes schemas 11 and 12 without rebuilding activity history or inventing base permission or split plans.
pub(super) fn from_v11(text: &str) -> Result<State, StateError> {
    let mut value: Value = serde_json::from_str(text).map_err(|source| StateError::Malformed { source })?;
    if let Some(version) = value.get_mut("schema_version") {
        *version = Value::from(super::SCHEMA_VERSION);
    }
    let wire: StateWire = serde_json::from_value(value).map_err(|source| StateError::Malformed { source })?;
    Ok(State::try_from(wire)?)
}
