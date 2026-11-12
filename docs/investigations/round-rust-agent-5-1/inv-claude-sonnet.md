# Item 5.1 — Claude Code, Codex, Antigravity and opencode adapters

Investigator `inv-claude-sonnet`, branch `agents/inv-claude-sonnet/20261112-154231`, base
[`ccbae53`](https://github.com/mateusz-szczyrzyca/team-agents/commit/ccbae53f5a4b8efbaa395aac8037ba2937d018bc).
Status: done; the code is pushed, no pull request opened (the merger integrates). See "Second pass" at the end: a run
of task `20261112-162930-inv-claude-sonnet` started from this branch's tip and checked the adapters against the real
recordings of the live installation.

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
| Succeeded when | `result` without error | `turn.completed` | `result.status == SUCCESS` | `step_finish` with reason `stop`, or the stream ends on the agent's last `text` (second pass) |
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

## Second pass: the adapters against real recordings

Task `20261112-162930-inv-claude-sonnet` found the item already implemented on its base (this branch's earlier tip,
which is the open PR #92 plus a merge of `main`; no review or comment existed). It did not redo the work. It ran every
parser over every run of the live installation (read-only; `~/agents/tasks/*/out-N.jsonl` and `events-N.jsonl`, first
three runs of each task, with a throwaway test that is not committed), and read the result against the old harness's
own verdict in each task's `runner.log`.

Before the fix, on `328` Claude, `366` Codex, `325` Antigravity and `611` opencode run files: `0` lines refused for
Claude, Codex and Antigravity; `1` for opencode; a session ID in all but the empty files. Two defects, both in opencode:

1. **A finished opencode run was `Incomplete`.** `596` of `611` files had no `step_finish` with `stop`; `486` of the
   tasks end on the `text` line, `exit 0`, and the old harness reported `123` of the last `150` of them as done (a
   `STATUS:` line in `final.md`). `classify_outcome` accepts a reported status only from a `Succeeded` stream (or a
   watchdog kill), so every one would have been `NoStatus` and, with a team, retried or sent to the human. The first
   pass had kept `tools.jsonl` ("tool calls and a final text but no `stop`") and asserted `Incomplete` for it.
   - Fix (`src/cli/opencode.rs`): the parser tracks whether the latest event is the agent's text (a tool call, a
     `step_start` or a `step_finish` clears it); `finish` turns an `Incomplete` stream into `Succeeded` when it ends on
     the text. A `stop` and an `error` work as before. After the fix `29` files are `Incomplete`: runs cut inside a step.
2. **An empty `sessionID` hid an error.** A recorded error event has `"sessionID":""`; `note_session` refused it and
   the whole event was dropped, so the run neither failed on the stream nor had the line in the error tail. An empty ID
   is now none.

Tests, test first: [RED] `opencode: a run that ends on its answer succeeds: test first` (3 failing), then GREEN. New:
`a_run_that_ends_on_its_answer_succeeds_without_a_stop` (a table: answer, answer then tool call / finished step / new
step / error, no answer) and `an_error_event_with_an_empty_session_id_is_still_the_error_of_the_run`. Two tests of
this change, not of `main`, followed the recordings: `shell_calls_are_commands_…` (`tools.jsonl` is `Succeeded`) and
`an_output_cut_at_any_byte_…` (a cut run is `Incomplete` while it has no final message, no longer "until the last line
is whole": a cut after the text is a finished run in the recordings). `agent gate` on the final commit: PASS.

The other `Incomplete` runs of the other CLIs (`19` Claude, `21` Codex, `11` Antigravity) end in the middle of a step
(a tool result, a heartbeat, a `step_update`) with no result: killed or still running, correctly incomplete.

Not changed, for a later item: the Claude, Codex and Antigravity result lines are fine on all recordings, but the
classification of an opencode run killed after its text rests on the exit status the run engine (6.1) gives.

## Third pass: task `20261112-172517-inv-claude-sonnet` (no code change)

This task started from [`6424843`](https://github.com/mateusz-szczyrzyca/team-agents/commit/6424843c48ef2277a6bb6c59a0b16d3994baf381),
the tip of the open PR #92 (which has no review and one comment: the script could not bring a later investigation onto
the PR because its branch had moved). The item was already implemented and checked twice, so it was not redone.

What was checked, independently of the two earlier reports:

- **Arguments against the old dispatch.** `run_claude`, `run_codex`, `run_gemini`, `run_opencode` and
  `opencode_config` of `bin/agent` read line by line against `build_run` of the four adapters: the order of the
  arguments, the position of the model, extra arguments, the session flag and the prompt, the MCP texts, the Codex
  timeouts (`900` / `120` / `300` s) and the opencode environment. No difference except the documented ones (no `-o`
  file for Codex; no server means no entry; `ENABLE_TOOL_SEARCH=false`, turn limit `300`, `agy` timeout `8h` and the
  Bash timeouts are constants — they equal the defaults of `CLAUDE_TOOL_SEARCH`, `CLAUDE_MAX_TURNS`,
  `AGY_PRINT_TIMEOUT`, `CLAUDE_BASH_TIMEOUT_MIN` and `OPENCODE_BASH_TIMEOUT_MS` in `bin/agent` and
  `config/agents.conf`).
- **Outcome rules of the old runners.** Claude `is_error` with exit `0` is a failure, `agy` with a status other than
  `SUCCESS` is a failure, opencode with an `error` line and no text is a failure: each is a failed stream in the new
  parsers, which `classify_outcome` then reads.
- **Gate.** `make rust` (`cargo fmt --check`, clippy `-D warnings`, tests, `cargo doc`, all `--locked`) at this tip:
  exit `0` on the second run. The first run failed on `git::tests::output_that_is_not_utf8_is_an_error` with
  `Text file busy` (ETXTBSY) — the `scripted_git` flake described under "Gate" above, not touched here (rule 1).

Nothing was found that needs a change, so no commit besides this note. The open points stay those above: the flaky
`scripted_git` tests, the over-long prompt argument and opencode's `session export` fallback (both for 6.1).

## Review pass 2 of 5 (review by inv-astra, five major findings)

Fixed, test first (RED commit, then GREEN commit, in this order on the branch):

- **Finding 1, Claude `errors`.** `claude/errors-only.jsonl` (hand-written after the documented result schema, listed in
  `tests/fixtures/cli/README.md`) and the test
  `an_error_result_without_a_result_text_keeps_its_errors_in_the_activity_and_the_error_tail` failed with an empty
  error tail; `Finish` now reads `errors` and each message is an `ActivityEvent::Error` and a line of the error tail.
- **Finding 3, Antigravity open responses.** The test
  `responses_that_are_never_done_are_bounded_in_number_and_the_open_ones_stay_usable` failed (the 65th open response
  was accepted). A new response step beyond `MAX_OPEN_RESPONSES` (`64`) is refused with the new
  `OutputCause::TooManyOpenSteps` (a variant added in `agent-core`, since no existing cause says it) and not kept; the
  parser goes on, and an open response can still finish.
- **Finding 5, Codex `_ =>`.** `CodexParser::started` lists the items that show no step when they start. A refactor
  with no behaviour change (`Test-First: not possible — TDD-4`).

Not changed, and why:

- **Finding 2, authoritative MCP servers.** Both halves are a change of an existing test's expectation, which rule 1
  forbids without the human: `a_run_with_agentbus_is_not_strict` pins the absence of `--strict-mcp-config` for a run
  that has servers, and the exact-argument tests of Codex pin the arguments, so an `enabled=true` per server or
  `enabled=false` for the omitted ones would break them. The Bash version did the same as the code does now (`strict`
  only for a run with no server; for Codex `enabled=false` only for the server it switches off). The human decides
  whether the new `agent` should be stricter than the old one. Question for the human: *should a run's MCP set be
  exclusive (`--strict-mcp-config` for every Claude run; Codex `enabled=true` for the given servers and
  `enabled=false` for the managed servers that are not given), at the price of changing those tests?*
- **Finding 4, history of tests.** The reviewer says it needs the human's explicit acceptance (or restoring the old
  tests); an agent cannot give it, and published history is not rewritten.
