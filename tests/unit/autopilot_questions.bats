#!/usr/bin/env bats
# --autopilot (a team): nobody answers questions — they go back to the agent once, which takes the safest option it
# recommends and lists its decisions in the PR; asked again in the same round, they go to the human.

load helpers
bats_require_minimum_version 1.5.0   # run ! (a command that must fail)

# meta task key — a field of a task (as in verify_loop.bats)
meta() { sed -n "s/^$2=//p" "$AGENTS_HOME/tasks/$1/meta.env"; }

@test "--autopilot: questions go back to the agent once; it decides and lists its decisions in the PR; nobody waits" {
  scenario '{"lead": {"new": "team-lead-wait", "resume": "team-lead-wait"}, "member": {"new": "questions-file", "resume": "team-sleep"}}'
  agent run --team lead,member --autopilot "a task"
  t=$(task_of member)
  wait_status "$t" sleeping 60
  [ -f "$AGENTS_HOME/tasks/$t/questions-1-decided.md" ]
  grep -q "Decisions taken without the human" "$AGENTS_HOME/tasks/$t/prompt-2.md"
  run ! grep -q '"event":"run_finished".*"status":"waiting"' "$AGENTS_HOME/logs/agent.jsonl"
}

@test "--autopilot: questions asked again in the same round go to the human" {
  scenario '{"lead": {"new": "team-lead-wait", "resume": "team-lead-wait"}, "member": {"new": "questions-file", "resume": "questions-file"}}'
  agent run --team lead,member --autopilot "a task"
  t=$(task_of member)
  wait_status "$t" waiting 60
  [ -f "$AGENTS_HOME/tasks/$t/questions-1-decided.md" ]
}
