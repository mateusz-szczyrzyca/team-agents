#!/usr/bin/env bats
# "done" is a status name passed as an argument (wait_status "$t" done), not the end of a loop.
# shellcheck disable=SC1010
# Unit tests of the `agent` script with stub CLIs (no real agents, systemd or GitHub).

load helpers

@test "a solo task finishes with STATUS: DONE and sends a notification" {
  scenario '{"solo": {"new": "done"}}'
  run agent run solo "do something"
  [ "$status" -eq 0 ]
  t=$(task_of solo)
  wait_status "$t" done
  # the status is written before the notification is sent, so wait for it instead of assuming the order
  wait_grep "✅ done" "$HOME/notifications.log"
}

@test "questions in a file: waiting → answer → resume" {
  scenario '{"solo": {"new": "questions-file", "resume": "done"}}'
  agent run solo "ambiguous task"
  t=$(task_of solo)
  wait_status "$t" waiting
  q="$AGENTS_HOME/tasks/$t/questions-1.md"
  run agent resume "$t"
  [ "$status" -ne 0 ]                                    # unanswered questions are rejected
  sed -i 's/^\*\*Answer:\*\*$/**Answer:** a/' "$q"
  agent resume "$t"
  wait_status "$t" done
  grep -q "Answer:\*\* a" "$HOME/prompt-solo-resume.txt"
}

@test "questions asked only in the final message are moved to a questions file" {
  scenario '{"solo": {"new": "questions-text"}}'
  agent run solo "task"
  t=$(task_of solo)
  wait_status "$t" waiting
  q="$AGENTS_HOME/tasks/$t/questions-1.md"
  grep -q '^## 1\. \*\*Format?\*\*' "$q" || grep -q '^## 1\. Format?' "$q"
  grep -q '^## 2\.' "$q"
}

@test "a turn without STATUS is passed to the human as a question" {
  scenario '{"solo": {"new": "no-status", "resume": "done"}}'
  agent run solo "task"
  t=$(task_of solo)
  wait_status "$t" waiting
  grep -q "What should the agent do next" "$AGENTS_HOME/tasks/$t/questions-1.md"
  sed -i 's/^\*\*Verdict:\*\*$/**Verdict:** continue as planned/' "$AGENTS_HOME/tasks/$t/questions-1.md"
  agent resume "$t"
  wait_status "$t" done
}

@test "the watchdog kills a hung run and resumes the session" {
  echo 'AGENT_HANG_SECS=3' >> "$AGENTS_HOME/agents.conf"
  scenario '{"solo": {"new": "hang", "resume": "done"}}'
  agent run solo "task"
  t=$(task_of solo)
  wait_status "$t" done 40
  grep -q "watchdog" "$AGENTS_HOME/tasks/$t/activity.jsonl"
  grep -q "A HUNG OPERATION WAS KILLED" "$HOME/prompt-solo-resume.txt"
}

@test "agent stop --running stops everything without notifications" {
  scenario '{"solo": {"new": "sleep:300"}, "lead": {"new": "sleep:300"}}'
  agent run solo "a"
  agent run lead "b"
  wait_status "$(task_of solo)" running
  wait_status "$(task_of lead)" running
  : > "$HOME/notifications.log"
  run agent stop --running
  [ "$status" -eq 0 ]
  [[ $output == *"Stopped 2 tasks"* ]]
  [ "$(status_of "$(task_of solo)")" = stopped ]
  [ "$(status_of "$(task_of lead)")" = stopped ]
  [ ! -s "$HOME/notifications.log" ]
}

@test "a team: the coordinator closes the team and its checkpoint is saved" {
  scenario '{"lead": {"new": "team-lead-done"}, "member": {"new": "team-sleep", "resume": "done"}}'
  run agent run --team lead,member "team task"
  [ "$status" -eq 0 ]
  wait_status "$(task_of lead)" done 30
  wait_status "$(task_of member)" done 30
  team=$(sed -n 's/^TEAM=//p' "$AGENTS_HOME/tasks/$(task_of lead)/meta.env")
  grep -q "stage 1 done" "$AGENTS_HOME/bus/checkpoints/team-$team.md"
}

@test "a team made only of workers is rejected" {
  run agent run --team worker,worker2 "x"
  [ "$status" -ne 0 ]
  [[ $output == *"workers cannot coordinate"* ]]
}

@test "agent run --resume continues a stopped team on the same branches" {
  scenario '{"lead": {"new": "sleep:300"}, "member": {"new": "sleep:300"}}'
  agent run --team lead,member "team task"
  old=$(task_of lead)
  wait_status "$old" running
  agent stop --running
  team=$(sed -n 's/^TEAM=//p' "$AGENTS_HOME/tasks/$old/meta.env")
  scenario '{"lead": {"new": "done"}, "member": {"new": "done"}}'
  run agent run --resume "$team" "also add chapter 3"
  [ "$status" -eq 0 ]
  new=$(task_of lead)
  [ "$new" != "$old" ]
  [ "$(sed -n 's/^BRANCH=//p' "$AGENTS_HOME/tasks/$new/meta.env")" = "$(sed -n 's/^BRANCH=//p' "$AGENTS_HOME/tasks/$old/meta.env")" ]
  wait_status "$new" done
  grep -q "RESUMED AFTER AN INTERRUPTION" "$HOME/prompt-lead-new.txt"
  grep -q "also add chapter 3" "$HOME/prompt-lead-new.txt"
}

@test "agent transcript merges bus messages and activity" {
  scenario '{"lead": {"new": "team-lead-done"}, "member": {"new": "team-sleep"}}'
  agent run --team lead,member "team task"
  wait_status "$(task_of lead)" done 30
  run agent transcript "$(task_of lead)"
  [ "$status" -eq 0 ]
  [[ $output == *"[bus]"* ]]
  [[ $output == *"[lead]"* ]]
}

@test "agent archive packs a finished task and removes it from tasks/" {
  command -v 7z >/dev/null || skip "7z not installed"
  scenario '{"solo": {"new": "done"}}'
  agent run solo "task"
  t=$(task_of solo)
  wait_status "$t" done
  run agent archive "$t"
  [ "$status" -eq 0 ]
  [ ! -d "$AGENTS_HOME/tasks/$t" ]
  ls "$AGENTS_HOME/tasks-archived/"*.7z
}

@test "every run writes structured JSON log records" {
  scenario '{"solo": {"new": "tool-done"}}'
  agent run solo "task"
  t=$(task_of solo)
  wait_status "$t" done
  log="$AGENTS_HOME/logs/agent.jsonl"
  wait_grep '"event":"run_finished"' "$log"
  # every line is a JSON object with the common fields
  run jq -e -s 'all(.[]; has("ts") and has("level") and .component == "agent" and has("event"))' "$log"
  [ "$status" -eq 0 ]
  for ev in task_created runner_starting run_started cli_finished run_finished; do
    jq -e --arg ev "$ev" --arg t "$t" 'select(.event == $ev and .task == $t)' "$log" >/dev/null
  done
  jq -e --arg t "$t" 'select(.event == "run_finished" and .task == $t and .status == "done")' "$log" >/dev/null
  # the bus writes its own log
  jq -e 'select(.component == "agentbus" and .event == "tool_call" and .level == "info" and .tool == "list_agents"
               and .agent == "solo")' \
    "$AGENTS_HOME/logs/agentbus.jsonl" >/dev/null
}

@test "agent log renders structured records and --raw shows them unchanged" {
  scenario '{"solo": {"new": "done"}}'
  agent run solo "task"
  t=$(task_of solo)
  wait_status "$t" done
  run timeout 2 agent log "$t"
  [[ $output == *"INFO run_started run=1 mode=new"* ]]
  [[ $output != *'"event":'* ]]
  run timeout 2 agent log "$t" --raw
  [[ $output == *'"event":"run_started"'* ]]
}

@test "a failing notification command is logged, not ignored" {
  printf '#!/bin/sh\necho "pushsafer: invalid key" >&2\nexit 7\n' > "$HOME/notify-fail.sh"
  chmod +x "$HOME/notify-fail.sh"
  echo "NOTIFY_CMD=\"$HOME/notify-fail.sh\"" >> "$AGENTS_HOME/agents.conf"
  scenario '{"solo": {"new": "done"}}'
  agent run solo "task"
  t=$(task_of solo)
  wait_status "$t" done
  log="$AGENTS_HOME/logs/agent.jsonl"
  wait_grep '"event":"notification_failed"' "$log"
  jq -e 'select(.event == "notification_failed" and .level == "warn" and .exit_code == "7"
               and (.stderr | contains("invalid key")))' "$log" >/dev/null
}

@test "script events are published as typed bus notices" {
  scenario '{"lead": {"new": "team-lead-slow"}, "member": {"new": "sleep:2"}}'
  agent run --team lead,member "team task"
  wait_status "$(task_of member)" done 30
  wait_status "$(task_of lead)" done 30
  msgs="$AGENTS_HOME/bus/messages.jsonl"
  # the member's DONE goes to the coordinator as a script notice with structured fields
  jq -e 'select(.kind == "script" and .data.notice == "member_finished" and .data.fields.agent == "member"
               and .to == "lead")' "$msgs" >/dev/null
  # events of agentbus itself carry their own kinds
  jq -e 'select(.kind == "team_created" and (.data.members | index("lead")))' "$msgs" >/dev/null
  jq -e 'select(.kind == "coordinator_claimed" and .data.coordinator == "lead")' "$msgs" >/dev/null
}

@test "the prompt tells the agent whether it may install missing tools" {
  scenario '{"solo": {"new": "done"}}'
  agent run solo "task"
  t=$(task_of solo)
  wait_status "$t" done
  grep -qE '^- Installing tools: you have (passwordless sudo|no sudo)\.' "$HOME/prompt-solo-new.txt"
  ! grep -q '{{PRIVILEGES}}' "$HOME/prompt-solo-new.txt"
}

@test "commits carry the agent's identity, signature and trailers; languages are set per audience" {
  ssh-keygen -q -t ed25519 -N "" -f "$HOME/agents-signing" >/dev/null
  cat >> "$AGENTS_HOME/agents.conf" <<CONF
AGENT_LANGUAGE="Polish"
GIT_EMAIL="12345+my-agents@users.noreply.github.com"
GIT_SIGNING_KEY="$HOME/agents-signing"
CONF
  scenario '{"solo": {"new": "commit"}}'
  agent run solo "add a file"
  t=$(task_of solo)
  wait_status "$t" done
  branch=$(sed -n 's/^BRANCH=//p' "$AGENTS_HOME/tasks/$t/meta.env")
  run git -C "$HOME/repo" log -1 --format='%an|%ae|%cn|%ce' "$branch"
  [ "$output" = "solo (team-agents)|12345+my-agents@users.noreply.github.com|solo (team-agents)|12345+my-agents@users.noreply.github.com" ]
  git -C "$HOME/repo" cat-file commit "$branch" | grep -q '^gpgsig -----BEGIN SSH SIGNATURE-----'
  run git -C "$HOME/repo" log -1 --format='%(trailers:key=Agent,valueonly)|%(trailers:key=Task,valueonly)' "$branch"
  [[ $output == "solo"*"|$t"* ]]
  prompt="$HOME/prompt-solo-new.txt"
  grep -q 'is written in \*\*English\*\*' "$prompt"      # repository and GitHub texts
  grep -q 'written in \*\*Polish\*\*' "$prompt"           # questions and the final message
  grep -q '^Agent-Model: claude$' "$prompt"
  ! grep -q '{{' "$prompt"
}

@test "without GIT_EMAIL commits still carry the agent's name" {
  scenario '{"solo": {"new": "commit"}}'
  agent run solo "add a file"
  t=$(task_of solo)
  wait_status "$t" done
  branch=$(sed -n 's/^BRANCH=//p' "$AGENTS_HOME/tasks/$t/meta.env")
  run git -C "$HOME/repo" log -1 --format='%an|%ae' "$branch"
  # the test user has no git e-mail configured: the documented fallback applies
  [ "$output" = "solo (team-agents)|solo@team-agents.invalid" ]
  ! git -C "$HOME/repo" cat-file commit "$branch" | grep -q '^gpgsig'
}

@test "a repository without AGENTS.md: warning at start, log record, note in the notification, prompt text" {
  scenario '{"solo": {"new": "done"}}'
  run agent run solo "task"
  [ "$status" -eq 0 ]
  [[ $output == *"WARNING: repo has no AGENTS.md"* ]]
  t=$(task_of solo)
  wait_status "$t" done
  jq -e 'select(.event == "agents_md_missing" and .level == "warn")' "$AGENTS_HOME/logs/agent.jsonl" >/dev/null
  wait_grep "Note: the repository has no AGENTS.md" "$HOME/notifications.log"
  grep -q 'This repository has no `AGENTS.md`' "$HOME/prompt-solo-new.txt"
}

@test "a repository with AGENTS.md: the prompt makes it binding and forbids changing it" {
  printf '# Test repository\n\nRun make test before committing.\n' > "$HOME/repo/AGENTS.md"
  git -C "$HOME/repo" add AGENTS.md
  git -C "$HOME/repo" -c user.name=t -c user.email=t@example.com commit -qm "Add AGENTS.md"
  scenario '{"solo": {"new": "done"}}'
  run agent run solo "task"
  [ "$status" -eq 0 ]
  [[ $output != *"has no AGENTS.md"* ]]
  t=$(task_of solo)
  wait_status "$t" done
  prompt="$HOME/prompt-solo-new.txt"
  grep -q 'read `AGENTS.md` in the repository root' "$prompt"
  grep -q 'Do not change any `AGENTS.md`' "$prompt"
  ! grep -q "no AGENTS.md" "$HOME/notifications.log"
}

@test "a team starts on a bus state written by the previous version (empty tiers)" {
  mkdir -p "$AGENTS_HOME/bus"
  cp "$BATS_TEST_DIRNAME/../../agentbus/testdata/state-legacy.json" "$AGENTS_HOME/bus/state.json"
  scenario '{"lead": {"new": "team-lead-done"}, "member": {"new": "sleep:1"}}'
  run agent run --team lead,member "team task"
  [ "$status" -eq 0 ]
  wait_status "$(task_of lead)" done 30
  ! grep -q '"tier": ""' "$AGENTS_HOME/bus/state.json"
}

@test "a team that cannot be created on the bus leaves no tasks, branches or worktrees behind" {
  mkdir -p "$AGENTS_HOME/bus"
  echo '{"teams":{"X":{"id":"X","members":{"a":{"agent":"a","state":"active","tier":"boss"}}}}}' > "$AGENTS_HOME/bus/state.json"
  scenario '{"lead": {"new": "done"}, "member": {"new": "done"}}'
  run agent run --team lead,member "team task"
  [ "$status" -ne 0 ]
  [[ $output == *"could not create the team on the bus"*"were removed"* ]]
  [ -z "$(ls "$AGENTS_HOME/tasks")" ]
  [ -z "$(git -C "$HOME/repo" branch --list 'agents/*')" ]
  [ "$(git -C "$HOME/repo" worktree list | wc -l)" -eq 1 ]
  jq -e 'select(.event == "run_abandoned")' "$AGENTS_HOME/logs/agent.jsonl" >/dev/null
}

@test "agent discard removes a task that never started and refuses one that ran" {
  scenario '{"solo": {"new": "done"}}'
  agent run solo "task"
  t=$(task_of solo)
  wait_status "$t" done
  run agent discard "$t"
  [ "$status" -ne 0 ]
  [[ $output == *"has already run"* ]]
  [ -d "$AGENTS_HOME/tasks/$t" ]
  # a task that was created but never started (as after an interrupted agent run)
  branch=$(sed -n 's/^BRANCH=//p' "$AGENTS_HOME/tasks/$t/meta.env")
  sed -i 's/^STATUS=.*/STATUS=new/' "$AGENTS_HOME/tasks/$t/meta.env"
  run agent discard "$t"
  [ "$status" -eq 0 ]
  [ ! -d "$AGENTS_HOME/tasks/$t" ]
  [ -z "$(git -C "$HOME/repo" branch --list "$branch")" ]
}

@test "a run stopped by an exhausted quota pauses the task and a timer resumes it" {
  scenario '{"solo": {"new": "quota", "resume": "done"}}'
  agent run solo "task"
  t=$(task_of solo)
  wait_status "$t" paused
  # paused, not failed: a timer is set for the reset time (1m30s + 1 min margin)
  wake_at=$(sed -n 's/^WAKE_AT=//p' "$AGENTS_HOME/tasks/$t/meta.env")
  (( wake_at - $(date +%s) > 120 && wake_at - $(date +%s) <= 150 ))
  timer=$(cat "$STUB_UNITS"/agent-"$t"-wake-*.timer)
  [[ $timer == 150s*"_wake $t" ]]
  wait_grep "⏸ paused" "$HOME/notifications.log"
  grep -q "quota or usage limit" "$HOME/notifications.log"
  jq -e --arg t "$t" 'select(.event == "task_paused" and .task == $t and .reason == "quota")' "$AGENTS_HOME/logs/agent.jsonl" >/dev/null
  # an early wake-up is deferred
  agent _wake "$t"
  [ "$(sed -n 's/^STATUS=//p' "$AGENTS_HOME/tasks/$t/meta.env")" = paused ]
  # the timer fires after the reset: the task resumes
  sed -i 's/^WAKE_AT=.*/WAKE_AT=1/' "$AGENTS_HOME/tasks/$t/meta.env"
  agent _wake "$t"
  wait_status "$t" done
}

@test "a team member stopped by an exhausted quota is paused, not dropped, and the team is told" {
  scenario '{"lead": {"new": "team-lead-slow"}, "member": {"new": "quota"}}'
  agent run --team lead,member "team task"
  m=$(task_of member)
  wait_status "$m" paused 30
  wait_member_state "$(sed -n 's/^TEAM=//p' "$AGENTS_HOME/tasks/$m/meta.env")" member paused
  wait_jq '.kind == "script" and .data.notice == "member_paused" and .data.fields.agent == "member"
           and .data.fields.reason == "quota"' "$AGENTS_HOME/bus/messages.jsonl"
  wait_status "$(task_of lead)" done 30
}

@test "the wake-loop guard pauses a member that keeps waking up without doing anything" {
  echo 'AGENT_WAKE_LOOP_MAX=2' >> "$AGENTS_HOME/agents.conf"
  scenario '{"lead": {"new": "sleep:15"}, "member": {"new": "idle", "resume": "idle"}}'
  agent run --team lead,member "team task"
  m=$(task_of member)
  wait_status "$m" sleeping 30
  team=$(sed -n 's/^TEAM=//p' "$AGENTS_HOME/tasks/$m/meta.env")
  # a message wakes it; it does nothing again → second idle run in a row → paused
  agentbus send --dir "$AGENTS_HOME/bus" --from human --to member "ping"
  wait_status "$m" paused 30
  [ "$(sed -n 's/^RUN=//p' "$AGENTS_HOME/tasks/$m/meta.env")" = 2 ]
  jq -e 'select(.event == "wake_loop_detected")' "$AGENTS_HOME/logs/agent.jsonl" >/dev/null
  wait_member_state "$team" member paused
  ls "$STUB_UNITS"/agent-"$m"-wake-*.timer >/dev/null
  # while paused, messages do not wake it
  agentbus send --dir "$AGENTS_HOME/bus" --from human --to member "ping again"
  sleep 3
  [ "$(sed -n 's/^RUN=//p' "$AGENTS_HOME/tasks/$m/meta.env")" = 2 ]
  wait_status "$(task_of lead)" done 30
}

@test "quota reset times are read from the messages of the CLIs" {
  source "$HOME/.local/bin/agent"
  d=$(mktemp -d)
  now=$(date +%s)
  printf 'Error: You have hit your usage limit. Try again in 2 hours 13 minutes.\n' > "$d/err-1.log"
  [ "$(quota_reset_seconds "$d" 1 "")" = $(( 2 * 3600 + 13 * 60 + 60 )) ]
  printf '' > "$d/err-1.log"
  printf '%s\n' '{"type":"result","is_error":true,"result":"Claude AI usage limit reached|'"$((now + 600))"'"}' > "$d/out-1.jsonl"
  secs=$(quota_reset_seconds "$d" 1 "")
  (( secs >= 655 && secs <= 661 ))
  rm "$d/out-1.jsonl"
  printf '429 Too Many Requests: rate limit exceeded\n' > "$d/err-1.log"
  [ "$(quota_reset_seconds "$d" 1 "")" = $(( AGENT_QUOTA_PAUSE_MIN * 60 + 60 )) ]
  printf "5-hour limit reached · resets 3pm\n" > "$d/err-1.log"
  secs=$(quota_reset_seconds "$d" 1 "")
  (( secs > 60 && secs <= 86400 + 60 ))
  # not a quota problem: no pause
  printf 'Error: connection reset by peer\n' > "$d/err-1.log"
  run quota_reset_seconds "$d" 1 ""
  [ "$status" -eq 1 ]
  # the agent's own report (with STATUS) is never read as a quota error
  printf '' > "$d/err-1.log"
  run quota_reset_seconds "$d" 1 $'Implemented the rate limit handling.\n\nSTATUS: DONE'
  [ "$status" -eq 1 ]
}

@test "questions without a recommendation are sent back to the agent once" {
  scenario '{"solo": {"new": "questions-norec", "resume": "questions-file"}}'
  agent run solo "task"
  t=$(task_of solo)
  wait_status "$t" waiting
  [ "$(sed -n 's/^RUN=//p' "$AGENTS_HOME/tasks/$t/meta.env")" = 2 ]
  grep -q "YOUR QUESTIONS NEED RECOMMENDATIONS" "$AGENTS_HOME/tasks/$t/prompt-2.md"
  [ "$(sed -n 's/^QUESTIONS_BOUNCED=//p' "$AGENTS_HOME/tasks/$t/meta.env")" = 1 ]
  grep -q '^\*\*Recommended:\*\* a)' "$AGENTS_HOME/tasks/$t/questions-1.md"
}

@test "saving the questions file marked ANSWERED sends the answers; ok accepts the recommendation" {
  scenario '{"solo": {"new": "questions-file", "resume": "done"}}'
  agent run solo "task"
  t=$(task_of solo)
  wait_status "$t" waiting
  q="$AGENTS_HOME/tasks/$t/questions-1.md"
  # the answering instructions are at the top; a path unit watches the file
  grep -q 'put \*\*ANSWERED\*\* on the last line' "$q"
  grep -q "PathModified=$q .*_answers_saved $t 1" "$STUB_UNITS/agent-$t-answers-1.path"
  # saved without the marker: nothing happens
  sed -i 's/^\*\*Answer:\*\*$/**Answer:** ok/' "$q"
  agent _answers_saved "$t" 1
  [ "$(sed -n 's/^STATUS=//p' "$AGENTS_HOME/tasks/$t/meta.env")" = waiting ]
  # marked ANSWERED, but a verdict is missing: you are told, nothing is sent
  sed -i 's/^\*\*Answer:\*\* ok$/**Answer:**/' "$q"
  printf '\n** ANSWERED **\n' >> "$q"
  agent _answers_saved "$t" 1
  [ "$(sed -n 's/^STATUS=//p' "$AGENTS_HOME/tasks/$t/meta.env")" = waiting ]
  wait_grep "not complete" "$HOME/notifications.log"
  # complete and marked: the answers go to the agent, "ok" expanded to the recommendation
  sed -i 's/^\*\*Answer:\*\*$/**Answer:** ok/' "$q"
  agent _answers_saved "$t" 1
  wait_status "$t" done
  grep -q "the human accepts the recommendation: a) — the simpler one" "$HOME/prompt-solo-resume.txt"
  run grep -q 'ANSWERED' "$HOME/prompt-solo-resume.txt"
  [ "$status" -ne 0 ]
  [ ! -f "$STUB_UNITS/agent-$t-answers-1.path" ]          # the watch is removed
}

@test "a new questions file opens in the VS Code window connected to this host" {
  export XDG_RUNTIME_DIR="$HOME/run"; mkdir -p "$XDG_RUNTIME_DIR"
  cli="$HOME/.vscode-server/cli/servers/Stable-abc/server/bin/remote-cli"
  mkdir -p "$cli"
  cat > "$cli/code" <<'STUB'
#!/usr/bin/env bash
# fake VS Code remote CLI: sockets named *dead* belong to closed windows
[[ $VSCODE_IPC_HOOK_CLI == *dead* ]] && { echo "Unable to connect to VS Code server" >&2; exit 1; }
echo "$VSCODE_IPC_HOOK_CLI $*" >> "$HOME/vscode.log"
STUB
  chmod +x "$cli/code"
  python3 -c 'import socket, sys
for p in sys.argv[1:]: socket.socket(socket.AF_UNIX).bind(p)' "$XDG_RUNTIME_DIR/vscode-ipc-live.sock" "$XDG_RUNTIME_DIR/vscode-ipc-dead.sock"
  touch -d '1 minute ago' "$XDG_RUNTIME_DIR/vscode-ipc-live.sock"   # the closed window's socket is newer
  scenario '{"solo": {"new": "questions-file"}}'
  agent run solo "task"
  t=$(task_of solo)
  wait_status "$t" waiting
  wait_grep "vscode-ipc-live.sock $AGENTS_HOME/tasks/$t/questions-1.md" "$HOME/vscode.log"
  run grep -q dead "$HOME/vscode.log"
  [ "$status" -ne 0 ]
}

@test "a relative NOTIFY_CMD is a configuration error that stops agent run" {
  echo 'NOTIFY_CMD="notify.sh"' >> "$AGENTS_HOME/agents.conf"
  scenario '{"solo": {"new": "done"}}'
  run agent run solo "task"
  [ "$status" -ne 0 ]
  [[ $output == *"NOTIFY_CMD must be an absolute path to an executable, got 'notify.sh'"* ]]
}

@test "a NOTIFY_CMD that is not an executable file is a configuration error" {
  printf 'not a program\n' > "$HOME/notify-plain.txt"
  echo "NOTIFY_CMD=\"$HOME/notify-plain.txt\"" >> "$AGENTS_HOME/agents.conf"
  scenario '{"solo": {"new": "done"}}'
  run agent run solo "task"
  [ "$status" -ne 0 ]
  [[ $output == *"is not an executable file"* ]]
}

@test "a NOTIFY_CMD of only whitespace is a configuration error, not an unbound variable" {
  echo 'NOTIFY_CMD="   "' >> "$AGENTS_HOME/agents.conf"
  scenario '{"solo": {"new": "done"}}'
  run agent run solo "task"
  [ "$status" -ne 0 ]
  [[ $output == *"only whitespace"* ]]
  [[ $output != *"unbound variable"* ]]
}

@test "agent status shows the last failed notification until a later delivery succeeds" {
  printf '#!/bin/sh\necho "pushsafer: invalid key" >&2\nexit 7\n' > "$HOME/notify-fail.sh"
  chmod +x "$HOME/notify-fail.sh"
  echo "NOTIFY_CMD=\"$HOME/notify-fail.sh\"" >> "$AGENTS_HOME/agents.conf"
  scenario '{"solo": {"new": "done"}}'
  agent run solo "task"
  wait_status "$(task_of solo)" done
  wait_grep "exit_code=7" "$AGENTS_HOME/notify-failure.txt"
  run agent status
  [ "$status" -eq 0 ]
  [[ $output == *"last notification failure: "*"exit_code=7"*"stderr=pushsafer: invalid key"* ]]

  printf '#!/bin/sh\necho delivered >> "$HOME/delivered.log"\n' > "$HOME/notify-fail.sh"
  scenario '{"solo": {"new": "done"}}'
  agent run solo "second task"
  wait_status "$(task_of solo)" done
  wait_grep "delivered" "$HOME/delivered.log"
  # the file is removed right after the successful delivery: poll the status until it is gone
  for _ in $(seq 1 100); do
    run agent status
    [[ $output != *"last notification failure"* ]] && break
  done
  [[ $output != *"last notification failure"* ]]
}
