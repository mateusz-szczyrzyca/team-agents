#!/usr/bin/env bats
# CI already invokes make lint: Rust failures must stop that same entry point.

setup() {
  REPO_ROOT=$(cd "$BATS_TEST_DIRNAME/../.." && pwd)
  ROOT=$(mktemp -d)
  mkdir -p "$ROOT/bin" "$ROOT/home"
  # Legacy checks are outside this test's scope; override only their recipes in a temporary fixture.
  cat > "$ROOT/legacy.mk" <<'MAKE'
check-generated lint-shell lint-go lint-docs:
	@:
MAKE
  cat > "$ROOT/bin/cargo" <<'CARGO'
#!/usr/bin/env bash
set -euo pipefail
printf '%s|%s\n' "$*" "${RUSTDOCFLAGS:-}" >> "$CARGO_CALLS"
if [[ ${FAIL_CLIPPY:-0} == 1 && $1 == clippy ]]; then
  exit 23
fi
CARGO
  chmod +x "$ROOT/bin/cargo"
}

teardown() { rm -rf "$ROOT"; }

# Run the repository's actual CI entry point with a fake Rust process boundary.
run_ci_lint() {
  run env HOME="$ROOT/home" PATH="$ROOT/bin:$PATH" CARGO_CALLS="$ROOT/cargo.log" \
    FAIL_CLIPPY="${1:-0}" make --no-print-directory -C "$REPO_ROOT" \
    -f Makefile -f "$ROOT/legacy.mk" lint
}

@test "the existing CI lint entry point runs every Rust gate check" {
  grep -qE '^[[:space:]]+run: make lint$' "$REPO_ROOT/.github/workflows/test.yml"
  run_ci_lint
  [ "$status" -eq 0 ]
  [ -f "$ROOT/cargo.log" ]
  run cat "$ROOT/cargo.log"
  [ "$status" -eq 0 ]
  [ "$output" = 'fmt --all -- --check|
clippy --workspace --all-targets --all-features --locked -- -D warnings|
test --workspace --all-features --locked|
doc --workspace --no-deps --all-features --locked|-D warnings' ]
}

@test "a Rust clippy failure fails the existing CI lint entry point" {
  run_ci_lint 1
  [ "$status" -ne 0 ]
  run cat "$ROOT/cargo.log"
  [ "$status" -eq 0 ]
  [ "$output" = 'fmt --all -- --check|
clippy --workspace --all-targets --all-features --locked -- -D warnings|' ]
}
