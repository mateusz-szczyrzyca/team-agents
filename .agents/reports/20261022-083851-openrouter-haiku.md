# Report: multi-instance team-agents (task 20261022-083851-openrouter-haiku)

Coordinator: openrouter-haiku. Team 20261022-083851-team. Integration branch
`agents/openrouter-haiku/20261021-213141`, draft PR #3.

## Milestone status

| Milestone | Status | Evidence |
|---|---|---|
| M0 design | done | `docs/multi-instance.md` (approved, round 4) |
| M1 hub mode | done, with gaps carried to M2 | integration `9d808f0`, `make lint test` zero findings, bats 23/23 |
| M2 instances | in progress | board B12 (registry + CLI adapters, codex-1), B13 (manifest + command channel, gemini-1), B14 (durable outbox, coordinator), B15 (outage state + watchdog exclusion) |
| M3 capabilities | not started | |
| M4 macOS port | not started | |
| M5 views and robustness | not started | |

## M1 checkpoint

What M1 delivers:
- Hub daemon with the `/v1/tools/<op>` routes (one route per operation, CLI included), typed errors per
  operation, replay ledger (24 h payload retention, ID tombstones), 25 s pending chunks.
- Hub authentication: `HUB_AUTH=tailscale` (LocalAPI whois) and `HUB_AUTH=token` (pinned self-signed
  fingerprint `HUB_CERT_FINGERPRINT`). Fail-closed run resolver (`82405db`, forged-identity test).
- Lead-local Unix socket (`<dir>/hub.sock`, mode 0600, stale socket unlinked at start). Lead agents
  reach the hub through it; requests are trusted as the local instance (`INSTANCE_NAME`).
- Lead client: `HUB_LISTEN` or an explicit `HUB_SOCKET` selects the socket. A missing or refused socket
  returns `Unavailable`. No token path and no local-file fallback.
- MCP forwarding for lead-run tools (`1426290`), kind-aware release decoder, errgen class sentinels for
  wire errors (`7ed067f`).

### Compatibility notes (rule 9)
- Single-instance behaviour is unchanged. bats 23/23 pass.
- One existing test changed with the human's explicit approval (round 2, questions-2.md):
  the fallback assertion in `TestHubClientEnvironmentAndPinnedTLS`, in the commit that replaced the
  fallback with socket routing (`9d808f0`).
- Removed fallback: the temporary local-file fallback for a lead with `HUB_LISTEN` set is gone. The log
  event `lead_local_fallback` (`LogLeadLocalFallback`) is kept as a retired name for log-schema
  compatibility; nothing emits it.
- Judgement call (flag for review): `HUB_LISTEN` selects the socket once the socket client exists. A
  missing socket is `Unavailable`, not a file fallback. An explicit `HUB_SOCKET` never falls back. The
  file path remains only for a lead with neither setting.

### Known gaps carried to M2 (must close before the PR is ready)
1. **Durable outbox (mandatory).** A new MCP call gets a new request ID, so there is no cross-call retry
   cache. Until M2 closes it: a caller that gets `Unavailable` or `OutcomeUnknown` must check the current
   state (e.g. `board_list`) before it resends a mutation. The error messages say so. Human approved the
   deferral (round 2).
2. **Outage state and watchdog exclusion** (design §2.6) are not implemented.
3. **CLI forwarding** is not wired in M1 (the byte adapter hook exists).
4. **Trusted run registration** is not built. The listener refuses to start without a trusted registry
   resolver (M2, B12).

### Lint suppressions so far (for the PR)
- `//nolint:gosec` on the socket path in `agentbus/hub_serve.go` (operator-configured `HUB_SOCKET` or
  `--dir`; no remote-controlled path). Each site has its reason on the same line.
- `//nolint:gosec` in new test files for permission tests that deliberately use insecure modes
  (0666/0777 endpoints, chmod 0700 directory traversal). Reasons on the same line.
- `//nolint:unparam` in `agentbus/agentbus_test.go` (approved in M0).
- Open: gemini-1 is replacing the bare `_ = os.Remove(...)` and `_ = conn.Close()` calls in
  `hub_serve.go` with logged side effects. Not yet pushed.

## Board
- B1–B5 are stale `todo` items from an earlier task. Their results were delivered in messages.
  Not part of this task's scope; to be closed at the end.
- B6–B11 done (codex-1, gemini-1).

## Open items before the PR is ready
- gemini-1 ignored-error fixes in `hub_serve.go`.
- M2–M5, the PR description (suppressions, compatibility, macOS CI proposal, manual verification),
  the final report, `TEAM DONE`, `gh pr ready`.
