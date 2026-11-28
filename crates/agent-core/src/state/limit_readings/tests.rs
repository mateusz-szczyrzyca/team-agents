use serde_json::json;

use super::*;
use crate::state::State;

fn at(text: &str) -> Timestamp {
    text.parse().expect("a valid timestamp")
}

fn account(name: &str) -> ReadingOwner {
    ReadingOwner::Account(name.parse().expect("an account name"))
}

fn agent(name: &str) -> ReadingOwner {
    ReadingOwner::Agent(name.parse().expect("an agent name"))
}

fn five_hour(used: f64, resets_at: &str) -> LimitReading {
    LimitReading::new(LimitWindow::FiveHour, used, at(resets_at)).expect("a reading")
}

fn seven_day(used: f64, resets_at: &str) -> LimitReading {
    LimitReading::new(LimitWindow::SevenDay, used, at(resets_at)).expect("a reading")
}

/// `reading` read at `read_at`.
fn read(reading: LimitReading, read_at: &str) -> KeptReading {
    KeptReading::new(reading, at(read_at))
}

const NOW: &str = "2026-11-27T14:00:00Z";
const LATER: &str = "2026-11-27T15:30:00Z";

// Rule: of two readings of a window the newer is kept; an older one is ignored; one whose window has reset is dropped.
#[test]
fn keep_reading_table() {
    let old = read(five_hour(0.2, LATER), "2026-11-27T13:00:00Z");
    let new = read(five_hour(0.6, LATER), "2026-11-27T13:30:00Z");
    let same_moment = read(five_hour(0.7, LATER), "2026-11-27T13:30:00Z");
    let over = read(five_hour(0.9, "2026-11-27T13:45:00Z"), "2026-11-27T13:40:00Z");
    let resets_now = read(five_hour(0.9, NOW), "2026-11-27T13:40:00Z");
    let now = at(NOW);

    let table: [(&str, Option<KeptReading>, KeptReading, Option<KeptReading>); 8] = [
        ("nothing kept: the new one is kept", None, new, Some(new)),
        ("a newer reading replaces an older", Some(old), new, Some(new)),
        ("an older reading is ignored", Some(new), old, Some(new)),
        (
            "a reading read at the same moment replaces",
            Some(new),
            same_moment,
            Some(same_moment),
        ),
        ("a new reading past its reset is ignored", Some(old), over, Some(old)),
        ("a new reading past its reset keeps nothing", None, over, None),
        ("a kept reading past its reset is dropped", Some(over), new, Some(new)),
        (
            "a window that resets at this moment is over",
            Some(resets_now),
            resets_now,
            None,
        ),
    ];
    for (name, kept, incoming, expected) in table {
        assert_eq!(keep_reading(kept, incoming, now), expected, "{name}");
    }
}

// Rule: the readings of a run are kept per window with the time they were read.
#[test]
fn a_run_s_readings_are_kept_per_window_with_the_time_they_were_read() {
    let mut readings = LimitReadings::default();
    let both = [five_hour(0.62, LATER), seven_day(0.41, "2026-12-01T09:00:00Z")];

    readings.keep(&account("comp-claude"), &both, at(NOW), at(NOW));

    let live = readings.live(at(NOW));
    assert_eq!(live.len(), 1);
    assert_eq!(live[0].0, &account("comp-claude"));
    assert_eq!(
        live[0].1,
        vec![KeptReading::new(both[0], at(NOW)), KeptReading::new(both[1], at(NOW))]
    );
}

// Rule: a later run replaces the reading of its window and leaves the other window alone.
#[test]
fn a_later_reading_replaces_its_window_and_keeps_the_other() {
    let mut readings = LimitReadings::default();
    let owner = account("comp-claude");
    readings.keep(
        &owner,
        &[five_hour(0.2, LATER), seven_day(0.4, "2026-12-01T09:00:00Z")],
        at("2026-11-27T13:00:00Z"),
        at("2026-11-27T13:00:00Z"),
    );

    readings.keep(&owner, &[five_hour(0.8, LATER)], at(NOW), at(NOW));

    let live = readings.live(at(NOW));
    let used: Vec<f64> = live[0].1.iter().map(|kept| kept.reading().used()).collect();
    assert_eq!(used, vec![0.8, 0.4]);
}

// Rule: an agent without an account has its own readings; an account and an agent of one name are two owners.
#[test]
fn an_agent_without_an_account_keeps_its_own_readings() {
    let mut readings = LimitReadings::default();
    readings.keep(&agent("comp"), &[five_hour(0.1, LATER)], at(NOW), at(NOW));
    readings.keep(&account("comp"), &[five_hour(0.9, LATER)], at(NOW), at(NOW));

    let live = readings.live(at(NOW));

    let owners: Vec<&ReadingOwner> = live.iter().map(|(owner, _)| *owner).collect();
    assert_eq!(owners, vec![&account("comp"), &agent("comp")]);
    assert_eq!(live[0].1[0].reading().used(), 0.9);
    assert_eq!(live[1].1[0].reading().used(), 0.1);
}

// Rule: a reading past its reset is dropped, also from the others when a reading of an owner is kept.
#[test]
fn readings_past_their_reset_are_dropped() {
    let mut readings = LimitReadings::default();
    readings.keep(&account("a-claude"), &[five_hour(0.5, LATER)], at(NOW), at(NOW));
    readings.keep(
        &agent("solo"),
        &[five_hour(0.5, "2026-11-27T20:00:00Z")],
        at(NOW),
        at(NOW),
    );

    assert_eq!(
        readings.live(at("2026-11-27T16:00:00Z")).len(),
        1,
        "hidden after the reset"
    );
    readings.keep(
        &agent("solo"),
        &[],
        at("2026-11-27T16:00:00Z"),
        at("2026-11-27T16:00:00Z"),
    );

    let json = serde_json::to_value(&readings).expect("serialises");
    assert_eq!(json.get("accounts"), None, "{json}");
    assert!(json["agents"]["solo"].is_array(), "{json}");
}

// Rule: the readings survive a restart: the state writes them and reads them back.
#[test]
fn restart_keeps_reading() {
    let mut state = State::new();
    let owner = account("comp-claude");
    state.keep_limit_readings(
        &owner,
        &[five_hour(0.62, LATER), seven_day(0.41, "2026-12-01T09:00:00Z")],
        at(NOW),
        at(NOW),
    );
    state.keep_limit_readings(&agent("solo"), &[five_hour(0.25, LATER)], at(NOW), at(NOW));

    let json = state.to_json().expect("a state is written");
    let loaded = State::from_json(&json).expect("the state is read back");

    assert_eq!(loaded, state);
    assert_eq!(
        loaded.limit_readings().live(at(NOW)),
        state.limit_readings().live(at(NOW))
    );
    assert_eq!(loaded.limit_readings().live(at(NOW)).len(), 2);
}

// Rule: a state written before the readings loads, and writes none.
#[test]
fn old_state_without_readings_loads() {
    let state = State::from_json(include_str!("../../../tests/fixtures/state-before-readings.json"))
        .expect("a state of main loads");

    assert!(state.limit_readings().is_empty());
    let json = state.to_json().expect("a state is written");
    assert!(!json.contains("limit_readings"), "{json}");
    assert_eq!(State::from_json(&json).expect("it loads again"), state);
}

// Rule: the readings are written as lists per owner, with the window named by its key in the CLI's records.
#[test]
fn readings_are_written_as_a_list_per_owner() {
    let mut readings = LimitReadings::default();
    readings.keep(&account("comp-claude"), &[five_hour(0.5, LATER)], at(NOW), at(NOW));

    let json = serde_json::to_value(&readings).expect("serialises");

    assert_eq!(
        json,
        json!({"accounts": {"comp-claude": [{
            "reading": {"window": "five_hour", "used": 0.5, "resets_at": "2026-11-27T15:30:00Z"},
            "read_at": "2026-11-27T14:00:00Z",
        }]}})
    );
}

// Rule: a stored reading is checked like one made now: a share outside 0..1 and an unknown window are refused.
#[test]
fn a_stored_reading_with_a_share_out_of_range_or_an_unknown_window_is_refused() {
    let stored = |window: &str, used: f64| {
        json!({"accounts": {"comp-claude": [{
            "reading": {"window": window, "used": used, "resets_at": "2026-11-27T15:30:00Z"},
            "read_at": "2026-11-27T14:00:00Z",
        }]}})
    };

    serde_json::from_value::<LimitReadings>(stored("five_hour", 0.5)).expect("a valid reading loads");
    for (window, used) in [("five_hour", 1.5), ("five_hour", -0.1), ("one_hour", 0.5)] {
        let error = serde_json::from_value::<LimitReadings>(stored(window, used)).expect_err("refused");
        assert!(error.is_data(), "{window} {used}: {error}");
    }
}

// Rule: two stored readings of one window leave the newer one.
#[test]
fn two_stored_readings_of_one_window_leave_the_newer() {
    let entry = |used: f64, read_at: &str| {
        json!({
            "reading": {"window": "five_hour", "used": used, "resets_at": "2026-11-27T15:30:00Z"},
            "read_at": read_at,
        })
    };
    let stored = json!({"agents": {"solo": [entry(0.9, "2026-11-27T13:30:00Z"), entry(0.3, "2026-11-27T13:00:00Z")]}});

    let readings: LimitReadings = serde_json::from_value(stored).expect("loads");

    let live = readings.live(at(NOW));
    assert_eq!(live[0].1.len(), 1);
    assert_eq!(live[0].1[0].reading().used(), 0.9);
}

// Rule: a stored entry with a key beside `reading` and `read_at` is refused, like every other unknown key of the list.
#[test]
fn a_stored_reading_with_an_unknown_key_is_refused() {
    let stored = json!({"accounts": {"comp-claude": [{
        "reading": {"window": "five_hour", "used": 0.5, "resets_at": "2026-11-27T15:30:00Z"},
        "read_at": "2026-11-27T14:00:00Z",
        "extra": 1,
    }]}});

    let error = serde_json::from_value::<LimitReadings>(stored).expect_err("an unknown key is refused");

    assert!(error.is_data(), "{error}");
}
