#!/usr/bin/env bats
# Workers were taken in the list's order within a provider: with eleven of them, openrouter-deepseek41 almost never
# had work. A worker is now drawn at random, among the free ones of another provider than the last one drawn.

load helpers

# workers — the human's eleven workers, of two providers (opencode models deepinfra/… and openrouter/…)
workers() {
  WORKER_AGENTS=(deepinfra-qwen openrouter-minimax deepinfra-glm openrouter-deepseek41 openrouter-qwen38-flash
    deepinfra-nemotron deepinfra-gptoss openrouter-luna openrouter-deepseek openrouter-qwen deepinfra-gemma)
  local w
  # shellcheck disable=SC2034 # read by bin/agent
  for w in "${WORKER_AGENTS[@]}"; do AGENT_TYPE[$w]=opencode; AGENT_MODEL[$w]="${w%%-*}/m"; done
  agent_running_task() { return 1; }   # every worker free
}

@test "after a worker of one provider, a worker of another is drawn — at random among them" {
  # shellcheck source=/dev/null
  source "$REPO_ROOT/bin/agent"
  workers
  declare -A seen=()
  for _ in $(seq 1 60); do
    echo openrouter > "$AGENTS_HOME/worker-last"
    w=$(free_worker)
    [[ $w == deepinfra-* ]]
    seen[$w]=1
  done
  ((${#seen[@]} >= 3))   # not the list's first deepinfra worker each time
}

@test "with only workers of the last provider free, one of them is drawn" {
  # shellcheck source=/dev/null
  source "$REPO_ROOT/bin/agent"
  workers
  agent_running_task() { [[ $1 == deepinfra-* ]]; }   # every deepinfra worker busy
  echo openrouter > "$AGENTS_HOME/worker-last"
  w=$(free_worker)
  [[ $w == openrouter-* ]]
}

@test "drawn and recorded one after another, every worker gets work — openrouter-deepseek41 too" {
  # shellcheck source=/dev/null
  source "$REPO_ROOT/bin/agent"
  workers
  declare -A seen=()
  for _ in $(seq 1 400); do
    w=$(free_worker)
    worker_drawn "$w"
    seen[$w]=1
  done
  [ "${#seen[@]}" -eq 11 ]
  [ -n "${seen[openrouter-deepseek41]:-}" ]
  [ "$(cat "$AGENTS_HOME/worker-last")" = "$(agent_provider "$w")" ]
}
