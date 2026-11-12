//! The system clock reads the machine's time.

#[cfg(test)]
mod clock {
    use agent_core::ids::Clock;
    use agent_runtime::clock::{SharedClock, SystemClock};
    use time::UtcDateTime;

    #[test]
    fn the_system_clock_is_between_two_readings_of_the_machines_time() {
        let before = UtcDateTime::now();

        let read = SystemClock.now();

        assert!(before <= read && read <= UtcDateTime::now(), "{before} {read}");
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
