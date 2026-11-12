# Recorded CLI output

Output of the four model CLIs, one directory per CLI, as the adapters of `crates/agent-runtime/src/cli/` read it.
Each file is the stdout of a run (one JSON object a line).

## Provenance

Recorded: lines of real runs of the old harness (its `out-N.jsonl` and `events-N.jsonl`). A file that says "lines of"
holds chosen lines of one longer run, in their order; `opencode/mcp-tools.jsonl` holds lines of three runs.

| File | What it is |
|---|---|
| `claude/done.jsonl` | a whole run: init, tool calls and results, the final text, the result |
| `claude/cut.jsonl` | the first six lines of `claude/done.jsonl`: a run that was killed |
| `codex/done.jsonl` | a whole run: thread, agentbus calls, a command, the final message, `turn.completed` |
| `codex/cut.jsonl` | the first seven lines of a run that was stopped |
| `codex/at-capacity.jsonl` | a whole run the provider refused: `error` and `turn.failed` |
| `codex/edit.jsonl` | lines of a run: a file change and a command, started and completed |
| `agy/done.jsonl` | a whole run: init, agentbus calls through `call_mcp_tool`, a response in pieces, the result |
| `agy/error.jsonl` | lines of a run that ended `ERROR`: init, a system message, the result |
| `opencode/done.jsonl` | a whole run: a step, its text and its `stop` |
| `opencode/tools.jsonl` | a run that printed tool calls and a final text but no `stop` |
| `opencode/mcp-tools.jsonl` | an `execute` call, an `edit`, and a call that failed |
| `opencode/rate-limit.jsonl` | a run the provider refused with HTTP 429: steps, then an `error` |

Written by hand after the CLI's documented format, because no recording of the case exists (the old harness's runs
ended in none of them): `claude/max-turns.jsonl`, `claude/api-error.jsonl`, `claude/errors-only.jsonl` (an error result with its messages in `errors` and no `result`) and `codex/failed.jsonl`.

## What was changed in a recording

The home directory of the machine is `/home/example` and the owner of the repository is `example-owner`. Fields no
adapter reads and that are large were dropped (Claude's tool, slash-command and plugin lists, per-model usage tables
and signatures; Antigravity's list of tools; opencode's snapshots and metadata). A text longer than 240 characters
(200 in `opencode/`) is cut and ends with `…`. Nothing else was edited.

No run was made for these files: a real run costs the human money and quota (`AGENTS.md`, rule 4).
