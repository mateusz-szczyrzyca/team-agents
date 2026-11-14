use rand::SeedableRng;
use rand::rngs::Xoshiro256PlusPlus;

use super::*;

#[test]
fn a_new_session_id_is_a_version_4_uuid() {
    let mut rng = Xoshiro256PlusPlus::seed_from_u64(7);

    for _ in 0..200 {
        let id = new_session_id(&mut rng).unwrap();

        let text = id.as_str();
        assert_eq!(text.len(), 36, "{text}");
        let groups: Vec<&str> = text.split('-').collect();
        assert_eq!(
            groups.iter().map(|group| group.len()).collect::<Vec<_>>(),
            [8, 4, 4, 4, 12],
            "{text}"
        );
        assert!(
            text.chars()
                .all(|c| c == '-' || c.is_ascii_hexdigit() && !c.is_ascii_uppercase()),
            "{text}"
        );
        assert_eq!(groups[2].chars().next(), Some('4'), "the version: {text}");
        assert!(
            matches!(groups[3].chars().next(), Some('8' | '9' | 'a' | 'b')),
            "the variant: {text}"
        );
    }
}

#[test]
fn two_session_ids_differ() {
    let mut rng = Xoshiro256PlusPlus::seed_from_u64(1);

    let first = new_session_id(&mut rng).unwrap();
    let second = new_session_id(&mut rng).unwrap();

    assert_ne!(first, second);
}

#[derive(Clone, Default)]
struct LogBuffer(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);

impl std::io::Write for LogBuffer {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn capture_warnings(run: impl FnOnce()) -> String {
    let buffer = LogBuffer::default();
    let writer = buffer.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_writer(move || writer.clone())
        .with_max_level(tracing::Level::WARN)
        .without_time()
        .with_ansi(false)
        .finish();
    tracing::subscriber::with_default(subscriber, run);
    String::from_utf8(buffer.0.lock().unwrap().clone()).unwrap()
}

#[test]
fn inconsistent_state_falls_back_to_continuation_and_logs_a_warning() {
    use crate::fixtures::{task, team_state};

    let state = team_state();
    let missing_log = capture_warnings(|| {
        assert_eq!(run_why(&state, &task(9), None), RunWhy::Continuation);
    });
    assert!(
        missing_log.contains("the task of a new run is not in the state"),
        "{missing_log}"
    );

    let fix_log = capture_warnings(|| {
        assert_eq!(run_why(&state, &task(1), Some(&Why::FixPass)), RunWhy::Continuation);
    });
    assert!(fix_log.contains("no item of the team is in a fix pass"), "{fix_log}");

    let review_log = capture_warnings(|| {
        assert_eq!(run_why(&state, &task(4), Some(&Why::Review)), RunWhy::Continuation);
    });
    assert!(
        review_log.contains("the reviewer has no pending review in the state"),
        "{review_log}"
    );
}
