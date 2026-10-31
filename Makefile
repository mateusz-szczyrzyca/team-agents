# Developer tasks. Before every commit: `make lint test` (see AGENTS.md).
#   make tools   installs the pinned linters (golangci-lint, staticcheck, shellcheck) into ~/.local/bin
#   make lint    every static check; fails when a linter is missing (run make tools)
#   make test    unit tests: Go (agentbus, agentlive) and bats (the agent script)
.PHONY: gate test unit-go unit-bats lint lint-go lint-shell lint-docs tools integration build generate check-generated

# Go modules of this repository (each has its own go.mod).
GO_MODULES := agentbus live tools/doccheck tools/errgen
# Bash scripts checked with shellcheck (bats files included: shellcheck understands @test blocks).
SHELL_SCRIPTS := bin/agent install.sh tools/install-linters.sh tools/update-web-vendor.sh tests/integration/run.sh examples/notify/*.sh \
	tests/unit/helpers.bash tests/unit/node_agents.bats tests/unit/node_report_agent.bats tests/unit/node_sync.bats tests/unit/node_sync_limits.bats tests/unit/node_sync_review.bats tests/unit/node_sync_state.bats tests/unit/node_sync_tails.bats tests/unit/node_sync_two_homes.bats tests/unit/node_sync_freshness.bats tests/unit/job_attribution.bats tests/unit/job_attribution_paths.bats tests/unit/web_jobs.bats tests/unit/lead.bats tests/unit/lead_cli.bats tests/unit/lead_cli_context.bats tests/unit/lead_install.bats tests/unit/agent.bats tests/unit/web.bats tests/unit/rounds.bats tests/unit/identity.bats tests/unit/limits.bats tests/unit/gate.bats tests/unit/skips.bats tests/unit/meta.bats tests/unit/authenticated.bats tests/unit/verify_loop.bats tests/unit/kick.bats tests/unit/findings.bats tests/unit/rotation.bats tests/unit/member_out.bats tests/unit/resilience.bats tests/unit/remote_proxy.bats tests/unit/remote_fixes.bats tests/unit/restart.bats tests/unit/team_close.bats tests/unit/autopilot_review.bats tests/unit/autopilot_merge.bats tests/unit/network.bats tests/unit/autopilot_questions.bats tests/unit/github_format.bats tests/unit/review_resilience.bats tests/unit/run_notes.bats tests/unit/github_trust.bats tests/unit/agent_gh.bats tests/unit/auto_stage.bats tests/unit/pause_notify.bats tests/unit/round_handover.bats tests/unit/wake_cmd.bats tests/unit/round_reports.bats tests/unit/node_activity.bats tools/gate.sh tests/unit/resume_alloc.bats tests/unit/node_checkout.bats tests/unit/node_install.bats tests/unit/node_two_homes.bats tests/unit/node_daemon.bats tests/unit/node_helper_hooks.bats tests/unit/node_large_assignment.bats tests/unit/delegate_jobs.bats tests/unit/node_safety.bats tests/unit/node_prelaunch_cancel.bats tests/unit/node_cancel_exit.bats tests/unit/node_recovery_fence.bats tests/unit/node_review_boundaries.bats tests/unit/stubs/systemd-run tests/unit/stubs/systemctl tests/unit/stubs/gh tests/unit/stubs/tailscale
SHELL_SCRIPTS += tests/unit/remote_controls.bats tests/unit/remote_control_homes.bash \
	tests/unit/remote_control_validation.bats \
	tests/unit/remote_control_heartbeat_limits.bats tests/unit/remote_control_cancel_order.bats \
	tests/unit/remote_control_proxy.bats tests/unit/remote_control_kick_exit.bats
SHELL_SCRIPTS += tests/unit/lint_shell_memory.bats tests/unit/remote_control_review.bats \
	tests/unit/handover_failed.bats tests/unit/prompt_file.bats
# Linters are looked up in PATH and in ~/.local/bin, where `make tools` installs them.
export PATH := $(HOME)/.local/bin:$(PATH)

test: unit-go unit-bats

unit-go:
	cd agentbus && go vet ./... && go test ./...
	cd live && go vet ./... && go test ./...

unit-bats:
	bats tests/unit

tools:
	tools/install-linters.sh

# The same checks with short output — one line per step, the failures condensed, full logs in build/gate/
# (tools/gate.sh). Agents use this: long tool output stays in their context and is read again at every later step.
gate:
	@tools/gate.sh

lint: check-generated lint-shell lint-go lint-docs

lint-shell:
	@command -v shellcheck >/dev/null || { echo "shellcheck is missing: run make tools"; exit 1; }
	bash -n bin/agent install.sh tools/install-linters.sh tests/integration/run.sh
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
	build/doccheck agentbus/*.go live/*.go

# Real agent CLIs, real accounts (costs usage). See tests/integration/run.sh.
integration:
	AGENT_IT=1 tests/integration/run.sh

build:
	cd agentbus && go build -o ../build/agentbus .
	cd live && go build -o ../build/agentlive .

# Regenerates agentbus/errors_gen.go from agentbus/errors.json.
generate:
	cd agentbus && go generate ./...

# Fails when errors_gen.go is out of date with errors.json.
check-generated:
	mkdir -p build
	cd tools/errgen && go run . -spec ../../agentbus/errors.json -out ../../build/errors_gen.go
	cmp -s build/errors_gen.go agentbus/errors_gen.go || { echo "agentbus/errors_gen.go is out of date: run make generate"; exit 1; }
