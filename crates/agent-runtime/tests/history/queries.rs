//! Filter intersections, boundaries and ordering over records written through the history API.

use std::collections::BTreeMap;

use agent_core::history::Cut;
use agent_core::log::{LogEvent, LogLevel, LogRecord, LogTaskScope, LogTeamScope};
use agent_runtime::history::{LogFilter, LogWindow, Reader, Store};

use super::support::{at, complete, database};

#[test]
fn every_filter_and_pair_matches_the_unfiltered_rows_including_fractional_boundaries() {
    for seed in [87, 8701, 8702] {
        let (_dir, path, mut db) = database();
        let world = complete(seed);
        db.write(&world.records()).unwrap();
        let run = &world.runs[0].start;
        for i in 0..8 {
            db.write_logs(&[LogRecord {
                at: at(10, i),
                module: "query-test".to_owned(),
                event: if i % 2 == 0 {
                    LogEvent::RunStarted
                } else {
                    LogEvent::DecisionTaken
                },
                team: (i % 3 != 0).then(|| LogTeamScope::new(run.team.clone(), run.rounds.clone())),
                task: (i % 3 != 0).then(|| LogTaskScope::new(run.task.clone(), Some(run.run.clone()))),
                message: format!("row {i}"),
                fields: BTreeMap::new(),
                cut: Cut::default(),
            }])
            .unwrap();
        }
        let reader = Reader::open(&path).unwrap();
        let all = reader.daemon_logs().unwrap();
        assert_eq!(
            reader.log_rows(&LogFilter::default(), LogWindow::Newest(100)).unwrap(),
            all
        );
        for a in 0..9 {
            for b in a..9 {
                let mut filter = LogFilter::default();
                for index in [a, b] {
                    match index {
                        0 => filter.team = Some(run.team.clone()),
                        1 => filter.round = Some(run.rounds[0].clone()),
                        2 => filter.task = Some(run.task.clone()),
                        3 => filter.run = Some(run.run.clone()),
                        4 => filter.agent = Some(run.agent.name.parse().unwrap()),
                        5 => filter.level = Some(LogLevel::Info),
                        6 => filter.event = Some(LogEvent::RunStarted),
                        7 => filter.since = Some(at(10, 2)),
                        8 => filter.until = Some(at(10, 6)),
                        _ => panic!("unknown filter index {index}"),
                    }
                }
                let expected: Vec<_> = all
                    .iter()
                    .filter(|row| {
                        filter.team.as_ref().is_none_or(|v| row.team() == Some(v))
                            && filter.round.as_ref().is_none_or(|v| row.rounds().contains(v))
                            && filter.task.as_ref().is_none_or(|v| row.task() == Some(v))
                            && filter.run.as_ref().is_none_or(|v| row.run() == Some(v))
                            && (filter.agent.is_none() || row.run() == Some(&run.run))
                            && filter.level.is_none_or(|v| row.level() == v)
                            && filter.event.is_none_or(|v| row.event() == v)
                            && filter.since.is_none_or(|v| row.at() >= v)
                            && filter.until.is_none_or(|v| row.at() <= v)
                    })
                    .cloned()
                    .collect();
                assert_eq!(
                    reader.log_rows(&filter, LogWindow::Newest(100)).unwrap(),
                    expected,
                    "seed={seed}, {a}/{b}"
                );
            }
        }
        assert_eq!(
            reader.log_rows(&LogFilter::default(), LogWindow::Newest(2)).unwrap(),
            all[6..]
        );
        assert_eq!(
            reader
                .log_rows(&LogFilter::default(), LogWindow::After { id: 2, limit: 2 })
                .unwrap(),
            all[2..4]
        );
        assert_eq!(
            reader.log_rows(&LogFilter::default(), LogWindow::Newest(0)).unwrap(),
            vec![]
        );
    }
}

#[test]
fn timestamp_filters_read_version_four_without_requiring_new_indexes() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("history.db");
    let mut db = agent_runtime::history::HistoryDb::open_with(
        &path,
        &agent_runtime::history::schema::MIGRATIONS[..4],
        agent_core::history::MaxTextBytes::default(),
    )
    .unwrap();
    agent_runtime::history::Store::write_logs(
        &mut db,
        &[agent_core::log::LogRecord {
            at: "2026-11-15T12:00:00Z".parse().unwrap(),
            module: "legacy".to_owned(),
            event: agent_core::log::LogEvent::DaemonStarted,
            team: None,
            task: None,
            message: "old".to_owned(),
            fields: std::collections::BTreeMap::new(),
            cut: agent_core::history::Cut::default(),
        }],
    )
    .unwrap();
    let reader = agent_runtime::history::Reader::open(&path).unwrap();
    let before = reader.schema().unwrap();
    assert_eq!(
        reader
            .log_rows(
                &agent_runtime::history::LogFilter {
                    since: Some("2026-11-15T12:00:00Z".parse().unwrap()),
                    ..Default::default()
                },
                agent_runtime::history::LogWindow::Newest(100)
            )
            .unwrap(),
        reader.daemon_logs().unwrap()
    );
    assert_eq!(reader.schema().unwrap(), before);
}
