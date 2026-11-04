#!/usr/bin/env bats
# Review of PR #61: a human stop of one task still hands its result to whoever waits for it, a finished or hung
# member of an --investigation does not block normal teams, install.sh keeps an explicit Serena opt-out, a fresh
# session gets the role-independent rules once, and the PR ownership rule says what a coordinator does when the
# subject's PR is on a branch it may not push.
load helpers

# stop_fixture writes a two-member team (A coordinator, sleeping; B member, running) with the board item B1 claimed
# by B, without starting any process.
stop_fixture() {
  local id st
  for id in A B; do
    st=sleeping; [[ $id == A ]] || st=running
    mkdir -p "$TASKS/$id"
    printf 'ID=%s\nAGENT=%s\nKIND=worker\nTEAM=T-team\nTIER=normal\nRUN=1\nSTATUS=%s\nREPO_ROOT=%s\nREPO_NAME=repo\nWORKTREE=%s\nBRANCH=agents/%s/x\n' \
      "$id" "$id" "$st" "$HOME/repo" "$HOME/repo" "$id" > "$TASKS/$id/meta.env"
  done
  printf '%s\n' '{"teams":{"T-team":{"id":"T-team","repo":"repo","lead":"A","members":{"A":{"agent":"A","state":"sleeping","tier":"normal","task":"A"},"B":{"agent":"B","state":"active","tier":"normal","task":"B"}},"board":{"B1":{"id":"B1","title":"parse","details":"parse it","level":"normal","status":"claimed","creator":"A","assignee":"B"}},"next_board_id":1}}}' > "$BUS_DIR/state.json"
}

# human_stopped id — what agent stop records before its runner's exit trap runs: the intent and the stopped status.
human_stopped() {
  touch "$TASKS/$1/stop-requested"
  meta_set "$1" STATUS stopped
}

# exit_stopped_runner id — the runner's exit trap of that task after a human stop (SIGTERM: exit code 143); the
# dispatchers only record that they ran.
exit_stopped_runner() {
  dispatch() { echo dispatch >> "$HOME/calls"; }
  dispatch_helpers() { echo dispatch_helpers >> "$HOME/calls"; }
  meta_load "$1"
  RUNNER_ID=$1 RUNNER_DONE=0 RUNNER_DELIVERED=1
  runner_exit 143
}

# board_status item — the status of a board item of T-team.
board_status() {
  agentbus team-get --dir "$BUS_DIR" --team T-team | jq -r --arg i "$1" '.board[$i].status'
}

# notices kind to — how many notices of that kind were sent to that agent.
notices() {
  local n
  [[ -f $BUS_DIR/messages.jsonl ]] || { echo 0; return; }
  n=$(jq -s --arg k "$1" --arg to "$2" '[.[] | select(.data.notice == $k and .to == $to)] | length' "$BUS_DIR/messages.jsonl")
  echo "$n"
}

@test "a member stopped by the human gives its board items back and its coordinator is told" {
  # shellcheck source=/dev/null
  source "$REPO_ROOT/bin/agent"
  stop_fixture
  human_stopped B
  exit_stopped_runner B
  [ "$(task_field B STATUS)" = stopped ]
  [ "$(board_status B1)" = todo ]
  [ "$(notices member_out A)" -eq 1 ]
  grep -qx dispatch "$HOME/calls"
}

@test "a member of a team the human stopped as a whole is not reported out and keeps its board items" {
  # shellcheck source=/dev/null
  source "$REPO_ROOT/bin/agent"
  stop_fixture
  human_stopped B
  meta_set B TEAM_STOPPED 1
  exit_stopped_runner B
  [ "$(task_field B STATUS)" = stopped ]
  [ "$(board_status B1)" = claimed ]
  [ "$(notices member_out A)" -eq 0 ]
}

@test "the members of a closed team are not told that its stopped coordinator is out" {
  # shellcheck source=/dev/null
  source "$REPO_ROOT/bin/agent"
  stop_fixture
  meta_set A STATUS running
  agentbus team-done --dir "$BUS_DIR" --team T-team
  human_stopped A
  exit_stopped_runner A
  [ "$(task_field A STATUS)" = stopped ]
  [ "$(notices member_out B)" -eq 0 ]
}

@test "an investigator requested by an agent and stopped by the human delivers a failed result to its requester" {
  # shellcheck source=/dev/null
  source "$REPO_ROOT/bin/agent"
  mkdir -p "$TASKS/I"
  printf 'ID=I\nAGENT=inv\nKIND=investigation\nINV_ID=5\nPARENT_TASK=R\nRUN=1\nSTATUS=running\nREPO_ROOT=%s\nREPO_NAME=repo\n' \
    "$HOME/repo" > "$TASKS/I/meta.env"
  inv_finish() { printf '%s|%s\n' "$1" "$2" >> "$HOME/inv_finish"; }
  human_stopped I
  exit_stopped_runner I
  [ "$(task_field I STATUS)" = stopped ]
  [ "$(cat "$HOME/inv_finish")" = "I|failed" ]
  grep -qx dispatch "$HOME/calls"
}

@test "an independent investigator stopped by the human delivers nothing to a requester" {
  # shellcheck source=/dev/null
  source "$REPO_ROOT/bin/agent"
  mkdir -p "$TASKS/IP" "$TASKS/I"
  printf 'ID=IP\nKIND=investigation_team\nSTATUS=running\nREPO_ROOT=%s\nREPO_NAME=repo\n' "$HOME/repo" > "$TASKS/IP/meta.env"
  printf 'ID=I\nAGENT=inv\nKIND=investigation\nINVESTIGATION_TEAM=IP\nINVESTIGATION_PHASE=1\nRUN=1\nSTATUS=running\nREPO_ROOT=%s\nREPO_NAME=repo\n' \
    "$HOME/repo" > "$TASKS/I/meta.env"
  inv_finish() { printf '%s|%s\n' "$1" "$2" >> "$HOME/inv_finish"; }
  human_stopped I
  exit_stopped_runner I
  [ "$(task_field I STATUS)" = stopped ]
  [ ! -e "$HOME/inv_finish" ]
}

@test "a helper stopped by the human sends a failed result to the agent that delegated to it" {
  # shellcheck source=/dev/null
  source "$REPO_ROOT/bin/agent"
  mkdir -p "$TASKS/H"
  printf 'ID=H\nAGENT=worker\nKIND=helper\nTIER=worker\nHELPER_ID=3\nFROM_AGENT=lead\nRUN=1\nSTATUS=running\nREPO_ROOT=%s\nREPO_NAME=repo\n' \
    "$HOME/repo" > "$TASKS/H/meta.env"
  helper_finish() { printf '%s|%s\n' "$1" "$2" >> "$HOME/helper_finish"; }
  human_stopped H
  exit_stopped_runner H
  [ "$(task_field H STATUS)" = stopped ]
  [ "$(cat "$HOME/helper_finish")" = "H|failed" ]
  grep -qx dispatch_helpers "$HOME/calls"
}

# investigation_cohort parent-status — a top-level --investigation of the fixture's repository (INV) with that status
investigation_cohort() {
  mkdir -p "$TASKS/INV"
  printf 'ID=INV\nKIND=investigation_team\nSTATUS=%s\nREPO_ROOT=%s\nREPO_NAME=repo\n' "$1" "$HOME/repo" > "$TASKS/INV/meta.env"
}

# cohort_member id status — an investigator of INV with that status
cohort_member() {
  mkdir -p "$TASKS/$1"
  printf 'ID=%s\nAGENT=%s\nKIND=investigation\nINVESTIGATION_TEAM=INV\nINVESTIGATION_PHASE=1\nSTATUS=%s\nREPO_ROOT=%s\nREPO_NAME=repo\n' \
    "$1" "$1" "$2" "$HOME/repo" > "$TASKS/$1/meta.env"
}

@test "a hung or excluded member of a finished --investigation does not block a normal team" {
  # shellcheck source=/dev/null
  source "$REPO_ROOT/bin/agent"
  stop_fixture
  investigation_cohort "done"
  cohort_member INVM1 hung
  cohort_member INVM2 excluded
  run normal_resume_allowed A
  [ "$status" -eq 0 ]
}

@test "a hung member blocks nobody even while its --investigation's record is not finished" {
  # shellcheck source=/dev/null
  source "$REPO_ROOT/bin/agent"
  stop_fixture
  mkdir -p "$TASKS/INVM"
  printf 'ID=INVM\nAGENT=INVM\nKIND=investigation\nINVESTIGATION_TEAM=GONE\nINVESTIGATION_PHASE=1\nSTATUS=hung\nREPO_ROOT=%s\nREPO_NAME=repo\n' \
    "$HOME/repo" > "$TASKS/INVM/meta.env"
  run normal_resume_allowed A
  [ "$status" -eq 0 ]
}

@test "a member still marked running does not block a normal team once its --investigation was stopped" {
  # shellcheck source=/dev/null
  source "$REPO_ROOT/bin/agent"
  stop_fixture
  investigation_cohort stopped
  cohort_member INVM running
  run normal_resume_allowed A
  [ "$status" -eq 0 ]
}

@test "a running member of an --investigation in progress still blocks a normal team" {
  # shellcheck source=/dev/null
  source "$REPO_ROOT/bin/agent"
  stop_fixture
  investigation_cohort running
  cohort_member INVM running
  run blocking_investigation A
  [ "$status" -eq 0 ]
  [[ $output == INV* ]]
}

@test "answers to a waiting member can be written while --investigation is in progress; the resume is refused" {
  # shellcheck source=/dev/null
  source "$REPO_ROOT/bin/agent"
  stop_fixture
  meta_set B STATUS waiting
  meta_set B ROUND 1
  printf '# Questions\n\n## 1. Which?\na) x\n\n**Recommended:** a)\n\n**Verdict:** a\n' > "$TASKS/B/questions-1.md"
  investigation_cohort running
  printf '#!/usr/bin/env bash\necho edited >> "%s/edited"\n' "$HOME" > "$HOME/editor"
  chmod +x "$HOME/editor"
  run env EDITOR="$HOME/editor" "$SELF" answer B <<< "y"
  grep -qx edited "$HOME/edited"
  [[ $output == *"--investigation"* ]]
  [ "$(task_field B RUN)" = 1 ]
}

@test "a stopped task's answers are still refused before the editor opens" {
  # shellcheck source=/dev/null
  source "$REPO_ROOT/bin/agent"
  stop_fixture
  meta_set B STATUS waiting
  meta_set B TEAM_STOPPED 1
  printf '#!/usr/bin/env bash\necho edited >> "%s/edited"\n' "$HOME" > "$HOME/editor"
  chmod +x "$HOME/editor"
  run env EDITOR="$HOME/editor" "$SELF" answer B
  [ "$status" -ne 0 ]
  [[ $output == *"stopped"* ]]
  [ ! -e "$HOME/edited" ]
}

# prompt_task — a solo task P whose prompt can be rendered without launching an agent.
prompt_task() {
  mkdir -p "$TASKS/P"
  printf 'ID=P\nAGENT=solo\nTYPE=claude\nKIND=worker\nREPO_NAME=repo\nREPO_ROOT=%s\nWORKTREE=%s\nBRANCH=main\nBASE=main\nRUN=1\n' \
    "$HOME/repo" "$HOME/repo" > "$TASKS/P/meta.env"
  printf 'Inspect the cache.\n' > "$TASKS/P/task.md"
  meta_load P
}

@test "a fresh session's prompt has the PR ownership rules and the Serena part once" {
  # shellcheck source=/dev/null
  source "$REPO_ROOT/bin/agent"
  prompt_task
  export AGENT_SERENA=serena FRESH_WHY='context limit'
  out=$(build_prompt P fresh "$TASKS/P/questions-1.md" 1)
  [ "$(grep -c '^## Pull request ownership' <<< "$out")" -eq 1 ]
  [ "$(grep -c 'MCP server `serena`' <<< "$out")" -eq 1 ]
  out=$(build_prompt P resume "$TASKS/P/questions-1.md" 1)
  [ "$(grep -c '^## Pull request ownership' <<< "$out")" -eq 1 ]
}

@test "the PR ownership rules tell a coordinator what to do when the subject's PR is on a branch it may not push" {
  # shellcheck source=/dev/null
  source "$REPO_ROOT/bin/agent"
  prompt_task
  local role out
  export KIND INVESTIGATION_TEAM INVESTIGATION_PHASE
  for role in solo merger; do
    KIND=worker INVESTIGATION_TEAM='' INVESTIGATION_PHASE=''
    [[ $role == solo ]] || { KIND=investigation INVESTIGATION_TEAM=I INVESTIGATION_PHASE=2; }
    meta_set P INVESTIGATION_TEAM "$INVESTIGATION_TEAM"
    out=$(build_prompt P new "$TASKS/P/questions-1.md" 1)
    [[ $out == *"head branch is not yours"* ]]
    [[ $out == *"STATUS: QUESTIONS"* ]]
    [[ $out != *"report to the coordinator."* ]]
    [[ $out != *'{{'* ]]
  done
  out=$(KIND=investigation INVESTIGATION_TEAM=I INVESTIGATION_PHASE=2 build_prompt P new "$TASKS/P/questions-1.md" 1)
  [[ $out == *"the PR on another branch"* ]]
}

# install_here [path] — runs the repository's install.sh with the stubs first on PATH (go, systemctl, systemd-run:
# no build, no unit is started); the PATH after the stubs is that one when given.
install_here() {
  mkdir -p "$HOME/stubs"
  printf '#!/usr/bin/env bash\nexit 0\n' > "$HOME/stubs/go"
  chmod +x "$HOME/stubs/go"
  ln -sf "$REPO_ROOT/tests/unit/stubs/systemctl" "$HOME/stubs/systemctl"
  ln -sf "$REPO_ROOT/tests/unit/stubs/systemd-run" "$HOME/stubs/systemd-run"
  run env PATH="$HOME/stubs:${1:-$PATH}" XDG_CONFIG_HOME="$HOME/.config" bash "$REPO_ROOT/install.sh"
}

# serena_installed — an executable serena where install.sh looks first ($HOME/.local/bin).
serena_installed() {
  printf '#!/usr/bin/env bash\nexit 0\n' > "$HOME/.local/bin/serena"
  chmod +x "$HOME/.local/bin/serena"
}

# path_without_serena — PATH without the directories that have a serena (the host's own, if any).
path_without_serena() {
  local d out=() dirs=()
  IFS=: read -r -a dirs <<< "$PATH"
  for d in "${dirs[@]}"; do [[ -x $d/serena ]] || out+=("$d"); done
  local IFS=:
  printf '%s' "${out[*]}"
}

@test "install.sh keeps an explicit empty AGENT_SERENA in an existing agents.conf, Serena installed" {
  serena_installed
  echo 'AGENT_SERENA=""' >> "$AGENTS_HOME/agents.conf"
  install_here
  [ "$status" -eq 0 ]
  [ "$(grep -c '^AGENT_SERENA=' "$AGENTS_HOME/agents.conf")" -eq 1 ]
  grep -qx 'AGENT_SERENA=""' "$AGENTS_HOME/agents.conf"
}

@test "install.sh adds AGENT_SERENA to an existing agents.conf without it when Serena is installed, and says so" {
  serena_installed
  install_here
  [ "$status" -eq 0 ]
  grep -qx 'AGENT_SERENA="serena"' "$AGENTS_HOME/agents.conf"
  [[ $output == *'AGENT_SERENA="serena"'* ]]
}

@test "install.sh leaves Serena off in a fresh agents.conf when it is not installed, and says so" {
  rm "$AGENTS_HOME/agents.conf"
  install_here "$(path_without_serena)"
  [ "$status" -eq 0 ]
  grep -qx 'AGENT_SERENA=""' "$AGENTS_HOME/agents.conf"
  [[ $output == *"Serena"* ]]
}

@test "install.sh enables Serena in a fresh agents.conf when it is installed" {
  rm "$AGENTS_HOME/agents.conf"
  serena_installed
  install_here
  [ "$status" -eq 0 ]
  grep -qx 'AGENT_SERENA="serena"' "$AGENTS_HOME/agents.conf"
}
