//! The system clock reads the machine's time.

#[cfg(test)]
mod clock {
    use agent_core::ids::Clock;
    use agent_runtime::clock::{SharedClock, SystemClock};
    use time::UtcDateTime;

    #[test]
    fn the_system_clock_is_between_two_readings_of_the_machines_time() {
        // The clock drops the fraction of a second, so it may read up to a second before the machine's time.
        let before = UtcDateTime::now() - time::Duration::seconds(1);

        let read = SystemClock.now();

        assert!(before <= read && read <= UtcDateTime::now(), "{before} {read}");
    }

    #[test]
    fn the_system_clock_counts_whole_seconds() {
        for _ in 0..50 {
            assert_eq!(SystemClock.now().nanosecond(), 0);
        }
    }

    #[test]
    fn a_shared_clock_gives_the_time_of_the_clock_it_shares() {
        struct Fixed;
        impl Clock for Fixed {
            fn now(&self) -> UtcDateTime {
                UtcDateTime::UNIX_EPOCH + time::Duration::days(1)
            }
        }
        let shared = SharedClock::new(Fixed);

        assert_eq!(shared.clone().now(), Fixed.now());
    }
}
