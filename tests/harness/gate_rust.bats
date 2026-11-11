#!/usr/bin/env bats
# Tests for the Rust workspace's checks in the gate: the `rust` Makefile target and its step in tools/gate.sh.

setup() {
  REPO_ROOT=$(cd "$BATS_TEST_DIRNAME/../.." && pwd)
  ROOT=$(mktemp -d)
  cat > "$ROOT/Makefile" <<'MK'
check-generated:
	@echo "check-generated ok"
lint-shell:
	@echo "lint-shell ok"
lint-go:
	@echo "lint-go ok"
lint-docs:
	@echo "lint-docs ok"
unit-go:
	@echo "unit-go ok"
unit-bats:
	@echo "unit-bats ok"
harness:
	@echo "harness ok"
rust:
	@echo "rust ok"
MK
}

teardown() { rm -rf "$ROOT"; }

@test "gate runs the rust step when the Makefile defines it" {
  run env GATE_ROOT="$ROOT" "$REPO_ROOT/tools/gate.sh"
  [ "$status" -eq 0 ]
  [[ "$output" == *"gate: 8 steps"* ]]
  [[ "$output" == *"ok    rust"* ]]
}

@test "a failing rust step shows cargo's diagnostics, not its progress" {
  cat >> "$ROOT/Makefile" <<'MK'
rust-fail:
	@echo "   Compiling agent-core v0.1.0 (/src/crates/agent-core)"
	@echo "    Checking clap v4.6.7"
	@echo "error[E0308]: mismatched types"
	@echo "  --> crates/agent-core/src/ids.rs:12:5"
	@echo "warning: unused variable: \`x\`"
	@echo "Diff in /src/crates/agent/src/main.rs:3:"
	@echo "test ids::tests::refuses_an_empty_name ... FAILED"
	@echo "---- ids::tests::refuses_an_empty_name stdout ----"
	@echo "thread 'ids::tests::refuses_an_empty_name' panicked at crates/agent-core/src/ids.rs:40:9:"
	@echo "test result: FAILED. 3 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out"
	@for i in $$(seq 1 30); do echo "noise line $$i"; done
	@exit 1
MK
  sed -i 's/^rust:$/rust: rust-fail/' "$ROOT/Makefile"
  run env GATE_ROOT="$ROOT" "$REPO_ROOT/tools/gate.sh" rust
  [ "$status" -ne 0 ]
  [[ "$output" == *"FAIL  rust"* ]]
  [[ "$output" == *"error[E0308]: mismatched types"* ]]
  [[ "$output" == *"--> crates/agent-core/src/ids.rs:12:5"* ]]
  [[ "$output" == *"warning: unused variable"* ]]
  [[ "$output" == *"Diff in /src/crates/agent/src/main.rs:3:"* ]]
  [[ "$output" == *"test ids::tests::refuses_an_empty_name ... FAILED"* ]]
  [[ "$output" == *"panicked at crates/agent-core/src/ids.rs:40:9:"* ]]
  [[ "$output" == *"test result: FAILED. 3 passed; 1 failed"* ]]
  [[ "$output" != *"Compiling agent-core"* ]]
  [[ "$output" != *"noise line"* ]]
}

@test "the repository's rust target runs fmt, clippy with warnings denied, the tests and the docs" {
  run make -s -n -C "$REPO_ROOT" rust
  [ "$status" -eq 0 ]
  [[ "$output" == *"cargo fmt --all -- --check"* ]]
  [[ "$output" == *"cargo clippy --workspace --all-targets --all-features --locked -- -D warnings"* ]]
  [[ "$output" == *"cargo test --workspace --all-features --locked"* ]]
  [[ "$output" == *'RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --all-features --locked'* ]]
}

@test "the repository's make test runs the rust target" {
  # make -n still runs the recipe that calls $(MAKE); TEST_LOCK_HELD keeps that dry run off the machine's test lock.
  run env TEST_LOCK_HELD=1 make -s -n -C "$REPO_ROOT" test
  [ "$status" -eq 0 ]
  [[ "$output" == *"cargo clippy --workspace"* ]]
}
