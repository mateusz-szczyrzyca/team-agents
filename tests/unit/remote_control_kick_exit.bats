#!/usr/bin/env bats
# A lead or human kick that stops a running unit must not journal a false failed transition from runner_exit.

load helpers

# kick_exit_fixture loads bin/agent in the isolated test home and stubs side-effect helpers.
kick_exit_fixture() {
  # The script is sourced from the isolated repository root.
  # shellcheck source=/dev/null
  source "$REPO_ROOT/bin/agent"
  mkdir -p "$LOG_DIR"
  : > "$LOG_DIR/agent.jsonl"
  # dispatch avoids starting unrelated tasks in this exit fixture.
  dispatch() { :; }
  # dispatch_helpers avoids starting unrelated helpers in this exit fixture.
  dispatch_helpers() { :; }
  # notify suppresses external notification delivery in this exit fixture.
  notify() { :; }
  # bus leaves team bookkeeping outside the runner-exit regression.
  bus() { :; }
  # notice leaves team messages outside the runner-exit regression.
  notice() { :; }
  # team_lead supplies the intended notification recipient without a live team.
  team_lead() { printf 'lead\n'; }
}

@test "kicking a solo job records stopped without journaling a failed runner exit" {
  kick_exit_fixture
  mkdir -p "$TASKS/T_SOLO"
  printf 'ID=T_SOLO\nAGENT=solo\nKIND=worker\nSTATUS=running\nRUN=1\nJOB_ID=j-solo\n' > "$TASKS/T_SOLO/meta.env"
  # systemctl runs the exit trap synchronously during the requested unit stop.
  systemctl() {
    if [[ ${1:-} == --user && ${2:-} == stop ]]; then
      (RUNNER_ID=T_SOLO RUNNER_DONE=0 JOB_ID=j-solo KIND=worker TEAM="" RUN=1 runner_exit 143)
    fi
  }
  cmd_kick T_SOLO lead > /dev/null
  [ "$(task_field T_SOLO STATUS)" = stopped ]
  jq -se 'map(.state) == ["stopped"]' "$TASKS/T_SOLO/job_transitions.jsonl" > /dev/null
}

@test "kicking a team member job records sleeping without journaling a failed runner exit" {
  kick_exit_fixture
  mkdir -p "$TASKS/T_MEMBER"
  printf 'ID=T_MEMBER\nAGENT=solo\nKIND=normal\nTEAM=T-team\nSTATUS=running\nRUN=2\nJOB_ID=j-member\n' > "$TASKS/T_MEMBER/meta.env"
  # team_after_run records any incorrect failure finalization by the runner.
  team_after_run() {
    printf 'unexpected team_after_run: %s\n' "$*" > "$HOME/team_after_run.err"
  }
  # systemctl runs the exit trap synchronously during the requested unit stop.
  systemctl() {
    if [[ ${1:-} == --user && ${2:-} == stop ]]; then
      (RUNNER_ID=T_MEMBER RUNNER_DONE=0 JOB_ID=j-member KIND=normal TEAM=T-team RUN=2 runner_exit 143)
    fi
  }
  cmd_kick T_MEMBER lead > /dev/null
  [ ! -f "$HOME/team_after_run.err" ]
  [ "$(task_field T_MEMBER STATUS)" = sleeping ]
  jq -se 'map(.state) == ["sleeping"]' "$TASKS/T_MEMBER/job_transitions.jsonl" > /dev/null
}

@test "an older kick does not mask an interrupted runner exit on a later run" {
  kick_exit_fixture
  mkdir -p "$TASKS/T_LATER"
  printf 'ID=T_LATER\nAGENT=solo\nKIND=worker\nSTATUS=running\nRUN=1\nJOB_ID=j-later\n' > "$TASKS/T_LATER/meta.env"
  # systemctl runs the exit trap synchronously during the requested unit stop.
  systemctl() {
    if [[ ${1:-} == --user && ${2:-} == stop ]]; then
      (RUNNER_ID=T_LATER RUNNER_DONE=0 JOB_ID=j-later KIND=worker TEAM="" RUN=1 runner_exit 143)
    fi
  }
  cmd_kick T_LATER lead > /dev/null
  meta_set T_LATER RUN 2
  meta_set T_LATER STATUS running
  : > "$TASKS/T_LATER/job_transitions.jsonl"
  (RUNNER_ID=T_LATER RUNNER_DONE=0 JOB_ID=j-later KIND=worker TEAM="" RUN=2 runner_exit 143)
  [ "$(task_field T_LATER STATUS)" = failed ]
  jq -se 'map(.state) == ["failed"]' "$TASKS/T_LATER/job_transitions.jsonl" > /dev/null
}
