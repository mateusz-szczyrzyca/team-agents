# Item 5.1 — Claude Code, Codex, Antigravity and opencode adapters

Investigator `inv-claude-sonnet`, branch `agents/inv-claude-sonnet/20261112-154231`, base
[`ccbae53`](https://github.com/mateusz-szczyrzyca/team-agents/commit/ccbae53f5a4b8efbaa395aac8037ba2937d018bc).
Status: done; the code is pushed, no pull request opened (the merger integrates).

## What and why

One `CliAdapter` per model CLI, so that every model runs through the contract of item 2.3 and a CLI's quirks stay in its
adapter. Pure code: the adapters build a `RunSpec` and parse stdout line by line; the runtime (6.1) starts the process.

Files, all under `crates/agent-runtime/`:

- [`src/cli.rs`](https://github.com/mateusz-szczyrzyca/team-agents/blob/5ef638633c0d921c17b6229e8f25d2d4da9d6e6e/crates/agent-runtime/src/cli.rs) —
  `adapter_for(CliKind)`, and what the four share: MCP server texts (Serena's `--context`), path and JSON helpers,
  tool-call classification into `ActivityEvent`s, the `Progress` of a parser, the line reader.
- [`src/cli/claude.rs`](https://github.com/mateusz-szczyrzyca/team-agents/blob/5ef638633c0d921c17b6229e8f25d2d4da9d6e6e/crates/agent-runtime/src/cli/claude.rs),
  [`codex.rs`](https://github.com/mateusz-szczyrzyca/team-agents/blob/5ef638633c0d921c17b6229e8f25d2d4da9d6e6e/crates/agent-runtime/src/cli/codex.rs),
  [`agy.rs`](https://github.com/mateusz-szczyrzyca/team-agents/blob/5ef638633c0d921c17b6229e8f25d2d4da9d6e6e/crates/agent-runtime/src/cli/agy.rs),
  [`opencode.rs`](https://github.com/mateusz-szczyrzyca/team-agents/blob/5ef638633c0d921c17b6229e8f25d2d4da9d6e6e/crates/agent-runtime/src/cli/opencode.rs) —
  the four adapters.
- `src/lib.rs` — `pub mod cli;`.
- `tests/cli_adapters/` (one test crate: `claude.rs`, `codex.rs`, `agy.rs`, `opencode.rs`, `support.rs`) and
  [`tests/fixtures/cli/`](https://github.com/mateusz-szczyrzyca/team-agents/tree/5ef638633c0d921c17b6229e8f25d2d4da9d6e6e/crates/agent-runtime/tests/fixtures/cli)
  with a `README.md` of the provenance of each file.
- `docs/architecture.md` (an "adapters" section) and `docs/configuration.md` (decision 7, see below).

Files outside the item's list, and why: `src/cli.rs` (the module root; the shared code is not duplicated four times),
`src/lib.rs` (declares it), the docs (the plan's documentation rule), `tests/cli_adapters/support.rs` and
`tests/fixtures/cli/README.md` (test support and provenance).

## Behaviour, per CLI

| | Claude Code | Codex | Antigravity (`agy`) | opencode |
|---|---|---|---|---|
| New run | `-p --output-format stream-json --verbose --dangerously-skip-permissions --mcp-config F --max-turns 300 [--strict-mcp-config] --settings F [--model M] --session-id ID EXTRA… PROMPT` | `exec --json --dangerously-bypass-approvals-and-sandbox -C DIR (-c mcp_servers.…)… [-m M] EXTRA… PROMPT` | `--output-format stream-json --dangerously-skip-permissions --print-timeout 8h [--model M] EXTRA… -p PROMPT` | `run --standalone --format json --auto [-m M] EXTRA… -- PROMPT` |
| Resumed | `--resume ID` instead of `--session-id ID` | `… EXTRA… resume ID PROMPT` | `… EXTRA… --conversation ID -p PROMPT` | `… EXTRA… --session ID -- PROMPT` |
| MCP | `mcp.json` + `claude-settings.json` in the run dir | `-c` settings, TOML strings (agentbus tool timeout 900 s; Serena start-up 120 s, tool 300 s) | `<worktree>/.agents/mcp_config.json`, excluded from git | `OPENCODE_CONFIG_CONTENT` (+ copy in run dir); OpenRouter / DeepInfra providers; model registered under its provider |
| Environment | none (settings carry `ENABLE_TOOL_SEARCH=false`, Bash timeout 20 min, background tasks off, no attribution) | none | none | config, `OPENCODE_DISABLE_AUTOUPDATE=1`, `OPENCODE_EXPERIMENTAL_BASH_DEFAULT_TIMEOUT_MS=600000` |
| Final message | `result.result` | last `agent_message` after `turn.completed` | `result.response` | last `text` part |
| Succeeded when | `result` without error | `turn.completed` | `result.status == SUCCESS` | `step_finish` with reason `stop` |
| Session | `session_id` (init, result) | `thread_id` | `conversation_id` | `sessionID` of every event |

The arguments are in the order of the old `bin/agent` (`run_claude`, `run_codex`, `run_gemini`, `run_opencode`), which
is the order known to work. Where the new code differs from the old script, on purpose:

- **Codex has no `-o` file.** The final message is the last `agent_message` of a turn that completed; that is what the
  file held.
- **A run with no MCP server is configured with none.** Claude gets an empty `mcpServers` and `--strict-mcp-config`
  (the old investigators); the others get no entry. The old Codex form `mcp_servers.agentbus.enabled=false` needs a
  command, which a request without the server does not have.
- **No token or cost lines** in the activity (accounting is out of scope of the plan).
- **`agy`'s `call_mcp_tool` with `ServerName: agentbus` is a bus call.** The old normalizer showed it as a generic tool
  (found in the recordings).
- **Serena's flags are split.** The request carries the whole command line (`serena start-mcp-server --project DIR
  --open-web-dashboard false`); the adapter adds `--context claude-code|codex|antigravity|ide`.

## Decisions that need the merger's or the human's eye

1. **The error tail includes the stream's own error lines.** `classify_outcome` finds a quota or a rate limit only in
   `RunOutput::error_tail` and in the final message of a run without a `STATUS:` line. opencode writes the provider's
   refusal on stdout (`{"type":"error",…,"status":429}`, recorded in `fixtures/cli/opencode/rate-limit.jsonl`; the old
   harness paused that task for a rate limit), with an empty stderr. So `Progress::finish` appends the stream's error
   lines (the whole line for opencode, the message for Codex and Antigravity) after stderr. Without it the rate limit of
   a real opencode run would be `NoStatus`.
2. **Quirk settings are constants, not configuration keys** (`docs/configuration.md`, decision 7, which said 5.1
   decides). Turn limit 300, `agy` print timeout 8 h, opencode Bash timeout 10 min, Claude Bash timeout 20 min and tool
   search off. A new optional key stays compatible if wanted.
3. **Unrecognised vs unknown.** A line that is JSON but lacks what the adapter reads (no `type`, a missing or mistyped
   field) is `OutputCause::Unrecognized`; text that is not JSON (a cut line, a banner) is `Json`. An event of a *type*
   the adapter does not know is not an error: the CLIs add events constantly (`rate_limit_event`, `turn.started`, …).
   `Unrecognized` carries no source in the 2.3 contract, so serde's own message is dropped there (the line is in the
   error).
4. **Efforts.** The item names "effort flags" for `agy`. `agy --effort` exists (checked with `agy --help`), but a
   `RunRequest` has no effort field, so it travels as the agent's `args` (`args = ["--effort", "high"]`), like Codex's
   `-c model_reasoning_effort=…`. A test pins that it reaches `agy` before `-p`.

## Not done here (gaps for 6.1; none is in the item)

- Laying down a `RunFile` (the runtime must create its directories: `.agents/` in the worktree) and keeping
  `worktree_excludes` out of git — documented in `cli.rs` and `docs/architecture.md`.
- A prompt too long for one argument. The old script wrote it to `.agent-prompt.md` and passed a request to read it
  (`AGENT_PROMPT_ARG_MAX`, 100 000 bytes). That sentence is prompt text, so it needs a prompt-catalog entry and a
  `RunSpec` file; the adapters pass the prompt as one argument, so a prompt over Linux's 128 KiB fails to start.
- opencode's old fallback for a resumed session whose result is not printed (`opencode session export`): a second
  process, so the run engine's. The parser reports `Incomplete` with no final message in that case.
- Antigravity's response pieces that never got their `DONE` (a killed run) were once shown as text at the end of the
  stream; the parser has no end-of-stream event, so they are not.

## Size

`1,828` changed lines outside tests by the PR-1 command (limit `5,000` for this plan); `4,086` added / `4` deleted in
`30` files in all: `1,796` lines of integration tests, `165` of fixtures, `192` of docs and the report, and the unit tests
at the end of `src/cli.rs` (not counted).

## Tests

Test-first, in order:

- [`b2f811d`](https://github.com/mateusz-szczyrzyca/team-agents/commit/b2f811d1b9523eaa3d8943a1e3c0104aef434942) RED: 
  `6 + 15 + 11 + 12 + 15` failing tests (helpers, Claude, Codex, Antigravity, opencode) against compiling stubs with
  wrong bodies.
- [`c5e0942`](https://github.com/mateusz-szczyrzyca/team-agents/commit/c5e094240255bf0bf39b0be35be6e5e65c40e85d) GREEN.
- [`588750c`](https://github.com/mateusz-szczyrzyca/team-agents/commit/588750c8e0c2a959181fb802681d9211b62d4d6b) RED: a
  null `is_error`, found by thinking about Claude's output after the first GREEN.
- [`5ef6386`](https://github.com/mateusz-szczyrzyca/team-agents/commit/5ef638633c0d921c17b6229e8f25d2d4da9d6e6e) GREEN.

The tests added after the RED commit: the Antigravity error-tail test now expects the result's error after stderr
(design decision 1 was made while writing GREEN), and the four test files were merged into one test crate so that the
helpers they share are all used. Both are tests of this change; no test of `main` was touched.

The item's checks:

- **Exact arguments for a new and a resumed run, and an MCP configuration naming `agentbus serve`**, per CLI:
  `a_new_run_has_these_exact_arguments`, `a_resumed_run_has_these_exact_arguments`,
  `the_mcp_configuration_names_agentbus_serve_…` (Claude, Antigravity, opencode) and the `-c` settings of Codex
  (`BUS_SETTINGS`) in `tests/cli_adapters/{claude,codex,agy,opencode}.rs`.
- **Recorded output gives the final message and the session ID**: `the_recorded_run_gives_its_activity_final_message_and_session`
  per CLI, on fixtures that are lines of real runs (see `fixtures/cli/README.md`: three files are hand-written, the
  rest recorded; no CLI was run for this).
- **A cut output gives a typed error, not a panic**: `an_output_cut_at_any_byte_gives_a_typed_error_and_no_panic`
  cuts a recorded run at *every* character boundary for Claude, Codex, Antigravity and opencode, and asserts: at most
  one error, always `OutputCause::Json`, an `Incomplete` stream and no final message until the last line is whole.
- Mutation checks (TDD-2), each broken and seen red: the Antigravity response bound, the TOML escape of U+007F, the
  stream error lines of opencode, the bound on error lines.

## Gate

- `agent gate` at `5ef6386`: **PASS** (`make gate`: `cargo fmt --check`, clippy `-D warnings`, `cargo test`, `cargo doc`
  `-D warnings`, all `--locked`, and the Go/bats/lint steps).
- **A flaky test that is not mine:** the first two gate runs, at `c5e0942`, failed in `agent-runtime` with
  `git::tests::a_status_in_another_format_is_an_error` and `…::a_revision_that_prints_no_commit_id_is_an_error`:
  `Text file busy` (ETXTBSY). `scripted_git` in `src/git.rs` writes a script and `exec`s it while other tests spawn
  processes; a fork between the write and the close keeps the file busy. I measured the same flake on the base:
  `14/300` runs of the `agent-runtime` unit-test binary fail at `ccbae53`, `15/300` at my HEAD. I did not touch the
  tests (rule 1). The third run passed its tests and failed on two rustdoc links of mine (fixed in `5ef6386`); the
  fourth passed. Suggested fix, for a round that may touch `git.rs`: write the script to a
  temporary name, `sync` and rename it, or serialise the `scripted_git` tests behind one mutex.

## Deviations

- One `#[expect(clippy::expect_used, reason = …)]`, on `env_name` in `src/cli/opencode.rs`: a literal environment
  variable name is parsed into an `EnvName`, which only has a fallible constructor in `agent-core`; the opencode tests
  build all three names on every run, so a wrong literal fails there. (`BuildRunError` has no variant for an invalid
  environment name, and adding one is a change of the 2.3 contract.)
- No new dependency; `serde`, `serde_json`, `strum` and `agent-core` were already in `agent-runtime`.
- No `#[allow]`, no `unsafe`, no `#[ignore]`; no existing test changed.

## Out of scope (noticed, not touched)

- The existing flaky `scripted_git` tests (above).
- `agent-core`'s `AbsolutePath` and `Program` can only be made from a `String` but expose a `Path`, so every consumer
  must prove UTF-8 again (`cli::path_text`). An `as_str` on both would remove that.
- `tests/fixtures/cli-outputs/*.json` in `agent-core` are reconstructed texts of the *old* tests, not recordings; the
  new `agent-runtime` fixtures are real lines and could replace them for `classify_outcome`'s tests.

## Suggestions for `AGENTS.md` / the notebook

- Record in the notebook: opencode reports provider errors on stdout, not stderr; Codex's `-o` file equals the last
  `agent_message`; `agy` has `--effort`; `~/agents/tasks/*/out-1.jsonl` and `events-1.jsonl` of the live installation
  are real recordings of all four CLIs and the best source of fixtures (read-only; sanitise paths and names).
- `AGENTS.md` could say that `make gate` refuses an uncommitted tree and that `agent gate` is the way to run it.
