# Developer tasks. Before every commit: `make lint test` (see AGENTS.md).
#   make tools   installs the pinned linters (golangci-lint, staticcheck, shellcheck) into ~/.local/bin
#   make lint    every static check; fails when a linter is missing (run make tools)
#   make test    unit tests: Go (agent, agentbus, agentlive) and bats (the agent script)
.PHONY: gate test unit-go unit-bats harness lint lint-go lint-shell lint-docs tools integration build generate check-generated build-agent unit-agent-bats check-agent-parity

# Go modules of this repository (each has its own go.mod).
GO_MODULES := agentbus live agent tools/doccheck tools/errgen
# Bash scripts checked with shellcheck (bats files included: shellcheck understands @test blocks).
SHELL_SCRIPTS := bin/agent install.sh tools/install-linters.sh tools/unit-bats.sh tools/tmp-sweep.sh tools/update-web-vendor.sh tests/integration/run.sh examples/notify/*.sh \
	tests/unit/helpers.bash tests/unit/node_agents.bats tests/unit/node_report_agent.bats tests/unit/node_sync.bats tests/unit/node_sync_limits.bats tests/unit/node_sync_review.bats tests/unit/node_sync_state.bats tests/unit/node_sync_tails.bats tests/unit/node_sync_two_homes.bats tests/unit/node_sync_freshness.bats tests/unit/job_attribution.bats tests/unit/job_attribution_paths.bats tests/unit/web_jobs.bats tests/unit/lead.bats tests/unit/lead_cli.bats tests/unit/lead_cli_context.bats tests/unit/lead_install.bats tests/unit/agent.bats tests/unit/web.bats tests/unit/rounds.bats tests/unit/identity.bats tests/unit/limits.bats tests/unit/gate.bats tests/unit/skips.bats tests/unit/meta.bats tests/unit/authenticated.bats tests/unit/verify_loop.bats tests/unit/kick.bats tests/unit/findings.bats tests/unit/rotation.bats tests/unit/member_out.bats tests/unit/resilience.bats tests/unit/remote_proxy.bats tests/unit/remote_fixes.bats tests/unit/restart.bats tests/unit/team_close.bats tests/unit/autopilot_review.bats tests/unit/autopilot_merge.bats tests/unit/network.bats tests/unit/autopilot_questions.bats tests/unit/github_format.bats tests/unit/review_resilience.bats tests/unit/run_notes.bats tests/unit/github_trust.bats tests/unit/agent_gh.bats tests/unit/auto_stage.bats tests/unit/pause_notify.bats tests/unit/round_handover.bats tests/unit/wake_cmd.bats tests/unit/round_reports.bats tests/unit/node_activity.bats tools/gate.sh tests/unit/resume_alloc.bats tests/unit/node_checkout.bats tests/unit/node_install.bats tests/unit/node_two_homes.bats tests/unit/node_daemon.bats tests/unit/node_helper_hooks.bats tests/unit/node_large_assignment.bats tests/unit/delegate_jobs.bats tests/unit/node_safety.bats tests/unit/node_prelaunch_cancel.bats tests/unit/node_cancel_exit.bats tests/unit/node_recovery_fence.bats tests/unit/node_review_boundaries.bats tests/unit/stubs/systemd-run tests/unit/stubs/systemctl tests/unit/stubs/gh tests/unit/stubs/tailscale
SHELL_SCRIPTS += tests/unit/remote_controls.bats tests/unit/remote_control_homes.bash \
	tests/unit/remote_control_validation.bats \
	tests/unit/remote_control_heartbeat_limits.bats tests/unit/remote_control_cancel_order.bats \
	tests/unit/remote_control_proxy.bats tests/unit/remote_control_kick_exit.bats
SHELL_SCRIPTS += tests/unit/remote_control_web.bats
SHELL_SCRIPTS += tests/unit/review_pair_models.bats tests/unit/github_attribution.bats
SHELL_SCRIPTS += tests/unit/remote_answer_round.bats tests/unit/remote_questions_atomic.bats \
	tests/unit/remote_questions_preparation_retry.bats
SHELL_SCRIPTS += tests/unit/handover_failed.bats tests/unit/prompt_file.bats tests/unit/review_byline.bats tests/unit/tokens.bats \
	tests/unit/context_hygiene.bats tests/unit/fresh_session.bats tests/unit/review_pair_random.bats tests/unit/resume_members.bats tests/unit/prompt_checkpoints.bats tests/unit/long_run_cut.bats tests/unit/round_only.bats tests/unit/fresh_relative.bats tests/unit/coordination.bats tests/unit/resume_stash.bats tests/unit/one_pr_per_round.bats tests/unit/investigation_mcp_off.bats tests/unit/round_discipline.bats tests/unit/member_pr.bats tests/unit/merged_elsewhere.bats tests/unit/foreground_commands.bats tests/unit/pr_ownership.bats tests/unit/tmp_sweep.bats tests/unit/review_overload.bats tests/unit/handback_wait.bats tests/unit/failover.bats tests/unit/worker_draw.bats tests/unit/fix_passes_default.bats tests/unit/fresh_session_id.bats tests/unit/notify_merged_start.bats tests/unit/notify_icon_first.bats tests/unit/push_text_icons.bats tests/unit/work_split.bats tests/unit/round_check.bats tests/unit/spend.bats tests/unit/script_split.bats tests/unit/coordinator_alone.bats tests/unit/pr_by_commit.bats tests/unit/pr_pin.bats tests/unit/next_round_pr.bats tests/unit/board_unstick.bats tests/unit/helper_failover.bats tests/unit/team_end_helpers.bats tests/unit/coordinator_takeover.bats tests/unit/debug_cmd.bats tests/unit/resume_large_board.bats tests/unit/install_model_prices.bats tests/unit/nostatus_autopilot.bats tests/unit/delegation_quality.bats tests/unit/debug_agent.bats tests/unit/fresh_member_done.bats tests/unit/test_lock.bats tests/unit/team_pause.bats tests/unit/precheck_rules.bats tests/unit/pr_round_size.bats tests/unit/agent_gate.bats tests/unit/board_done_gate.bats tests/unit/investigation_roles.bats tests/unit/helper_unclear.bats tests/unit/delegation_feedback.bats tests/unit/gate_queue.bats tests/unit/debug_all.bats tests/unit/unclear_look.bats tests/unit/usage_honest.bats tests/unit/gate_dirty.bats tests/unit/team_lineage.bats tests/unit/gate_own_run.bats tests/unit/gate_scopes.bats tests/unit/investigation_task_in_pr.bats tests/unit/serena_project_sync.bats tests/unit/allow_task.bats tests/unit/investigation_allow_wait.bats tests/unit/member_in_team.bats
SHELL_SCRIPTS += tests/unit/remote_control_notifications.bats
SHELL_SCRIPTS += tests/unit/lint_shell_memory.bats tests/unit/remote_control_review.bats \

SHELL_SCRIPTS += tests/unit/review_pair_rotate.bats tests/unit/autopilot_review_cap.bats
SHELL_SCRIPTS += tests/unit/investigation_team_cli.bats tests/unit/investigation_team_validation.bats \
	tests/unit/investigation_team_isolation.bats tests/unit/investigation_team_opencode_schema.bats
SHELL_SCRIPTS += tests/unit/investigation_team_runtime.bats tests/unit/investigation_team_supervision.bats \
	tests/unit/investigation_team_regressions.bats
SHELL_SCRIPTS += tests/unit/investigation_team_publication_retry.bats tests/unit/investigation_team_admission.bats \
	tests/unit/investigation_team_reboot_regression.bats
SHELL_SCRIPTS += tests/unit/investigation_team_stdin.bats
SHELL_SCRIPTS += tests/unit/investigation_team_phase_selection.bats tests/unit/investigation_team_handoff.bats \
	tests/unit/stubs/fake-investigation-git tests/unit/investigation_team_phase_queue.bats \
	tests/unit/investigation_team_recovery_regressions.bats
SHELL_SCRIPTS += tests/unit/investigation_team_resume_recovery.bats
SHELL_SCRIPTS += tests/unit/investigation_team_advisory_resume.bats
SHELL_SCRIPTS += tests/unit/investigation_team_resume_boundaries.bats
SHELL_SCRIPTS += tests/unit/investigation_single_investigator.bats tests/unit/investigation_clarification.bats \
	tests/unit/investigation_terminal_completion.bats tests/unit/investigation_pipeline_clarification.bats \
	tests/unit/investigation_prompt_clarification_resume.bats
SHELL_SCRIPTS += tests/unit/investigation_single_pipeline.bats tests/unit/investigation_pipeline_fixture.bash \
	tests/unit/investigation_independent_resume_prompt.bats
SHELL_SCRIPTS += tests/unit/investigation_resume_independence.bats
SHELL_SCRIPTS += tests/unit/investigation_independent_labels.bats
SHELL_SCRIPTS += tests/unit/investigation_completed_legacy.bats
SHELL_SCRIPTS += tests/unit/round_id.bats tests/unit/round_runs.bats tests/unit/tokens_round.bats tests/unit/round_team_run.bats
SHELL_SCRIPTS += tools/check-agent-test-map.sh tests/harness/check_agent_test_map.bats tests/harness/gate.bats \
	tools/agent-bats.sh tests/harness/agent_bats.bats tests/harness/agent_bats_runner.bats \
	tools/check-agent-parity.sh tests/harness/check_agent_parity.bats tests/harness/agent_parity_gate.bats
SHELL_SCRIPTS += tools/test-lock.sh
# Linters are looked up in PATH and in ~/.local/bin, where `make tools` installs them.
export PATH := $(HOME)/.local/bin:$(PATH)

# Every target that runs tests holds the machine's test lock (tools/test-lock.sh): one test run at a time. A busy
# lock refuses with "another tests are running now - please wait before you try again" (exit 75); GATE_WAIT=1 waits.
# A recipe copied alone into another Makefile (the harness's fixtures) finds TEST_LOCK empty there and runs unlocked.
TEST_LOCK = tools/test-lock.sh $@

test:
	@$(TEST_LOCK) $(MAKE) --no-print-directory unit-go unit-bats harness

unit-go:
	@$(TEST_LOCK) bash -c 'set -e; for m in agentbus live agent; do (cd $$m && go vet ./... && go test ./...); done'

harness:
	@$(TEST_LOCK) bats tests/harness/*.bats

# The Bats tests run in parallel, BATS_JOBS at once (GNU parallel; default: the number of processors), the slowest
# files first (tools/unit-bats.sh). A file listed in BATS_SERIAL runs one after another with the other listed ones,
# beside the parallel group: a file that does not bear running beside others. These failed now and then with every
# file in parallel, on a machine loaded by 32 jobs or by the agents' own work — tests of restarts, a node's heartbeat,
# a lead's start and a review's course, bound to timings such a machine misses. BATS_JOBS=1, or no GNU parallel: every
# file one after another. BATS_FILES narrows the run (e.g. while working).
BATS_JOBS ?= $(shell nproc 2>/dev/null || echo 1)
BATS_FILES ?= $(wildcard tests/unit/*.bats)
BATS_SERIAL ?= tests/unit/node_daemon.bats tests/unit/node_two_homes.bats tests/unit/restart.bats \
	tests/unit/review_resilience.bats tests/unit/wake_cmd.bats tests/unit/autopilot_review_cap.bats

# The runs' temporary files go on disk, not to a tmpfs /tmp: there what earlier runs left takes memory, and GNU parallel
# cannot write its buffers ("Is the disk full?") though the file system has room. Outside the repository, which
# agent-bats.sh refuses. tools/tmp-sweep.sh first removes what runs stopped hard left there and in /tmp.
BATS_TMP ?= $(or $(XDG_CACHE_HOME),$(HOME)/.cache)/team-agents/tmp

unit-bats:
	@$(TEST_LOCK) bash -c 'mkdir -p "$(BATS_TMP)" && tools/tmp-sweep.sh "$(BATS_TMP)" /tmp && \
		TMPDIR="$(BATS_TMP)" BATS_JOBS="$(BATS_JOBS)" BATS_FILES="$(BATS_FILES)" BATS_SERIAL="$(BATS_SERIAL)" tools/agent-bats.sh --lane oracle'

tools:
	tools/install-linters.sh

# The same checks with short output — one line per step, the failures condensed, full logs in build/gate/
# (tools/gate.sh). Agents use this: long tool output stays in their context and is read again at every later step.
gate:
	@$(TEST_LOCK) tools/gate.sh

lint: check-generated lint-shell lint-go lint-docs

lint-shell:
	@command -v shellcheck >/dev/null || { echo "shellcheck is missing: run make tools"; exit 1; }
	bash -n bin/agent install.sh tools/install-linters.sh tools/check-agent-test-map.sh tools/agent-bats.sh tools/check-agent-parity.sh tests/integration/run.sh
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
	build/doccheck agentbus/*.go live/*.go agent/*.go

# Real agent CLIs, real accounts (costs usage). See tests/integration/run.sh.
integration:
	AGENT_IT=1 tests/integration/run.sh

build-agent:
	cd agent && go build -o ../build/agent-go .

# Lane B: active/done rows of the Bats test suite against the real Go executable.
unit-agent-bats: build-agent
	@$(TEST_LOCK) bash -c 'tools/agent-bats.sh && tools/check-agent-parity.sh --evidence build/agent-bats-evidence.json'

# Verifies parity ledger, classifications, mapping, and hashes.
check-agent-parity:
	tools/check-agent-parity.sh

build: build-agent
	cd agentbus && go build -o ../build/agentbus .
	cd live && go build -o ../build/agentlive .

# Regenerates errors_gen.go from errors.json for agentbus and agent modules.
generate:
	cd agentbus && go generate ./...
	cd agent && go generate ./...

# Fails when errors_gen.go is out of date with errors.json.
check-generated:
	mkdir -p build
	cd tools/errgen && go run . -spec ../../agentbus/errors.json -out ../../build/errors_gen.go
	cmp -s build/errors_gen.go agentbus/errors_gen.go || { echo "agentbus/errors_gen.go is out of date: run make generate"; exit 1; }
	cd tools/errgen && go run . -spec ../../agent/errors.json -out ../../build/agent_errors_gen.go
	cmp -s build/agent_errors_gen.go agent/errors_gen.go || { echo "agent/errors_gen.go is out of date: run make generate"; exit 1; }
