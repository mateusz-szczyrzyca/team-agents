#!/usr/bin/env bats
# AGENT_REVIEW_DRAW=rotate: every investigator takes its turn in --autopilot's review pairs. A pair is never the
# pair of either of the two reviews before it while another pair can be made of the free investigators, and those
# that reviewed least recently come first — whatever the PR, a fix pass's re-review included.

load helpers

# pair_of pr — the pair start_review_pair starts for a coordinator task of that PR with AGENT_REVIEW_DRAW=rotate,
# among the free investigators in FREE (four by default); prints "A B". The pairs' history stays in $AGENTS_HOME.
pair_of() {
  local id="T$1"
  mkdir -p "$AGENTS_HOME/tasks/$id"
  printf 'ID=%s\nREPO_NAME=repo\nPR_NUMBER=%s\nPR_URL=https://github.com/o/r/pull/%s\n' "$id" "$1" "$1" \
    > "$AGENTS_HOME/tasks/$id/meta.env"
  ( source "$REPO_ROOT/bin/agent"
    # shellcheck disable=SC2086  # FREE is a list of names
    free_investigator_list() { printf '%s\n' ${FREE:-inv-a inv-b inv-c inv-d}; }
    start_review_slot() { echo "$4" > "$2/$3.agent"; echo 1 > "$2/$3.inv"; }
    act() { :; }
    auto_stage() { :; }
    AGENT_REVIEW_DRAW=rotate start_review_pair "$id" > /dev/null
    d=$AGENTS_HOME/reviews/repo-$1
    echo "$(cat "$d/A.agent") $(cat "$d/B.agent")" )
}

# same_pair "a b" "c d" — exit 0 when the two pairs are the same two investigators, in either order
same_pair() {
  local a b c d
  read -r a b <<< "$1"; read -r c d <<< "$2"
  [[ ($a == "$c" && $b == "$d") || ($a == "$d" && $b == "$c") ]]
}

# different_pair "a b" "c d" — exit 0 when the two pairs are not the same two investigators
different_pair() { ! same_pair "$1" "$2"; }

@test "rotate: with four free investigators the second pair is the other two, the third one of each" {
  [ "$(pair_of 1)" = "inv-a inv-b" ]
  [ "$(pair_of 2)" = "inv-c inv-d" ]
  read -r a b <<< "$(pair_of 3)"
  [[ $a == inv-a || $a == inv-b ]]
  [[ $b == inv-c || $b == inv-d ]]
}

@test "rotate: a pair is never one of the two before it; six pairs are the six possible, each investigator in three" {
  pairs=()
  for n in 1 2 3 4 5 6; do pairs+=("$(pair_of "$n")"); done
  for i in 2 3 4 5; do
    different_pair "${pairs[i]}" "${pairs[i - 1]}"
    different_pair "${pairs[i]}" "${pairs[i - 2]}"
  done
  different_pair "${pairs[1]}" "${pairs[0]}"
  [ "$(printf '%s\n' "${pairs[@]}" | sort -u | wc -l)" -eq 6 ]   # a pair not seen yet before one seen
  for x in inv-a inv-b inv-c inv-d; do
    [ "$(printf '%s\n' "${pairs[@]}" | tr ' ' '\n' | grep -cx "$x")" -eq 3 ]
  done
}

@test "rotate: a PR's next review pass gets another pair (its re-review is not the same two investigators)" {
  first=$(pair_of 9)
  second=$(pair_of 9)
  different_pair "$second" "$first"
}

@test "rotate: three free investigators — after a,b and a,c the pair is b,c" {
  [ "$(FREE="inv-a inv-b inv-c" pair_of 1)" = "inv-a inv-b" ]
  [ "$(FREE="inv-a inv-b inv-c" pair_of 2)" = "inv-a inv-c" ]   # a: least recent with c, never used
  [ "$(FREE="inv-a inv-b inv-c" pair_of 3)" = "inv-b inv-c" ]
}

@test "rotate: an investigator that was busy during the last pairs is taken as soon as it is free" {
  [ "$(FREE="inv-a inv-b inv-c" pair_of 1)" = "inv-a inv-b" ]
  [ "$(FREE="inv-a inv-b inv-c" pair_of 2)" = "inv-a inv-c" ]
  read -r a b <<< "$(pair_of 3)"
  [[ $a == inv-d || $b == inv-d ]]
}

@test "rotate: with only two free investigators the pair is those two, even when they were the last pair" {
  [ "$(FREE="inv-a inv-b" pair_of 1)" = "inv-a inv-b" ]
  [ "$(FREE="inv-a inv-b" pair_of 2)" = "inv-a inv-b" ]
}

@test "rotate: the history keeps the last pairs, one per line, whatever the draw" {
  for n in $(seq 1 25); do pair_of "$n" > /dev/null; done
  [ "$(wc -l < "$AGENTS_HOME/review-pairs")" -eq 20 ]
  ( source "$REPO_ROOT/bin/agent"
    free_investigator_list() { printf '%s\n' inv-x inv-y; }
    start_review_slot() { echo "$4" > "$2/$3.agent"; echo 1 > "$2/$3.inv"; }
    act() { :; }
    auto_stage() { :; }
    mkdir -p "$AGENTS_HOME/tasks/T99"
    printf 'ID=T99\nREPO_NAME=repo\nPR_NUMBER=99\nPR_URL=u\n' > "$AGENTS_HOME/tasks/T99/meta.env"
    AGENT_REVIEW_DRAW=list start_review_pair T99 > /dev/null )
  [ "$(tail -n 1 "$AGENTS_HOME/review-pairs")" = "inv-x inv-y" ]
}

@test "--autopilot, rotate: three review passes that need fixes, each by another pair; then the human decides" {
  echo '[{"number": 7, "url": "https://github.com/o/r/pull/7", "state": "OPEN"}]' > "$HOME/gh-prs.json"
  printf 'INVESTIGATORS_TYPE[inv2]=claude\nINVESTIGATORS_TYPE[inv3]=claude\nINVESTIGATORS_TYPE[inv4]=claude\nAGENT_REVIEW_DRAW=rotate\nAGENT_REVIEW_GATE=true\nAGENT_HUMAN_GITHUB=boss\n' >> "$AGENTS_HOME/agents.conf"
  scenario '{"lead": {"new": "team-lead-done", "resume": "team-lead-done"}, "member": {"new": "team-sleep", "resume": "done"}, "inv": {"new": "review-fixes"}, "inv2": {"new": "review-fixes"}, "inv3": {"new": "review-fixes"}, "inv4": {"new": "review-fixes"}}'
  agent run --team lead,member --autopilot "team task"
  wait_grep '"event":"review_fixes_exhausted"' "$AGENTS_HOME/logs/agent.jsonl" 300
  wait_grep "@boss" "$HOME/gh.log"
  # each pass's pair: the investigators of the review tasks of each coordinator task, oldest first
  pairs=()
  for t in $(for d in "$AGENTS_HOME"/tasks/*-lead; do echo "${d##*/}"; done); do
    p=$(grep -l "^PARENT_TASK=$t\$" "$AGENTS_HOME"/tasks/*/meta.env | xargs sed -n 's/^AGENT=//p' | sort | tr '\n' ' ')
    pairs+=("$p")
  done
  [ "${#pairs[@]}" -eq 3 ]   # three reviews, no fourth pass
  for p in "${pairs[@]}"; do [ "$(wc -w <<< "$p")" -eq 2 ]; done
  [ "${pairs[1]}" != "${pairs[0]}" ]
  [ "${pairs[2]}" != "${pairs[1]}" ]
  [ "${pairs[2]}" != "${pairs[0]}" ]
  [ "$(printf '%s' "${pairs[*]}" | tr ' ' '\n' | grep -v '^$' | sort -u | wc -l)" -eq 4 ]   # all four reviewed
  third=$(for d in "$AGENTS_HOME"/tasks/*-lead; do echo "${d##*/}"; done | sed -n 3p)
  grep -q "Fix pass 2 of 2" "$AGENTS_HOME/tasks/$third/task.md"
  # a re-review by another investigator gets the previous review of its slot, named as someone else's: the third
  # pair (inv, inv3) follows the second (inv3, inv4)
  grep -q "The previous review of this PR, by inv3" "$AGENTS_HOME/bus/state.json"
  grep -q "The previous review of this PR, by inv4" "$AGENTS_HOME/bus/state.json"
}
