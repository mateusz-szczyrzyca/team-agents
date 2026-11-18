//! Output activity remains observable independently of artifact capture limits.

use std::fs::{File, FileTimes};
use std::time::{Duration, SystemTime};

use agent_core::config::AbsolutePath;
use agent_core::diagnostic_assessment::{Anomaly, assess};
use agent_core::ids::{RunId, TaskId};
use agent_core::state::{Crew, Member, MergeMode, Participant, TaskListText, Team, Timestamp};
use agent_core::status::TaskStatus;

use super::collect::Collection;
use crate::store::Home;

#[test]
fn running_output_time_is_collected_even_when_the_output_exceeds_capture_limits() {
    for bytes in [1, 4 * 1024 * 1024 + 1] {
        let dir = tempfile::tempdir().unwrap();
        let home = Home::new(&AbsolutePath::try_from(dir.path().to_str().unwrap().to_owned()).unwrap());
        let task: TaskId = "task-20261117-120000-a1b2".parse().unwrap();
        let run: RunId = "run-20261117-120001-a1b2".parse().unwrap();
        let started: Timestamp = "2026-11-17T12:00:00Z".parse().unwrap();
        let last: Timestamp = "2026-11-17T12:01:00Z".parse().unwrap();
        let now: Timestamp = "2026-11-17T13:00:00Z".parse().unwrap();
        let mut member = Member::new(task.clone(), Participant::Normal("codex-1".parse().unwrap()));
        member.begin_run(run.clone(), started, None).unwrap();
        member.transition(TaskStatus::Running).unwrap();
        let team = Team::new(
            "team-20261117-120000-a1b2".parse().unwrap(),
            "example".parse().unwrap(),
            started,
            MergeMode::Autopilot,
            TaskListText::new("Build.\n\n1. Feature\n[ ] 1.1 Work\n".to_owned()).unwrap(),
            Crew::solo(member).unwrap(),
        );
        let run_dir = home.run_dir(&task, &run);
        std::fs::create_dir_all(&run_dir).unwrap();
        let output = File::create(run_dir.join("out.jsonl")).unwrap();
        output.set_len(bytes).unwrap();
        let modified = SystemTime::from(last.as_utc());
        output.set_times(FileTimes::new().set_modified(modified)).unwrap();
        let mut collection = Collection::new(&team, now, Duration::from_secs(120)).unwrap();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();

        runtime
            .block_on(collection.observe(&home, &team, Some(Ok(String::new()))))
            .unwrap();

        assert_eq!(
            collection.summary.tasks.first().unwrap().last_activity,
            Some(last),
            "output size {bytes}"
        );
        assert_eq!(collection.assessment.tasks.first().unwrap().last_activity, Some(last));
        assert_eq!(
            assess(&collection.assessment)
                .into_iter()
                .filter(|anomaly| matches!(anomaly, Anomaly::StaleTask(_)))
                .collect::<Vec<_>>(),
            vec![Anomaly::StaleTask(task)],
        );
    }
}
