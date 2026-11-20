# Developer tasks. The gate of a change: `make gate` (see AGENTS.md).
#   make gate       the gate: the steps of GATE_STEPS with short output — the Rust workspace's checks (make rust)
#   make gate-full  every step, the lint and the tests of agentbus (Go) included: for a change of agentbus or the tools
#   make tools   installs the pinned linters (golangci-lint, staticcheck, shellcheck) into ~/.local/bin
#   make lint    every static check and the Rust gate (also the CI entry point); fails when a tool is missing
#   make test    unit tests: Go (agentbus) and the Rust workspace
#   make rust    the Rust workspace's checks: fmt, clippy -D warnings, tests, docs (docs/architecture.md)
.PHONY: gate gate-full test unit-go unit-bats lint lint-go lint-shell lint-docs tools build generate check-generated rust

# Go modules of this repository (each has its own go.mod): the bus the agents talk to, and the tools that check it.
GO_MODULES := agentbus tools/doccheck tools/errgen
# Bash scripts checked with shellcheck.
SHELL_SCRIPTS := tools/install-linters.sh tools/gate.sh tools/test-lock.sh examples/notify/*.sh
# Linters are looked up in PATH and in ~/.local/bin, where `make tools` installs them.
export PATH := $(HOME)/.local/bin:$(PATH)

# Every target that runs tests holds the machine's test lock (tools/test-lock.sh): one test run at a time. A busy
# lock refuses with "another tests are running now - please wait before you try again" (exit 75); GATE_WAIT=1 waits.
# A recipe copied alone into another Makefile (the harness's fixtures) finds TEST_LOCK empty there and runs unlocked.
TEST_LOCK = tools/test-lock.sh $@

test:
	@$(TEST_LOCK) $(MAKE) --no-print-directory unit-go rust

unit-go:
	@$(TEST_LOCK) bash -c 'set -e; for m in agentbus; do (cd $$m && go vet ./... && go test ./...); done'

# The Bash implementation and its Bats suite are gone (0.25.0). The target stays, with nothing to run, because the CI
# workflow still calls it and only the human changes .github/workflows/; remove it together with that step.
unit-bats:
	@echo "unit-bats: nothing to run — the Bash implementation and its Bats suite were removed"

# The Rust workspace (Cargo.toml, crates/; see docs/architecture.md): formatting, clippy with every warning an error,
# the tests (doctests included) and the docs with every warning an error. --locked: Cargo.lock is committed and must
# match. rustup installs the toolchain of rust-toolchain.toml with its components.
rust:
	@command -v cargo >/dev/null || { echo "cargo is missing: install rustup (https://rustup.rs)"; exit 1; }
	cargo fmt --all -- --check
	cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
	@$(TEST_LOCK) cargo test --workspace --all-features --locked
	RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --all-features --locked

tools:
	tools/install-linters.sh

# The same checks with short output — one line per step, the failures condensed, full logs in build/gate/
# (tools/gate.sh). Agents use this: long tool output stays in their context and is read again at every later step.
# The gate — what a change must pass, and what `agent gate` runs: the steps of GATE_STEPS, the Rust step alone. The
# other steps check agentbus (Go), which the rounds of the Rust workspace do not change.
# `make gate-full` runs every step: for a change of agentbus or the tools.
GATE_STEPS ?= rust

gate:
	@$(TEST_LOCK) tools/gate.sh $(GATE_STEPS)

gate-full:
	@$(TEST_LOCK) tools/gate.sh

# CI already runs this target; include the complete Rust gate in that same entry point.
lint: check-generated lint-shell lint-go lint-docs rust

lint-shell:
	@command -v shellcheck >/dev/null || { echo "shellcheck is missing: run make tools"; exit 1; }
	bash -n $(SHELL_SCRIPTS)
	@# Release sourced ASTs between files so small hosts retain full lint coverage.
	@lint_result=0; for script in $(SHELL_SCRIPTS); do \
		shellcheck -S warning "$$script" || lint_result=1; \
	done; exit "$$lint_result"

lint-go:
	@for t in golangci-lint staticcheck; do command -v $$t >/dev/null || { echo "$$t is missing: run make tools"; exit 1; }; done
	@for m in $(GO_MODULES); do \
		echo "== $$m"; \
		(cd $$m && test -z "$$(gofmt -l .)" && golangci-lint run --config $(CURDIR)/.golangci.yml ./... && staticcheck ./...) || exit 1; \
	done

# Every constant, variable, type and struct field has a comment (see AGENTS.md, "Go").
lint-docs:
	cd tools/doccheck && go build -o ../../build/doccheck .
	build/doccheck agentbus/*.go

build:
	cd agentbus && go build -o ../build/agentbus .

# Regenerates agentbus/errors_gen.go from agentbus/errors.json.
generate:
	cd agentbus && go generate ./...

# Fails when errors_gen.go is out of date with errors.json.
check-generated:
	mkdir -p build
	cd tools/errgen && go run . -spec ../../agentbus/errors.json -out ../../build/errors_gen.go
	cmp -s build/errors_gen.go agentbus/errors_gen.go || { echo "agentbus/errors_gen.go is out of date: run make generate"; exit 1; }
