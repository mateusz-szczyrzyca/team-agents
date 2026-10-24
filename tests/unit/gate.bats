#!/usr/bin/env bats
# make gate (tools/gate.sh): the checks of `make lint test` with short output — one line per step, the relevant
# lines of a failure, the full logs in a directory. A long tool output stays in an agent's context and is read
# again at every later step; this keeps it small without hiding what a failure needs. The steps here are a fake
# Makefile whose targets print what the real tools print (and a lot of noise).

setup() {
  ROOT=$(mktemp -d)
  cat > "$ROOT/Makefile" <<'MK'
check-generated:
	@echo "cmp -s build/errors_gen.go agentbus/errors_gen.go"
noisy-ok:
	@for i in $$(seq 1 500); do echo "line $$i of a passing step"; done
unit-go:
	@for i in $$(seq 1 300); do echo "=== RUN   TestNoise$$i"; echo "--- PASS: TestNoise$$i (0.00s)"; done
	@echo "--- FAIL: TestCountRound (0.00s)"
	@echo "    roundsize_test.go:142: countRound: got 3 want 5"
	@echo "FAIL"
	@echo "FAIL	agentbus	0.031s"
	@exit 1
unit-bats:
	@echo "1..40"
	@for i in $$(seq 1 39); do echo "ok $$i passing test $$i"; done
	@echo "not ok 40 the round limit sends a big change back"
	@echo "# (in test file tests/unit/rounds.bats, line 30)"
	@echo "#   \`grep -q \"30 lines\" prompt.txt' failed"
	@for i in $$(seq 1 200); do echo "# log line $$i of the failed test"; done
	@exit 1
lint-go:
	@echo "live/web.go:12:5: G104: Errors unhandled (gosec)"
	@echo "	f.Close()"
	@echo "	^"
	@echo "1 issues:"
	@echo "* gosec: 1"
	@exit 1
MK
}

teardown() { rm -rf "$ROOT"; }

@test "passing steps take one line each; the full output is in the log" {
  run env GATE_ROOT="$ROOT" tools/gate.sh check-generated noisy-ok
  [ "$status" -eq 0 ]
  [ "${#lines[@]}" -le 4 ]
  [[ $output == *"ok"*"noisy-ok"* ]]
  [ "$(wc -l < "$ROOT/build/gate/noisy-ok.log")" -eq 500 ]
}

@test "a failing Go test shows its failure, not the hundreds of passing ones" {
  run env GATE_ROOT="$ROOT" tools/gate.sh unit-go
  [ "$status" -ne 0 ]
  [[ $output == *"FAIL"*"unit-go"*"build/gate/unit-go.log"* ]]
  [[ $output == *"--- FAIL: TestCountRound"* && $output == *"countRound: got 3 want 5"* ]]
  [[ $output != *"TestNoise"* ]]
  [ "${#lines[@]}" -le 12 ]
}

@test "a failing bats test shows its name and diagnosis, capped; the linter shows its findings without code" {
  run env GATE_ROOT="$ROOT" tools/gate.sh unit-bats lint-go
  [ "$status" -ne 0 ]
  [[ $output == *"not ok 40 the round limit sends a big change back"* && $output == *"rounds.bats, line 30"* ]]
  [[ $output != *"passing test"* ]]
  [[ $output == *"live/web.go:12:5: G104: Errors unhandled (gosec)"* ]]
  [[ $output != *"f.Close()"* ]]
  [ "${#lines[@]}" -le 40 ]
}

@test "every step runs even after a failure, and the summary names the failed ones" {
  run env GATE_ROOT="$ROOT" tools/gate.sh unit-go check-generated lint-go
  [ "$status" -ne 0 ]
  [[ $output == *"ok"*"check-generated"* ]]
  [[ $output == *"2 of 3 steps failed"* ]]
}
