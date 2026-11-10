#!/usr/bin/env bats
# agent tokens <team>: a team's tasks are the tasks of the team and every task reachable from them through
# PARENT_TASK or INVESTIGATION_TEAM, at any depth — never another team's; each row ends with the task's ROLE and
# ROUND. agent tokens --round <id>: every task of that round or of a round inside it (<id>/…), then sums per role and
# per model, and the total.

load helpers

# task id meta — a task directory with that meta.env and an empty activity
task() { mkdir -p "$AGENTS_HOME/tasks/$1"; printf '%s' "$2" > "$AGENTS_HOME/tasks/$1/meta.env"; : > "$AGENTS_HOME/tasks/$1/activity.jsonl"; }
# act id text — an info line in the task's activity
act() { jq -nc --arg t "$2" '{ts: "2026-10-31 10:00:00", k: "info", t: $t}' >> "$AGENTS_HOME/tasks/$1/activity.jsonl"; }
# bus_team id lead-agent lead-task — the team on the bus, coordinated by lead-agent
bus_team() {
  jq -nc --arg id "$1" --arg a "$2" --arg t "$3" --arg home "$AGENTS_HOME" '{id: $id, repo: "repo", wake_cmd: "/bin/true",
      home: $home, lead: $a, members: {($a): {agent: $a, task: $t, state: "active", tier: "normal"}}}' \
    | agentbus team-create --dir "$AGENTS_HOME/bus"
}
# col task n — field n from the end (1 = last) of the task's row
col() { awk -v t="$1" -v n="$2" '$1 == t {print $(NF - n + 1)}' <<< "$output"; }

# a team of a coordinator and a member; the member's worker, which asks for an investigation; an investigation team
# the member asked for, with a phase and a review; two tasks linked in a cycle; another team with its own
# investigation, and one of its tasks linked to this team
team_fixture() {
  task T-lead $'ID=T-lead\nAGENT=lead\nTEAM=T-team\nKIND=worker\nROUND_ID=R12\n'
  act T-lead 'usage: in 1000 (cached 500), out 10'
  bus_team T-team lead T-lead
  task T-member $'ID=T-member\nAGENT=member\nTEAM=T-team\nKIND=worker\nROUND_ID=R12\n'
  act T-member 'usage: in 2000 (cached 1000), out 20'
  task H-worker $'ID=H-worker\nAGENT=worker\nKIND=helper\nPARENT_TASK=T-member\nROUND_ID=R12\n'
  act H-worker 'usage: in 300 (cached 0), out 3'
  task W-inv $'ID=W-inv\nAGENT=inv\nKIND=investigation\nPARENT_TASK=H-worker\nROUND_ID=R12\n'
  act W-inv 'usage: in 40 (cached 0), out 4'
  mkdir -p "$AGENTS_HOME/tasks/I-team"   # an investigation team's parent: no activity of its own
  printf 'ID=I-team\nAGENT=investigation-team\nKIND=investigation_team\nPARENT_TASK=T-member\nROUND_ID=R12\n' > "$AGENTS_HOME/tasks/I-team/meta.env"
  task I-phase $'ID=I-phase\nAGENT=inv\nKIND=investigation\nPARENT_TASK=I-team\nINVESTIGATION_TEAM=I-team\nROUND_ID=R12\n'
  act I-phase 'usage: in 5 (cached 0), out 1'
  task I-review $'ID=I-review\nAGENT=inv\nKIND=investigation\nREVIEW=1\nPARENT_TASK=I-team\nINVESTIGATION_TEAM=I-team\nROUND_ID=R12\n'
  act I-review 'usage: in 6 (cached 0), out 1'
  task Cyc-a $'ID=Cyc-a\nAGENT=inv\nKIND=investigation\nPARENT_TASK=T-lead\nINVESTIGATION_TEAM=Cyc-b\nROUND_ID=R12\n'
  act Cyc-a 'usage: in 7 (cached 0), out 1'
  task Cyc-b $'ID=Cyc-b\nAGENT=inv\nKIND=investigation\nPARENT_TASK=Cyc-a\nROUND_ID=R12\n'
  act Cyc-b 'usage: in 8 (cached 0), out 1'
  task X-lead $'ID=X-lead\nAGENT=lead\nTEAM=X-team\nKIND=worker\n'
  act X-lead 'usage: in 900001, out 1'
  task X-inv $'ID=X-inv\nAGENT=inv\nKIND=investigation\nPARENT_TASK=X-lead\n'
  act X-inv 'usage: in 900002, out 1'
  task X-linked $'ID=X-linked\nAGENT=member\nTEAM=X-team\nKIND=worker\nPARENT_TASK=T-member\n'
  act X-linked 'usage: in 900003, out 1'
}

@test "agent tokens <team>: every task reachable from the team's, at any depth, each with its role and round" {
  team_fixture
  run timeout 30 agent tokens T-team
  [ "$status" -eq 0 ]
  echo "$output" >&3
  [[ $(head -n 1 <<< "$output") == *"COST"*"ROLE"*"ROUND" ]]
  [ "$(col T-lead 2)" = coordinator ]
  [ "$(col T-member 2)" = member ]
  [ "$(col H-worker 2)" = worker ]
  [ "$(col W-inv 2)" = investigation ]    # asked for by a worker: three levels down
  [ "$(col I-team 2)" = investigation ]
  [ "$(col I-phase 2)" = investigation ]  # an investigation team's phase: two levels down
  [ "$(col I-review 2)" = review ]
  [ "$(col Cyc-a 2)" = investigation ]    # the cycle is followed once
  [ "$(col Cyc-b 2)" = investigation ]
  [ "$(col T-lead 1)" = R12 ]
  [[ $output != *"9000"* ]]               # another team's tasks and its investigation are not this team's
  [[ $output != *X-* ]]
  [ "$(grep -c '^Cyc-a ' <<< "$output")" -eq 1 ]
  [[ $(grep '^total ' <<< "$output") == *" 3366 "*" 1500 "*" 41 "* ]]
}

@test "agent tokens <task> ends its row with the role and the round; a task made before rounds shows -" {
  task S1 $'ID=S1\nAGENT=solo\nKIND=worker\nROUND_ID=R3/x\n'
  act S1 'usage: in 10 (cached 0), out 1'
  task S0 $'ID=S0\nAGENT=solo\nKIND=worker\n'
  act S0 'usage: in 10 (cached 0), out 1'
  run agent tokens S1
  [ "$status" -eq 0 ]
  [ "$(col S1 1)" = R3/x ]
  [ "$(col S1 2)" = coordinator ]   # a solo task leads its own work
  run agent tokens S0
  [ "$(col S0 1)" = - ]
}

@test "an old task without ROUND_ID does not inherit the caller's ambient value" {
  task Old $'ID=Old\nAGENT=solo\nKIND=worker\n'
  act Old 'usage: in 10 (cached 0), out 1'
  task Unknown $'ID=Unknown\nAGENT=solo\nKIND=worker\nROUND_ID=\'?\'\n'
  act Unknown 'usage: in 20 (cached 0), out 2'
  run env ROUND_ID='?' agent tokens Old
  [ "$status" -eq 0 ]
  [ "$(col Old 1)" = - ]
  run env ROUND_ID='?' agent tokens --round '?'
  [ "$status" -eq 0 ]
  [[ $output == *Unknown* ]]
  [[ $output != *Old* ]]
}

# round_fixture — tasks of rounds R12, R12/A, R12/A/#74 and of R120, R1 and none, in several roles and models
round_fixture() {
  task r12 $'ID=r12\nAGENT=lead\nMODEL=opus\nTEAM=R-team\nKIND=worker\nROUND_ID=R12\n'
  act r12 'usage: in 1000 (cached 100), out 10, cost $0.5000'
  bus_team R-team lead r12
  task r12a $'ID=r12a\nAGENT=member\nMODEL=sonnet\nTEAM=R-team\nKIND=worker\nROUND_ID=R12/A\n'
  act r12a 'usage: in 2000 (cached 200), out 20, cost $0.2500'
  task r12a74 $'ID=r12a74\nAGENT=worker\nMODEL=sonnet\nKIND=helper\nPARENT_TASK=r12a\nROUND_ID=\'R12/A/#74\'\n'
  act r12a74 'usage: in 400 (cached 40), out 4, cost $0.1250'
  task r12rev $'ID=r12rev\nAGENT=inv\nMODEL=opus\nKIND=investigation\nREVIEW=1\nPARENT_TASK=r12\nROUND_ID=R12\n'
  act r12rev 'usage: in 80 (cached 8), out 8'
  task r120 $'ID=r120\nAGENT=lead\nMODEL=opus\nKIND=worker\nROUND_ID=R120\n'
  act r120 'usage: in 700001, out 1'
  task r1 $'ID=r1\nAGENT=lead\nMODEL=opus\nKIND=worker\nROUND_ID=R1\n'
  act r1 'usage: in 700002, out 1'
  task rnone $'ID=rnone\nAGENT=lead\nMODEL=opus\nKIND=worker\n'
  act rnone 'usage: in 700003, out 1'
}

@test "agent tokens --round R12: R12 and the rounds inside it, never R120 or R1; sums per role and model, the total" {
  round_fixture
  run agent tokens --round R12
  [ "$status" -eq 0 ]
  echo "$output" >&3
  [ "$(col r12 2)" = coordinator ]
  [ "$(col r12a 2)" = member ]
  [ "$(col r12a74 2)" = worker ]
  [ "$(col r12a74 1)" = 'R12/A/#74' ]
  [ "$(col r12rev 2)" = review ]
  [[ $output != *"70000"* && $output != *r120* && $output != *rnone* ]]
  [ -z "$(col r1 1)" ]
  # sums: in, cached, out per role and per model match the rows
  [[ $(grep '^role coordinator ' <<< "$output") == *" 1000 "*" 100 "*" 10 "*"0.5000"* ]]
  [[ $(grep '^role member ' <<< "$output") == *" 2000 "*" 200 "*" 20 "*"0.2500"* ]]
  [[ $(grep '^role worker ' <<< "$output") == *" 400 "*" 40 "*" 4 "*"0.1250"* ]]
  [[ $(grep '^role review ' <<< "$output") == *" 80 "*" 8 "*" 8 "* ]]
  [[ $(grep '^model opus ' <<< "$output") == *" 1080 "*" 108 "*" 18 "*"0.5000"* ]]
  [[ $(grep '^model sonnet ' <<< "$output") == *" 2400 "*" 240 "*" 24 "*"0.3750"* ]]
  [[ $(grep '^total ' <<< "$output") == *" 3480 "*" 348 "*" 42 "*"0.8750"* ]]
  run agent tokens --round R12/A
  [ "$status" -eq 0 ]
  [[ -n $(col r12a 1) && -n $(col r12a74 1) && -z $(col r12 1) ]]
  [[ $(grep '^total ' <<< "$output") == *" 2400 "* ]]
}

@test "agent tokens --round without a task of that round says so" {
  round_fixture
  run agent tokens --round R99
  [ "$status" -ne 0 ]
  [[ $output == *"R99"* ]]
  run agent tokens --round
  [ "$status" -ne 0 ]
}
