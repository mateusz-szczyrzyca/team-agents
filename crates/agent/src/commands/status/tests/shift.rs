//! `agent status` shows on a lead task with shifts `shift: 1h40m of 2h worked`.

use std::collections::BTreeMap;

use agent_core::config::Config;
use agent_core::ids::TaskId;
use agent_core::local_time::Zone;
use agent_core::protocol::TeamSummary;
use agent_core::state::{State, Timestamp};
use time::{OffsetDateTime, UtcOffset};

use super::shifts;
use super::{Detail, Extras, Notes, Under, View, render_with_notes};
use crate::fixtures::{task, team_state};

/// The configuration with `lead_shift_minutes = 120`.
fn shift_config() -> Config {
    Config::from_toml_str(
        "[limits]\nlead_shift_minutes = 120\n\
         [agents.claude-1]\ncli = \"claude\"\n\
         [investigators.inv-opus]\ncli = \"claude\"\n\
         [investigators.inv-codex]\ncli = \"codex\"\n\
         [tiers]\nnormal = [\"claude-1\"]\nworkers = []\n\
         [binaries]\nclaude = \"claude\"\ncodex = \"codex\"\nagentbus = \"agentbus\"\n",
    )
    .unwrap()
}

/// A team state where the coordinator has worked for 100 minutes (1h40m = 6000s).
fn state_with_worked_lead() -> State {
    fn record(value: &mut serde_json::Value, target: &TaskId) {
        match value {
            serde_json::Value::Object(object) => {
                if object.contains_key("agent")
                    && object.get("task").and_then(serde_json::Value::as_str) == Some(target.as_str())
                {
                    object.insert("lead_worked_secs".into(), 6000.into());
                }
                for child in object.values_mut() {
                    record(child, target);
                }
            }
            serde_json::Value::Array(values) => {
                for child in values {
                    record(child, target);
                }
            }
            _ => {}
        }
    }
    let state = team_state();
    let mut json: serde_json::Value = serde_json::from_str(&state.to_json().unwrap()).unwrap();
    record(&mut json, &task(1));
    State::from_json(&json.to_string()).unwrap()
}

fn summaries(state: &State) -> Vec<TeamSummary> {
    state.teams().iter().map(TeamSummary::from).collect()
}

#[test]
fn status_shows_shift_progress_on_lead_task() {
    let config = shift_config();
    let state = state_with_worked_lead();
    let teams = summaries(&state);
    let now = Timestamp::from_utc(OffsetDateTime::from_unix_timestamp(1_700_000_000).unwrap().into()).unwrap();

    let shifts_map = shifts(&config, &state, now, &teams);
    assert_eq!(
        shifts_map.get(&task(1)).map(String::as_str),
        Some("shift: 1h40m of 2h worked")
    );

    let notes = Notes {
        extras: Extras {
            under: Under {
                requests: &|_| Vec::new(),
                workers: &|_| Vec::new(),
            },
            accounts: &BTreeMap::new(),
        },
        waits: &|_| None,
        shifts: &|task| shifts_map.get(task).cloned(),
    };
    let rendered = render_with_notes(
        &teams,
        View::Full,
        Detail::Tasks,
        &|_| None,
        &|_| String::new(),
        notes,
        Zone::Fixed(UtcOffset::UTC),
    );

    assert!(
        rendered.contains("task-20261111-153001-k3x9  coordinator  inv-opus   queued, shift: 1h40m of 2h worked"),
        "rendered output was:\n{rendered}"
    );
}
