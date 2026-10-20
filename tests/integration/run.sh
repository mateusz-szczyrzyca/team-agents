#!/usr/bin/env bash
# shellcheck disable=SC1010  # "== done" compares with the word "done"; it is not a loop keyword
# Integration tests with REAL agent CLIs (they use your accounts and cost tokens/usage limits).
#
#   AGENT_IT=1 tests/integration/run.sh                  # one agent per configured CLI type
#   AGENT_IT=1 IT_AGENTS="claude-1 deepinfra-glm" tests/integration/run.sh
#   AGENT_IT=1 IT_TEAM="claude-1,codex-1" tests/integration/run.sh   # also a small team
#
# The tests run in a separate AGENTS_HOME (a copy of your agents.conf, secrets.env and the repository's
# prompts) and in a throw-away git repository with a local bare remote, so your real tasks, bus state and
# repositories are not touched. Requirements: an installed `agent` (./install.sh), logged-in CLIs,
# systemd --user (with lingering) — exactly what real runs need.
set -euo pipefail

[[ ${AGENT_IT:-} == 1 ]] || { echo "integration tests are disabled — run with AGENT_IT=1"; exit 0; }

SRC=$(cd "$(dirname "$0")/../.." && pwd)
REAL_HOME=${AGENTS_HOME:-$HOME/agents}
TIMEOUT_MIN=${IT_TIMEOUT_MIN:-20}
WORK=$(mktemp -d "${TMPDIR:-/tmp}/agent-it.XXXXXX")
export AGENTS_HOME="$WORK/agents"
mkdir -p "$AGENTS_HOME"/{tasks,worktrees,bus,prompts}
cp "$REAL_HOME/agents.conf" "$REAL_HOME/secrets.env" "$AGENTS_HOME/"
cp "$SRC"/prompts/*.md "$AGENTS_HOME/prompts/"
[[ -f $REAL_HOME/hardware.md ]] && cp "$REAL_HOME/hardware.md" "$AGENTS_HOME/"
# no notifications from test runs
printf '\nNOTIFY_CMD=""\nNOTIFY_SCRIPT=""\nNOTIFY_DIR="%s"\n' "$AGENTS_HOME/notifications" >> "$AGENTS_HOME/agents.conf"

git init -q --bare "$WORK/remote.git"
git init -q -b main "$WORK/repo"
cd "$WORK/repo"
echo "# integration test repository" > README.md
git add README.md && git -c user.name=it -c user.email=it@localhost commit -q -m init
git remote add origin "$WORK/remote.git" && git push -q origin main

TASK_TEXT='This is an automated integration test of the agent tooling, not a real task. Do exactly this and nothing else:
1. Call the agentbus tool list_agents once.
2. Create the file it-check.txt containing exactly the line: integration ok
3. Commit it on your branch (do not push, do not open a pull request, do not use gh).
4. Finish with STATUS: DONE.
Everything is fully specified: do not ask questions.'

declare -A RESULT=()
status_of() { sed -n 's/^STATUS=//p' "$AGENTS_HOME/tasks/$1/meta.env"; }

# wait_done <task id> — waits until the task is finished (or waiting for answers / timeout)
wait_done() {
  local t=$1 end=$(( $(date +%s) + TIMEOUT_MIN * 60 )) st
  while (( $(date +%s) < end )); do
    st=$(status_of "$t")
    case $st in done|failed|stopped|hung|waiting|excluded) echo "$st"; return 0 ;; esac
    sleep 10
  done
  echo timeout
}

# check_task <task id> — verifies the outcome of a single-agent task
check_task() {
  local t=$1 dir="$AGENTS_HOME/tasks/$1" wt branch problems=()
  wt=$(sed -n 's/^WORKTREE=//p' "$dir/meta.env"); branch=$(sed -n 's/^BRANCH=//p' "$dir/meta.env")
  [[ $(status_of "$t") == done ]] || problems+=("status $(status_of "$t")")
  grep -q '"k":"mcp"' "$dir/activity.jsonl" 2>/dev/null || problems+=("no agentbus tool call in the activity log")
  git -C "$wt" show "$branch:it-check.txt" 2>/dev/null | grep -qx 'integration ok' || problems+=("it-check.txt missing on $branch")
  grep -q 'STATUS: *DONE' "$dir/final.md" 2>/dev/null || problems+=("final message without STATUS: DONE")
  if ((${#problems[@]})); then printf 'FAIL (%s)' "$(IFS='; '; echo "${problems[*]}")"; else printf 'ok'; fi
}

echo "» accounts"
agent authenticated || true

# agents to test: IT_AGENTS, or the first configured agent of each CLI type
if [[ -n ${IT_AGENTS:-} ]]; then
  read -ra agents <<< "$IT_AGENTS"
else
  agents=()
  for type in claude codex gemini opencode; do
    a=$(bash -c "source '$AGENTS_HOME/agents.conf'; for k in \"\${!AGENT_TYPE[@]}\"; do [[ \${AGENT_TYPE[\$k]} == $type ]] && echo \$k; done" | sort | head -1)
    [[ -n $a ]] && agents+=("$a")
  done
fi

for a in "${agents[@]}"; do
  echo "» $a"
  out=$(agent run "$a" "$TASK_TEXT")
  t=$(sed -n 's/^Started: //p' <<< "$out")
  echo "  task $t: $(wait_done "$t")"
  RESULT[$a]=$(check_task "$t")
done

if [[ -n ${IT_TEAM:-} ]]; then
  echo "» team $IT_TEAM"
  agent run --team "$IT_TEAM" "$TASK_TEXT
For a team: the coordinator does the steps above itself; the other members only answer the quiz and finish." >/dev/null
  team_done=0
  for _ in $(seq 1 $((TIMEOUT_MIN * 6))); do
    team_done=$(agent team --all 2>/dev/null | grep -c 'finished: true' || true)
    (( team_done > 0 )) && break
    sleep 10
  done
  RESULT[team]=$( (( team_done > 0 )) && echo ok || echo "FAIL (team not finished in ${TIMEOUT_MIN} min)")
fi

echo
echo "Results (logs: $AGENTS_HOME/tasks):"
fail=0
for k in "${!RESULT[@]}"; do
  printf '  %-20s %s\n' "$k" "${RESULT[$k]}"
  [[ ${RESULT[$k]} == ok ]] || fail=1
done
echo "Clean up afterwards: agent stop --running (with AGENTS_HOME=$AGENTS_HOME) and rm -rf $WORK"
exit $fail
