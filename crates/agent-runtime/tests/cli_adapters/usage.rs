//! What each adapter reads of a run's context size, tokens written and cost from its stream. Where the CLI tells
//! nothing of one of them the part is "not told" (`None`), never a guessed number.

use agent_core::cli::{CliAdapter, RunUsage};
use agent_runtime::cli::agy::AgyAdapter;
use agent_runtime::cli::claude::ClaudeAdapter;
use agent_runtime::cli::codex::CodexAdapter;
use agent_runtime::cli::opencode::OpencodeAdapter;

use crate::support::{TestResult, fixture};

/// The usage the parser of `adapter` reports after it has read every line of `stdout`.
fn usage_of(adapter: &dyn CliAdapter, stdout: &str) -> RunUsage {
    let mut parser = adapter.parser();
    for line in stdout.lines() {
        let _ignored_steps = parser.push_line(line);
    }
    parser.usage()
}

#[test]
fn opencode_reads_the_tokens_and_the_cost_of_its_step() -> TestResult {
    let usage = usage_of(&OpencodeAdapter, &fixture("opencode", "done.jsonl")?);

    assert_eq!(
        usage,
        RunUsage {
            first_context: Some(16_733),
            last_context: Some(16_733),
            tokens_written: Some(997),
            cost_usd: Some(0.000_788_611),
        }
    );
    Ok(())
}

#[test]
fn claude_reads_each_model_call_once_and_never_the_total_of_the_result() {
    // The stream repeats a call's usage on every content block of its message; the result's usage is the total.
    let stdout = [
        r#"{"type":"assistant","message":{"id":"msg_1","role":"assistant","content":[{"type":"thinking","thinking":""}],"usage":{"input_tokens":3,"cache_creation_input_tokens":50,"cache_read_input_tokens":100,"output_tokens":20}},"session_id":"s-1"}"#,
        r#"{"type":"assistant","message":{"id":"msg_1","role":"assistant","content":[{"type":"text","text":"Reading."}],"usage":{"input_tokens":3,"cache_creation_input_tokens":50,"cache_read_input_tokens":100,"output_tokens":20}},"session_id":"s-1"}"#,
        r#"{"type":"assistant","message":{"id":"msg_2","role":"assistant","content":[{"type":"text","text":"Done."}],"usage":{"input_tokens":1,"cache_creation_input_tokens":0,"cache_read_input_tokens":200,"output_tokens":30}},"session_id":"s-1"}"#,
        r#"{"type":"result","subtype":"success","is_error":false,"result":"STATUS: DONE","session_id":"s-1","total_cost_usd":0.5,"usage":{"input_tokens":4,"output_tokens":50}}"#,
    ]
    .join("\n");

    let usage = usage_of(&ClaudeAdapter, &stdout);

    assert_eq!(
        usage,
        RunUsage {
            first_context: Some(153),
            last_context: Some(201),
            tokens_written: Some(50),
            cost_usd: Some(0.5),
        }
    );
}

#[test]
fn claude_tells_no_size_for_a_stream_without_usage() -> TestResult {
    let usage = usage_of(&ClaudeAdapter, &fixture("claude", "cut.jsonl")?);

    assert_eq!(usage, RunUsage::default());
    Ok(())
}

#[test]
fn codex_tells_its_output_tokens_and_no_context_and_no_cost() -> TestResult {
    let usage = usage_of(&CodexAdapter, &fixture("codex", "done.jsonl")?);

    assert_eq!(
        usage,
        RunUsage {
            first_context: None,
            last_context: None,
            tokens_written: Some(500),
            cost_usd: None,
        }
    );
    Ok(())
}

#[test]
fn codex_tells_nothing_for_a_cut_run() -> TestResult {
    let usage = usage_of(&CodexAdapter, &fixture("codex", "cut.jsonl")?);

    assert_eq!(usage, RunUsage::default());
    Ok(())
}

#[test]
fn agy_sums_the_output_of_its_steps_and_takes_the_context_from_input_and_cache() -> TestResult {
    let usage = usage_of(&AgyAdapter, &fixture("agy", "done.jsonl")?);

    assert_eq!(
        usage,
        RunUsage {
            first_context: Some(29_299),
            last_context: Some(32_308),
            tokens_written: Some(2_060),
            cost_usd: None,
        }
    );
    Ok(())
}

#[test]
fn claude_reads_the_output_total_of_its_result() -> TestResult {
    let usage = usage_of(&ClaudeAdapter, &fixture("claude", "done.jsonl")?);

    assert_eq!(usage.tokens_written, Some(2_053));
    Ok(())
}

#[test]
fn claude_tells_no_last_context_when_the_last_call_tells_no_usage() {
    let stdout = [
        r#"{"type":"assistant","message":{"id":"msg_1","role":"assistant","content":[{"type":"text","text":"Reading."}],"usage":{"input_tokens":3,"cache_creation_input_tokens":50,"cache_read_input_tokens":100,"output_tokens":20}},"session_id":"s-1"}"#,
        r#"{"type":"assistant","message":{"id":"msg_2","role":"assistant","content":[{"type":"text","text":"Done."}]},"session_id":"s-1"}"#,
    ]
    .join("\n");

    let usage = usage_of(&ClaudeAdapter, &stdout);

    assert_eq!(
        usage,
        RunUsage {
            first_context: Some(153),
            last_context: None,
            tokens_written: None,
            cost_usd: None,
        }
    );
}

#[test]
fn claude_takes_the_output_total_of_its_result_also_when_a_call_tells_no_usage() {
    let stdout = [
        r#"{"type":"assistant","message":{"id":"msg_1","role":"assistant","content":[{"type":"text","text":"Reading."}]},"session_id":"s-1"}"#,
        r#"{"type":"result","subtype":"success","is_error":false,"result":"STATUS: DONE","session_id":"s-1","usage":{"input_tokens":4,"output_tokens":77}}"#,
    ]
    .join("\n");

    let usage = usage_of(&ClaudeAdapter, &stdout);

    assert_eq!(
        usage,
        RunUsage {
            first_context: None,
            last_context: None,
            tokens_written: Some(77),
            cost_usd: None,
        }
    );
}

#[test]
fn agy_reads_the_output_total_of_its_result_when_no_step_tells_usage() -> TestResult {
    let usage = usage_of(&AgyAdapter, &fixture("agy", "error.jsonl")?);

    assert_eq!(
        usage,
        RunUsage {
            first_context: None,
            last_context: None,
            tokens_written: Some(32_984),
            cost_usd: None,
        }
    );
    Ok(())
}
