use std::time::Duration;

use tokio::io::{AsyncWriteExt, BufReader};

use super::*;

async fn read(bytes: &[u8], max: usize) -> Result<Option<String>, LineError> {
    read_line(&mut BufReader::new(bytes), max).await
}

#[tokio::test]
async fn a_line_is_read_without_its_newline() {
    assert_eq!(read(b"{\"a\":1}\n", 100).await.unwrap().as_deref(), Some("{\"a\":1}"));
}

#[tokio::test]
async fn lines_come_one_after_the_other_and_then_none() {
    let mut reader = BufReader::new(&b"one\ntwo\n\nthree\n"[..]);

    let mut lines = Vec::new();
    while let Some(line) = read_line(&mut reader, 100).await.unwrap() {
        lines.push(line);
    }

    assert_eq!(lines, ["one", "two", "", "three"]);
}

#[tokio::test]
async fn a_stream_that_ends_before_any_byte_has_no_line() {
    assert_eq!(read(b"", 100).await.unwrap(), None);
}

#[tokio::test]
async fn a_stream_that_ends_inside_a_line_is_truncated() {
    let error = read(b"half a li", 100).await.unwrap_err();

    assert!(matches!(error, LineError::Truncated), "{error:?}");
}

#[tokio::test]
async fn a_line_of_exactly_the_limit_is_read_and_one_byte_more_is_too_long() {
    assert_eq!(read(b"12345\n", 5).await.unwrap().as_deref(), Some("12345"));

    let error = read(b"123456\n", 5).await.unwrap_err();

    assert!(matches!(error, LineError::TooLong { max: 5 }), "{error:?}");
}

#[tokio::test]
async fn a_line_over_the_limit_is_refused_before_its_end_is_read() {
    // No newline at all: the reader must not wait for one.
    let long = vec![b'x'; 10_000];

    let error = read(&long, 100).await.unwrap_err();

    assert!(matches!(error, LineError::TooLong { max: 100 }), "{error:?}");
}

#[tokio::test]
async fn a_line_that_is_not_text_is_refused() {
    let error = read(b"\xff\xfe\n", 100).await.unwrap_err();

    assert!(matches!(error, LineError::NotUtf8(_)), "{error:?}");
}

#[tokio::test]
async fn text_that_is_not_ascii_is_read_whole_across_a_small_buffer() {
    let text = "żółć 日本語 🦀";
    let mut bytes = text.as_bytes().to_vec();
    bytes.push(b'\n');
    let mut reader = BufReader::with_capacity(3, &bytes[..]);

    let line = read_line(&mut reader, 100).await.unwrap();

    assert_eq!(line.as_deref(), Some(text));
}

#[tokio::test]
async fn a_written_line_reaches_the_stream_whole() {
    let mut out = Vec::new();

    write_line(&mut out, "{\"a\":1}\n").await.unwrap();

    assert_eq!(out, b"{\"a\":1}\n");
}

#[tokio::test(start_paused = true)]
async fn a_line_read_in_parts_survives_a_read_that_is_dropped_before_its_end() {
    let (mut client, server) = tokio::io::duplex(64);
    let mut reader = BufReader::new(server);
    let mut pending = Vec::new();
    client.write_all(b"{\"half\":").await.unwrap();

    // The read takes the first part and waits for the rest; the timeout drops it, as a `select!` does.
    let dropped = tokio::time::timeout(
        Duration::from_millis(50),
        read_line_into(&mut reader, &mut pending, 100),
    )
    .await;
    client.write_all(b"1}\n").await.unwrap();
    let line = read_line_into(&mut reader, &mut pending, 100).await.unwrap();

    assert!(dropped.is_err(), "the first read waits for the end of the line");
    assert_eq!(line.as_deref(), Some("{\"half\":1}"));
    assert_eq!(pending, Vec::<u8>::new(), "nothing of the line is left over");
}
