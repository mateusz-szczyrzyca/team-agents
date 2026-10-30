#!/usr/bin/env bats
# Control receipts share the bounded heartbeat with cumulative job reports.

load helpers

# receipt_state writes valid pending acknowledgement fixtures with distinct, bounded identifiers.
receipt_state() {
  python3 - "$1" <<'PY'
import json, os, sys
from pathlib import Path
acks = [dict(Job='j%03d'%i+'x'*110, TaskID='T%03d'%i+'y'*110,
             File='questions-1.md', SHA256='a'*64) for i in range(int(sys.argv[1]))]
jobs = {a['Job']: dict(task_id=a['TaskID'], assignment=1, launched=True, obsolete=False,
                      state='done', answer_ack=a) for a in acks}
home = Path(os.environ['HOME'])
(home/'receipts.json').write_text(json.dumps(dict(jobs=jobs), separators=(',',':')))
(home/'declaration.json').write_text(json.dumps(dict(Node='node-test', AnswerAcks=acks), separators=(',',':')))
PY
}

@test "node sends at most 64 acknowledgement receipts and retains the remainder" {
  receipt_state 65
  # The script is loaded from the isolated test repository, never from the running installation.
  # shellcheck source=/dev/null
  source "$REPO_ROOT/bin/agent"
  NODE_STATE=$(cat "$HOME/receipts.json")
  local first second
  first=$(node_heartbeat_json)
  [ "$(jq '.AnswerAcks|length' <<< "$first")" -eq 64 ]
  node_clear_control_acks "$first"
  second=$(node_heartbeat_json)
  [ "$(jq '.AnswerAcks|length' <<< "$second")" -eq 1 ]
  [ "$(jq '[.jobs[].answer_ack//empty]|length' <<< "$NODE_STATE")" -eq 1 ]
}

@test "node fits valid reports and defers acknowledgements without losing them" {
  receipt_state 64
  python3 - <<'PY'
import datetime, json, os
from pathlib import Path
home = Path(os.environ['HOME'])
state = json.loads((home/'receipts.json').read_text())
declaration = json.loads((home/'declaration.json').read_text())
base = datetime.datetime(2026, 1, 1, tzinfo=datetime.timezone.utc)
report = dict(ID='j-report', Node='node-test', Assignment=1, State='running', TaskID='T_REPORT',
              Branch='agents/solo/report', Result='r'*65500, At='', Transitions=[])
for i in range(10000):
    at = (base+datetime.timedelta(seconds=i)).isoformat().replace('+00:00','Z')
    report['State'] = 'running' if i%2 == 0 else 'waiting'
    report['At'] = at
    report['Transitions'].append(dict(State=report['State'], At=at))
    if len(json.dumps(dict(Node='node-test',Reports=[report]),separators=(',',':'))) >= 255000:
        break
state['jobs']['j-report'] = dict(task_id='T_REPORT', assignment=1, launched=True,
                                obsolete=False, pending=True, report=report)
assert len(json.dumps(dict(Node='node-test',Reports=[report]),separators=(',',':'))) < 262144
assert len(json.dumps(dict(declaration,Reports=[report]),separators=(',',':'))) > 262144
(home/'receipts.json').write_text(json.dumps(state,separators=(',',':')))
(home/'report.json').write_text(json.dumps(report,separators=(',',':')))
PY
  run bash -c '
    source "$1"
    NODE_STATE=$(cat "$HOME/receipts.json")
    node_heartbeat_json() { cat "$HOME/declaration.json"; }
    node_request() { cat > "$HOME/sent.json"; printf "{\"Job\":null,\"Stops\":[]}\n"; }
    node_flush_reports
    printf "%s\n" "$NODE_STATE" > "$HOME/after.json"
  ' _ "$REPO_ROOT/bin/agent"
  if [[ $status -ne 0 ]]; then printf '%s\n' "$output" >&2; return 1; fi
  [ "$(wc -c < "$HOME/sent.json")" -le 262144 ]
  jq -e --slurpfile report "$HOME/report.json" '.Reports[0]==$report[0] and (has("AnswerAcks")|not)' "$HOME/sent.json" >/dev/null
  jq -e '[.jobs[].answer_ack//empty]|length==64' "$HOME/after.json" >/dev/null
}

@test "a stale answer is acknowledged without overwriting a different waiting round" {
  # The dynamic include uses the isolated HOME installed by helpers.
  # shellcheck source=/dev/null
  source "$REPO_ROOT/bin/agent"
  NODE_STATE='{"jobs":{"j1":{"task_id":"T1","assignment":1,"launched":true,"obsolete":false,"state":"waiting"}}}'
  mkdir -p "$TASKS/T1"
  printf 'ID=T1\nAGENT=solo\nSTATUS=waiting\nROUND=2\nRUN=3\n' > "$TASKS/T1/meta.env"
  printf 'previous round\n' > "$TASKS/T1/questions-1.md"
  printf 'current round\n' > "$TASKS/T1/questions-2.md"
  local content sha payload
  content=$'# Questions\n\n## 1. Proceed?\n**Verdict:** ok\n\n**ANSWERED**'
  sha=$(printf '%s' "$content" | sha256sum); sha=${sha%% *}
  payload=$(jq -nc --arg content "$content" --arg sha "$sha" '{Job:"j1",TaskID:"T1",File:"questions-1.md",Content:$content,SHA256:$sha}')
  node_apply_answer "$payload"
  [ "$(cat "$TASKS/T1/questions-1.md")" = 'previous round' ]
  [ "$(cat "$TASKS/T1/questions-2.md")" = 'current round' ]
  [ "$(task_field T1 STATUS)" = waiting ]
  jq -e '.jobs.j1.answer_ack.File=="questions-1.md"' <<< "$NODE_STATE" >/dev/null
}
