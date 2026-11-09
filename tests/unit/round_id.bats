#!/usr/bin/env bats
# The round of a task: one line "Round: <id>" of its text (R12, R12/A — parts of letters, digits, . _ - separated by
# /), "?" without one. agent run refuses a text with two such lines or an ID that breaks the pattern. Every task
# records its round in meta.env (ROUND_ID) when it is made: from its text, on a --resume from the resume text or the
# previous team's, and a task made for another task (a worker, an investigation, a review, an investigation team's
# phases) from that task, at that moment.

load helpers
load investigation_pipeline_fixture

# round_of task — the task's ROUND_ID as the script reads it (meta.env values are shell-quoted)
# shellcheck source=/dev/null
round_of() { (source "$AGENTS_HOME/tasks/$1/meta.env" && printf '%s' "${ROUND_ID-<unset>}"); }
meta() { sed -n "s/^$2=//p" "$AGENTS_HOME/tasks/$1/meta.env"; }

@test "round_id_of: none gives ?, one line its ID with blanks trimmed, two lines or a broken ID are refused" {
  source "$REPO_ROOT/bin/agent"
  [ "$(round_id_of $'a task\nwithout a round')" = '?' ]
  [ "$(round_id_of $'Do it.\nRound:   R12/A  \nmore')" = R12/A ]
  [ "$(round_id_of $'Round:\tR12.1_x-y\r')" = R12.1_x-y ]
  [ "$(round_id_of 'Round: harness/round-accounting')" = harness/round-accounting ]
  run round_id_of $'Round: R12\ntext\nRound: R13'
  [ "$status" -eq 1 ]
  [[ $output == *"line 3"* && $output == *"line 1"* && $output == *"Round: R13"* ]]
  local bad
  for bad in 'R12//A' '/R12' 'R12/' 'R12 A' 'R#12' ''; do
    run round_id_of "intro"$'\n'"Round: $bad"
    [ "$status" -eq 1 ] || { echo "accepted: '$bad' ($output)"; false; }
    [[ $output == *"line 2"* ]]
  done
  # "Round:" only at the start of a line counts: a word in the text is no round
  [ "$(round_id_of 'The Round: is named in plans')" = '?' ]
}

@test "agent run refuses a text with two Round: lines, or a broken ID, before anything starts" {
  run agent run solo $'Round: R12\nwork\nRound: R13'
  [ "$status" -ne 0 ]
  [[ $output == *"line 3"* ]]
  run agent run --team lead,member $'Round: R12//A\nwork'
  [ "$status" -ne 0 ]
  [[ $output == *"Round: R12//A"* ]]
  [[ -z $(ls -A "$AGENTS_HOME/tasks") ]]
  [[ -z $(git -C "$HOME/repo" branch --list 'agents/*') ]]
}

@test "agent run records the round of its text in every task it makes; a text without one gives ?" {
  scenario '{"solo": {"new": "done"}, "lead": {"new": "done"}, "member": {"new": "done"}}'
  agent run solo $'Round: R7\nsolo work' > /dev/null
  s=$(task_of solo)
  [ "$(round_of "$s")" = R7 ]
  wait_status "$s" done
  agent run solo 'more solo work' > /dev/null
  s2=$(task_of solo)
  [ "$(round_of "$s2")" = '?' ]
  wait_status "$s2" done
  agent run --team lead,member $'Build the parser.\n\nRound:  R12/A \n' > /dev/null
  [ "$(round_of "$(task_of lead)")" = R12/A ]
  [ "$(round_of "$(task_of member)")" = R12/A ]
  wait_status "$(task_of lead)" done
  wait_status "$(task_of member)" done
}

@test "agent run --investigation records the round in the investigation team and in each of its phases" {
  single_pipeline_setup
  run agent run --investigation --team inv $'Investigate a temporary fixture\nRound: R12/inv'
  [ "$status" -eq 0 ]
  parent=$(task_of investigation-team)
  [ "$(round_of "$parent")" = R12/inv ]
  wait_status "$parent" '^(done|failed)$' 45
  local t n=0
  for t in $(grep -lx "INVESTIGATION_TEAM=$parent" "$AGENTS_HOME"/tasks/*/meta.env | xargs -n1 dirname | xargs -n1 basename); do
    [ "$(round_of "$t")" = R12/inv ] || { echo "$t: $(round_of "$t")"; false; }
    n=$((n + 1))
  done
  ((n >= 3))   # phase 1, the merger, the reviewer
}

@test "agent run --investigation refuses a broken Round: line before anything starts" {
  single_pipeline_setup
  run agent run --investigation --team inv $'topic\nRound: R12 and more'
  [ "$status" -ne 0 ]
  [[ $output == *"line 2"* ]]
  [[ -z $(ls -A "$AGENTS_HOME/tasks") ]]
}

@test "agent run --resume keeps the previous round, takes the resume text's own Round: line, refuses a broken one" {
  scenario '{"solo": {"new": "done"}}'
  agent run solo $'Round: R12\nsolo work' > /dev/null
  s=$(task_of solo)
  wait_status "$s" done
  agent run --resume "$s" "go on" > /dev/null
  s2=$(task_of solo)
  [ "$s2" != "$s" ]
  [ "$(round_of "$s2")" = R12 ]
  wait_status "$s2" done
  agent run --resume "$s2" $'the next part\nRound: R12/B' > /dev/null
  s3=$(task_of solo)
  [ "$(round_of "$s3")" = R12/B ]
  [ "$(round_of "$s2")" = R12 ]   # the task before keeps its own
  wait_status "$s3" done
  run agent run --resume "$s3" $'Round: R12/B/\nwork'
  [ "$status" -ne 0 ]
  [[ $output == *"line 1"* ]]
  [ "$(task_of solo)" = "$s3" ]
}

@test "a hand-back's text (the system's, not the human's) never decides the round: the previous one is kept" {
  scenario '{"solo": {"new": "done"}}'
  agent run solo $'Round: R12\nsolo work' > /dev/null
  s=$(task_of solo)
  wait_status "$s" done
  AGENT_HANDBACK_LEAD=solo agent run --resume "$s" $'review findings\nRound: 2 of 3 is next' > /dev/null
  s2=$(task_of solo)
  [ "$s2" != "$s" ]
  [ "$(round_of "$s2")" = R12 ]
  wait_status "$s2" done
}

# asked — a team (round R12/A) whose member asks a worker to count files; sets m and team
asked() {
  printf 'AGENT_COORDINATOR_PREFER="lead"\n' >> "$AGENTS_HOME/agents.conf"
  scenario '{"lead": {"new": "team-lead-wait"}, "member": {"new": "team-sleep"}, "worker": {"new": "done"}, "worker2": {"new": "done"}}'
  agent run --team lead,member $'team task\nRound: R12/A' > /dev/null
  m=$(task_of member); team=$(meta "$m" TEAM)
  wait_status "$m" sleeping
  "$BUILD_DIR/agentbus" delegate --dir "$AGENTS_HOME/bus" --team "$team" --as member --task "$m" --repo repo \
    --branch "$(meta "$m" BRANCH)" --worktree "$(meta "$m" WORKTREE)" --agent-cmd agent --home "$AGENTS_HOME" \
    --request "count the files" --context "c" > /dev/null
}

@test "a worker gets its requester's round when it is made, and keeps it when the requester's changes" {
  asked
  local w=""
  for _ in $(seq 1 100); do
    w=$(grep -lx "PARENT_TASK=$m" "$AGENTS_HOME"/tasks/*/meta.env | xargs -r grep -lx 'KIND=helper' | head -n 1)
    [ -n "$w" ] && break
    sleep 0.2
  done
  w=$(basename "$(dirname "$w")")
  [ "$(round_of "$w")" = R12/A ]
  sed -i 's|^ROUND_ID=.*|ROUND_ID=R13|' "$AGENTS_HOME/tasks/$m/meta.env"
  [ "$(round_of "$w")" = R12/A ]
  run agent tokens "$team"
  [ "$status" -eq 0 ]
  [ "$(awk -v t="$w" '$1 == t {print $NF}' <<< "$output")" = R12/A ]
  agent stop --running > /dev/null
}

@test "an investigation and a review asked for by a task get that task's round" {
  scenario '{"solo": {"new": "sleep:300"}, "inv": {"new": "done"}}'
  agent run solo $'Round: R12/C\nsolo work' > /dev/null
  s=$(task_of solo)
  wait_status "$s" running
  "$BUILD_DIR/agentbus" inv-add --dir "$AGENTS_HOME/bus" --kind review --from-agent solo --from-task "$s" --repo repo \
    --branch "$(meta "$s" BRANCH)" --tasks-dir "$AGENTS_HOME/tasks" --question "review it" \
    --pr-url https://github.com/example/repo/pull/74 --pr-number 74 > /dev/null
  agent dispatch > /dev/null
  local r=""
  for _ in $(seq 1 100); do r=$(ls "$AGENTS_HOME/tasks" | grep -- '-inv[0-9]*$' || true); [ -n "$r" ] && break; sleep 0.2; done
  [ -n "$r" ]
  [ "$(meta "$r" REVIEW)" = 1 ]
  [ "$(round_of "$r")" = R12/C ]
  wait_status "$r" '^(done|failed)$'
  agent stop --running > /dev/null
}

@test "every place that makes a task's metadata (ROUND=0) records its ROUND_ID with it" {
  local n=0 line
  while IFS= read -r line; do
    n=$((n + 1))
    [[ $line == *ROUND_ID* ]] || { echo "no ROUND_ID: $line"; false; }
  done < <(grep -E "[\"' ]ROUND[= ]0[\"' ;]" "$REPO_ROOT/bin/agent")
  ((n >= 6))
}
