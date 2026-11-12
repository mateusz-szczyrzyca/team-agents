# Item 4.2 — GitHub API and agentbus (`inv-claude-sonnet`)

Branch `agents/inv-claude-sonnet/20261112-134008`, base [`1171868`](https://github.com/mateusz-szczyrzyca/team-agents/commit/1171868c15518c832df4a0eb869be1706563bcc0), head [`45c0718`](https://github.com/mateusz-szczyrzyca/team-agents/commit/45c07183c04a6677a759f23678412f7b04155861). No pull request was opened (Phase 1). This file is written so that its sections can be lifted into the PR description.

## What and why

`agent-runtime` gets its two remote effects, each testable without GitHub and without a running team:

- [`GitHub`](https://github.com/mateusz-szczyrzyca/team-agents/blob/45c07183c04a6677a759f23678412f7b04155861/crates/agent-runtime/src/github.rs): a `reqwest` client for one repository. It creates, updates, marks ready, merges and reads a pull request, posts a review or a comment, and lists comments and reviews **filtered to the trusted accounts**. The token comes from `Secrets`, the base URL is injected.
- [`AgentBus`](https://github.com/mateusz-szczyrzyca/team-agents/blob/45c07183c04a6677a759f23678412f7b04155861/crates/agent-runtime/src/agentbus.rs): typed wrapper over the `agentbus` subcommands the daemon uses, with typed arguments, typed results, and exit statuses mapped to `FailureClass`.

## Task

> 4. Runtime services — Depends on: 2 (merged before this round)
>
> [ ] 4.2 GitHub API and agentbus
> `agent-runtime`, remote effects. A `reqwest` client for the GitHub REST API — create, update, mark ready, merge and read a PR, post a review or comment, list comments filtered to the trusted accounts; token from `secrets.toml`; timeouts and capped retries for idempotent calls only; tested against a local fake server.
> A typed wrapper over the `agentbus` commands the daemon uses (team create, get, set and done; board add, dump, claim and release; delegation queue; checkpoints; send; status), exit codes mapped to typed errors, tested against the real `agentbus` built from `agentbus/`.
> Expected result:
> - Files: `crates/agent-runtime/src/{github,agentbus}.rs`.
> - Adds: `GitHub` (the PR, review and comment operations, the trusted-account filter); `AgentBus` (the commands the daemon uses, typed results and errors).
> - Tests: each GitHub call against a local fake server, retries and timeouts included; each agentbus command against the real binary in a temporary bus directory.
> - Why: the two remote effects, testable without GitHub and without a running team.
>
> Checks:
> - The GitHub tests reach only the local fake server: its base URL is injected, none names api.github.com.
> - The agentbus tests run the binary built from `agentbus/` in a temporary bus directory and never touch `~/agents`.

## Size

`2,988` changed lines outside tests (PR-1 command of the rules; limit `5,000`, split threshold `4,000`). One PR, no split needed. Total diff `+7,184` / `−18` in `10` files, of which `Cargo.lock` is `+1,269` and the tests `+2,489`.

## Files

Expected by the item:

- [`crates/agent-runtime/src/github.rs`](https://github.com/mateusz-szczyrzyca/team-agents/blob/45c07183c04a6677a759f23678412f7b04155861/crates/agent-runtime/src/github.rs)
- [`crates/agent-runtime/src/agentbus.rs`](https://github.com/mateusz-szczyrzyca/team-agents/blob/45c07183c04a6677a759f23678412f7b04155861/crates/agent-runtime/src/agentbus.rs)

Outside the list, each with its reason:

- `Cargo.toml`, `crates/agent-runtime/Cargo.toml`, `Cargo.lock`: `reqwest` (approved: "reqwest (rustls)"), declared once in `[workspace.dependencies]` with `default-features = false, features = ["rustls"]` (DEP-2). `tokio` gets `net` and `test-util` as a **dev**-dependency of `agent-runtime` only, for the fake server and the paused clock. `Cargo.lock` is what `cargo build` wrote.
- [`crates/agent-runtime/src/lib.rs`](https://github.com/mateusz-szczyrzyca/team-agents/blob/45c07183c04a6677a759f23678412f7b04155861/crates/agent-runtime/src/lib.rs): two `pub mod` lines.
- [`crates/agent-runtime/src/process.rs`](https://github.com/mateusz-szczyrzyca/team-agents/blob/45c07183c04a6677a759f23678412f7b04155861/crates/agent-runtime/src/process.rs): `capture_with_input`. `agentbus team-create` reads the team JSON on its **standard input**, and `capture` closes stdin. `capture` now delegates to a shared `capture_feeding`; its behaviour is unchanged (the existing test `capture_closes_the_standard_input` is untouched and passes). Three tests were **added** to the existing test module (none changed).
- Tests: `crates/agent-runtime/tests/{github,agentbus}.rs` and `tests/support/mod.rs` (the fake HTTP server).

## Tests

Test-first: each module has a RED commit (types, signatures, wrong-value bodies, the tests, the failing run in the message) and a GREEN commit.

- $\color{red}{\textbf{RED}}$ [`d99eda4`](https://github.com/mateusz-szczyrzyca/team-agents/commit/d99eda4552354c8667dfbd5129d1cec3c5fdc59c): GitHub, `0` passed / `53` failed
- $\color{green}{\textbf{GREEN}}$ [`910b884`](https://github.com/mateusz-szczyrzyca/team-agents/commit/910b884d1a2e6f91381b098b8ae84f6192c6c879): GitHub, `58/58`
- $\color{red}{\textbf{RED}}$ [`e36e9d1`](https://github.com/mateusz-szczyrzyca/team-agents/commit/e36e9d11fcf624165593dde45c526f4f9347a4b3): AgentBus and stdin capture, `36` integration + `18` unit + `2` process tests failed
- $\color{green}{\textbf{GREEN}}$ [`2d14934`](https://github.com/mateusz-szczyrzyca/team-agents/commit/2d14934e6e8a8a728de64308543d57aaa5aa0c6e): `39/39` integration, `26/26` unit
- [`1b211c2`](https://github.com/mateusz-szczyrzyca/team-agents/commit/1b211c2307c1e1eb1aca224328042c2a7433334b): `ReviewState` reads like the other keyword enums (REFACTOR; one test added for an unknown state)
- [`8bfe537`](https://github.com/mateusz-szczyrzyca/team-agents/commit/8bfe53720630c3a440f939e1f3f5a06650adb0ea) and [`45c0718`](https://github.com/mateusz-szczyrzyca/team-agents/commit/45c07183c04a6677a759f23678412f7b04155861): results of the mutation checks below

Final counts: `60` GitHub tests, `39` agentbus integration tests, `26` unit tests in `agentbus.rs`, `3` new tests in `process.rs`.

GitHub (fake server on a loopback port, `127.0.0.1:0`):

- every call: method, path, headers (`Authorization: Bearer`, `Accept`, `X-GitHub-Api-Version`, `User-Agent`), JSON body, decoded result
- statuses to `Rejection` (`401 403 404 405 409 422 429 5xx`), GitHub's message bounded to `512` characters, a non-JSON error body
- retries: a read and an update are retried on `5xx`, transport errors and timeouts with doubling delay; a rate limit with a short `Retry-After` is waited for, with a long one reported; `create`, `merge`, `post_review`, `post_comment` and the `mark_ready` mutation are made **once** (a test per call), on the paused clock
- timeouts: a server that never answers gives `Timeout { attempts: 3 }` for a read and `attempts: 1` for a post
- trusted accounts: case-insensitive, deleted accounts and bots that are not valid logins are counted, never shown; pagination (`100` per page, stops at a page one short of full, gives up after `20` pages)
- hardening: a redirect is an error and the token is not sent on; an answer over `8 MiB` is refused; `http://` is accepted for loopback only

AgentBus (real binary, `go build` of `agentbus/` once into `CARGO_TARGET_TMPDIR`, a `tempfile` `AGENTS_HOME` per test):

- every command, including a team whose members are read back, the three wake outcomes of `team-set` (`Woken::{No, Message, Board}`), board numbering, designation, claim and release, the queue (`delegate`, `helper-next` once per request, `helper-set` field by field, `helper-get`), checkpoints (non-ASCII), `send` (chat, notice with fields, text starting with `-`), `status` with an agent that really runs `agentbus serve`
- exit statuses to `FailureClass` for the classes the bus gives in these commands (`InvalidArgument`, `NotFound`, `NotInTeam`, `Permission`, `Conflict`), `Unclassified` for status `1`, `Run`/`Timeout` for a lock held by the test (`flock` on `bus.lock`) with the paused clock, `Run`/`Spawn` for a missing program
- in the crate: the exit-status table `2..=10`, signals, output over the limit, non-UTF-8, and each reader of what a command prints with the formats of the Go code

Mutation checks (a change to the code, the tests must go red): about twenty mutations were tried (merge retried, trust everyone, redirects followed, `404` retried, no paging, page size off by one, no token, timeouts not retried, long `Retry-After` capped instead of refused, no backoff growth, draft flag, `mark_ready` always mutating, level fixed, no `--` before the text, summary dropped, board wake confused, state fixed, lead dropped). Two survived at first (page size off by one, board level) and got tests; one survives by design (see Deviations: the dispatcher).

## Checks of the item

1. *The GitHub tests reach only the local fake server.* `grep -rn "api\.github\.com" crates/agent-runtime/tests` finds nothing. In `src/github.rs` the name is in the `PUBLIC_API` constant and two doc comments. The base URL is a parameter, and `ApiBase` accepts `https://` or `http://` to a **loopback** host only, so a test cannot send the token anywhere else by mistake.
2. *The agentbus tests run the binary built from `agentbus/` in a temporary bus directory and never touch `~/agents`.* The binary is built by the test from `agentbus/`; every `AgentBus` is made on a `tempfile` directory, and every process the tests start has `AGENTS_HOME` set to it and `--dir` pointing into it. After the runs, a search of `~/agents/logs` and `~/agents/bus` for the test team's ID found nothing.

## Gate

- `make rust`: pass (`cargo fmt --check`, clippy `-D warnings`, tests, doctests, `cargo doc` with warnings denied, all `--locked`)
- `make lint`: pass, `0` findings
- `make gate`: `PASS` at the head of the code (the first run, on an earlier commit, once failed `git::tests::output_that_is_not_utf8_is_an_error` (`ExecutableFileBusy`, see below; the later runs passed)
- `make test`: Go packages pass (`agentbus`, `agentlive`, `agent`); the bats suite has **`8`** failing tests of the old investigation flow (`234 248 249 250 251 270 654 672`: an `investigation-team` task stays `running`). I ran `bats -f "single investigator" tests/unit` (`3` of them) on the **base commit** in a separate worktree: the same `3` fail there. The other `5` are the same family; I did not run them on the base.
- repeated runs for flakiness: the GitHub tests `40/40`, the agentbus tests `25/25`, the lib tests `40/40`

## Deviations

- **A file outside the item's list** (`process.rs`): `capture_with_input`, reason above.
- **`GitHubError::Client` has no test.** `reqwest::ClientBuilder::build` fails only when the TLS backend cannot start; there is nothing to inject (TDD-6). It is one `map_err` in `GitHub::new`.
- **The dispatcher argument of `delegate` is observable** (review pass 1, finding 8): `agentbus delegate --agent-cmd X` starts `X _dispatch-helpers` detached, and a recording script in `$AGENTS_HOME/dispatched` shows that it ran, with which arguments and which home. The test `delegate_runs_the_dispatcher_it_is_given_with_dispatch_helpers_in_its_home` fails when the dispatcher or the home is another.
- **`.ok()` in the readers of printed text** (`parse_added`, `parse_released`, `parse_agent_line`, `leading_number`): a number or a name that does not parse becomes `AgentBusError::Unexpected { output }` through `ok_or_else`; the printed text is in the error and there is no OS cause to keep. Same for `Err(_) => String::new()` of GitHub's error body (specified: "a non-JSON error body gives an empty message", tested) and `unwrap_or_default()` of a `null` body.
- **Jitter is not from `rand`.** `Jitter::Spread` takes the sub-second part of `RandomState` (the OS-seeded hasher of std): no new feature of `rand`, no RNG in the struct. `Jitter::None` makes the delay exact in tests.
- **`BusTier` has no `Investigator`.** The Go `Tier` knows `normal` and `worker` only and refuses anything else when it reads a team; the daemon maps an investigator that coordinates to `Normal` when it creates the team.
- No `#[expect]`, no `#[allow]`, no `unsafe`, no lint or CI configuration changed.

## Findings about `agentbus` that the next items need

Learned by running the real binary; each is stated by a test.

- Creating a team posts a message for `team:<id>`. A member that goes to sleep without having read it is woken at once (`Woken::Message`). The bus has **no command to read messages** (`read_messages` is an MCP tool of `agentbus serve`); the tests drive it over stdio JSON-RPC. The daemon will need the same, or a `Woken::Message` after the creation of every team is expected.
- `team-set` when the team is `done` never wakes anyone.
- `board-add` and `board-claim` check the team and the item, **not that the caller is a member**: `myTeam` returns the team named by `--team`. A non-member can add to a board.
- `board-add` prints `Added B3 (normal). The result …` (the Go comment says it prints the ID). The reader takes the ID from there.
- `delegate` on the command line does not know the tier of the requester (`Server.tier` is not set), so it cannot refuse a worker; the MCP tool can.
- Checkpoints are read with `checkpoint-get`, **written only through the MCP tool**; the key is `team-` + the team ID, so with the new IDs it is `team-team-2026…`.
- Timestamps in the bus are `2006-01-02 15:04:05` in local time, not RFC 3339. `AgentBus` reads none of them (the daemon keeps its own times).
- A command waits for `bus.lock` without a limit; `AgentBus` bounds it (`60 s` by default, `with_timeout`) and kills the process group.
- `agentbus status` prints free text (an agent's status, a reservation's resource and purpose), and that text may have line breaks. A line that starts no record goes on the text of the record before it (review pass 1, finding 3); agents are parsed, reservations are returned as lines.

## For the merger (Phase 2)

- Other investigators implement the same item. The likeliest conflicts: `Cargo.toml` and `Cargo.lock` (`reqwest` version and features: `0.13.5`, `rustls`), `process.rs` (a stdin-capable capture), `lib.rs` (two lines).
- Public names to compare or unify: `GitHub`, `GitHubSettings`, `RetryPolicy`, `ApiBase`, `RepoSlug`, `RemotePullRequest`, `TrustedList<T>`; `AgentBus`, `AgentBusError`, `FailureClass`, `BusTeam`, `Delegation`, `DelegationRequest`.
- The tests are in a `#[cfg(test)] mod` inside each `tests/*.rs` file, as in `public_api.rs`, so that the lint allowances of tests apply to their helpers.

## Out of scope (noticed, not touched)

- The existing test `git::tests::output_that_is_not_utf8_is_an_error` writes an executable and runs it at once; under load it failed once with `ETXTBSY` ("Text file busy"). Existing tests are not mine to change; it deserves a fix of its own (for instance, run the script through `sh`).
- `docs/architecture.md` lists the modules of `agent-core` but none of `agent-runtime`; a list of its modules would help the next items.
- The bats failures of the old investigation flow on the base commit (above).

## Suggestions for `AGENTS.md`

None needed for this item. One for the Rust section: say how a test that needs an executable file should get one (a script on disk races with `fork` in other test threads).


## Review pass 1 (PR #91, the review of `b016dad`)

- Finding 1: `GitHub::post_comment_review` and `CommentReview` send the `COMMENT` event, the one the author of a pull request may post. `NewReview.verdict` stays a `ReviewVerdict` (changing its type would have changed the committed tests); a private `ReviewEvent` carries the wire text.
- Finding 2: `mark_ready` reads the answer of the mutation: no `data` is a `Decode` error, `isDraft: true` is the new `GitHubError::StillDraft`.
- Finding 3: `parse_status` keeps the line breaks of a status, a resource and a purpose.
- Finding 4: `MergeMethod`, `StateChange` and `RawState` take their texts from strum.
- Finding 5: `GitHubError::InvalidToken` and `GitHubError::Encode` are removed.
- Finding 6: needs the human. The two `NotInTeam` cases of the RED commit were wrong about the unchanged Go bus (above); a new test pins the real behaviour. They were corrected inside this PR, which adds the tests; no test of `main` changed. The human is asked on the PR to confirm.
- Finding 7: the history is published and is not rewritten. The edge tests that arrived with their implementation in `910b884` are named in the PR under Deviations; the work of this pass is red first where a failing test is possible (findings 1, 2, 3) and says `Test-First: not possible — <reason>` where it is not (4, 5, 6, 8).
- Finding 8: the dispatcher test above.
- Finding 9: the PR description is rewritten with the sections of PR-17.
