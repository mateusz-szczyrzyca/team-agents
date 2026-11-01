#!/usr/bin/env bats
# A member's uncommitted work goes on into its resumed worktree even when git will not stash it as it is: a file added
# with --intent-to-add ("Entry … not uptodate. Cannot merge") stopped a whole round's resume. The changes are staged
# and stashed again; if git still refuses, the refusal says git's own words.

load helpers

meta() { sed -n "s/^$2=//p" "$AGENTS_HOME/tasks/$1/meta.env"; }

@test "a resume carries a member's changes on though one of its files was added with --intent-to-add" {
  scenario '{"lead": {"new": "sleep:300"}, "member": {"new": "sleep:300"}}'
  agent run --team lead,member "team task" > /dev/null
  lead=$(task_of lead)
  m=$(task_of member)
  wait_status "$m" running
  agent stop --running > /dev/null
  w=$(meta "$m" WORKTREE)
  echo "a change" >> "$w/README.md"
  echo "a new prompt" > "$w/new-prompt.md"
  git -C "$w" add -N new-prompt.md   # as an agent's CLI adds a new file
  run agent run --resume "$(meta "$lead" TEAM)" --same-round "go on"
  [ "$status" -eq 0 ]
  m2=$(task_of member)
  [ "$m2" != "$m" ]
  w2=$(meta "$m2" WORKTREE)
  grep -q "a change" "$w2/README.md"
  grep -q "a new prompt" "$w2/new-prompt.md"
  grep -q '"event":"resume_stash_retry"' "$AGENTS_HOME/logs/agent.jsonl"
  agent stop --running > /dev/null
}
