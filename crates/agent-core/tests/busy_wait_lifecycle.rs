//! Ending a queued busy wait abandons its age; resuming starts a fresh interval.

#[cfg(test)]
mod tests {
    use agent_core::state::{Member, Participant, Timestamp};
    use agent_core::status::TaskStatus;

    #[test]
    fn pausing_a_queued_task_then_resuming_starts_a_fresh_busy_wait() {
        let mut member = Member::new(
            "task-20261121-100000-wait".parse().unwrap(),
            Participant::Normal("claude-1".parse().unwrap()),
        );
        let first: Timestamp = "2026-11-21T10:00:00Z".parse().unwrap();
        let resumed: Timestamp = "2026-11-21T11:00:00Z".parse().unwrap();
        assert_eq!(member.start_waiting(first), first);

        member.transition(TaskStatus::Paused).unwrap();
        assert_eq!(member.waiting_since(), None);
        member.transition(TaskStatus::Queued).unwrap();
        assert_eq!(member.waiting_since(), None);

        assert_eq!(member.start_waiting(resumed), resumed);
        assert_eq!(member.waiting_since(), Some(resumed));
        let later: Timestamp = "2026-11-21T11:01:00Z".parse().unwrap();
        assert_eq!(member.start_waiting(later), resumed);
    }

    #[test]
    fn stopping_a_queued_task_then_resuming_starts_a_fresh_busy_wait() {
        let mut member = Member::new(
            "task-20261121-100000-wait".parse().unwrap(),
            Participant::Normal("claude-1".parse().unwrap()),
        );
        let first: Timestamp = "2026-11-21T10:00:00Z".parse().unwrap();
        let resumed: Timestamp = "2026-11-21T11:00:00Z".parse().unwrap();
        assert_eq!(member.start_waiting(first), first);

        member.transition(TaskStatus::Stopped).unwrap();
        assert_eq!(member.waiting_since(), None);
        member.transition(TaskStatus::Queued).unwrap();
        assert_eq!(member.waiting_since(), None);

        assert_eq!(member.start_waiting(resumed), resumed);
        assert_eq!(member.waiting_since(), Some(resumed));
        let later: Timestamp = "2026-11-21T11:01:00Z".parse().unwrap();
        assert_eq!(member.start_waiting(later), resumed);
    }
}
