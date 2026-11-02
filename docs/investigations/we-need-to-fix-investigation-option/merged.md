# `--investigation` fixes — merged result (inv-opus, Phase 2)

Combines both Phase 1 results on base `facf3b8`. The findings files [`inv-opus.md`](inv-opus.md) and
[`inv-astra.md`](inv-astra.md) are kept unchanged. The combined patch is
[`we-need-to-fix-investigation-option.patch`](we-need-to-fix-investigation-option.patch) (`git am` on `facf3b8`).
It replaces the two per-investigator patches.

## Point by point

| # | Requirement | Result |
|---|---|---|
| 1 | One investigator allowed | Already on `main` (both investigators agree). The help text and README now show `--team "A"` (inv-astra). |
| 2 | Finished flow is `done`, not `failed` | Already on `main` for new runs. Legacy Phase 4 parents left `waiting` now finish with their recorded result (inv-astra). The PR comment says `--investigation`, not `--autopilot` (inv-opus). |
| 3 | No waiting for a decision | Already on `main` for new runs. Dispatch and supervision now finalize already-delivered legacy hand-offs that still wait (inv-astra). |
| 4 | Main dashboard shows real agents only | The team row was already filtered on `main`. Investigators now show under their own names, not as `investigator # (…)` / `inv#` / `for` (inv-opus). |
| 5 | Investigators may ask questions | Already on `main` (both agree). |

## Open questions for the human (from inv-opus)

- A review without a `VERDICT:` line still ends as `failed`. An existing test requires that, so it was not changed.
- The installed `~/.local/bin/agent` is older than `main`. Points 1–3 and 5 reach you only after you reinstall.
