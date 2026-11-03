#!/usr/bin/env bats
# A pair that has reviewed a PR never reviews it again while another pair can: in every AGENT_REVIEW_DRAW mode. With
# four investigators A, B, C and D the reviews go A+B, then C+D, then mixed pairs (A+C, B+D, …). When only the PR's
# previous pair is free, the review waits for another investigator (AGENT_REVIEW_ROTATE_WAIT_MIN) before it repeats.

load helpers

# review_pass pr [seed] — one review pass of that PR: the pair start_review_pair starts for its coordinator task among
# the free investigators in FREE (four by default) out of those in ALL (FREE by default), the draw fixed by the seed;
# then both reviews end (their verdicts written, as review_pair_done does). Prints "A B", or fails with no pair.
review_pass() {
  local id="T$1"
  mkdir -p "$AGENTS_HOME/tasks/$id"
  printf 'ID=%s\nREPO_NAME=repo\nPR_NUMBER=%s\nPR_URL=https://github.com/o/r/pull/%s\nREVIEW_PAIR_SINCE=%s\n' \
    "$id" "$1" "$1" "${SINCE:-$(date +%s)}" > "$AGENTS_HOME/tasks/$id/meta.env"
  ( source "$REPO_ROOT/bin/agent"
    # shellcheck disable=SC2086  # FREE is a list of names
    free_investigator_list() { printf '%s\n' ${FREE:-inv-a inv-b inv-c inv-d}; }
    # shellcheck disable=SC2086  # ALL and FREE are lists of names
    investigator_names() { printf '%s\n' ${ALL:-${FREE:-inv-a inv-b inv-c inv-d}}; }
    start_review_slot() { echo "$4" > "$2/$3.agent"; echo 1 > "$2/$3.inv"; rm -f "$2/$3.verdict"; }
    act() { :; }
    auto_stage() { :; }
    AGENT_REVIEW_SEED=${2:-1} start_review_pair "$id" > /dev/null || exit 1
    d=$AGENTS_HOME/reviews/repo-$1
    echo needs-fixes > "$d/A.verdict"; echo needs-fixes > "$d/B.verdict"
    echo "$(cat "$d/A.agent") $(cat "$d/B.agent")" )
}

# shares pair pair — 0 when the two pairs ("a b") have exactly one investigator in common
shares_one() {
  local n=0 x
  for x in $1; do [[ " $2 " != *" $x "* ]] || n=$((n + 1)); done
  ((n == 1))
}

@test "default draw: a PR's second review is done by the other two investigators, its third by a mixed pair" {
  local s first second third
  for s in 1 2 3 4 5 6 7; do
    rm -rf "$AGENTS_HOME/reviews"
    first=$(review_pass "1$s" "$s")
    second=$(review_pass "1$s" "$((s + 3))")
    third=$(review_pass "1$s" "$((s + 5))")
    [ "$second" != "$first" ]
    ! shares_one "$first" "$second"
    for x in $first; do [[ " $second " != *" $x "* ]]; done
    [ "$third" != "$first" ] && [ "$third" != "$second" ]
    shares_one "$third" "$first" && shares_one "$third" "$second"
  done
}

@test "default draw: across PRs A+B is followed by C+D, then by a mixed pair" {
  local s p1 p2 p3
  for s in 1 2 3 4 5; do
    rm -rf "$AGENTS_HOME/reviews"
    p1=$(review_pass "2${s}1" "$s")
    p2=$(review_pass "2${s}2" "$((s + 1))")
    p3=$(review_pass "2${s}3" "$((s + 2))")
    for x in $p1; do [[ " $p2 " != *" $x "* ]]; done
    shares_one "$p3" "$p1" && shares_one "$p3" "$p2"
  done
}

@test "default draw: only the PR's previous pair free — it waits AGENT_REVIEW_ROTATE_WAIT_MIN, then repeats it" {
  [ "$(FREE="inv-a inv-b" ALL="inv-a inv-b inv-c inv-d" review_pass 31)" = "inv-a inv-b" ]
  FREE="inv-a inv-b" ALL="inv-a inv-b inv-c inv-d" run review_pass 31
  [ "$status" -ne 0 ]
  [ "$(wc -l < "$AGENTS_HOME/reviews/history")" -eq 1 ]
  [ "$(SINCE=$(($(date +%s) - 1300)) FREE="inv-a inv-b" ALL="inv-a inv-b inv-c inv-d" review_pass 31)" = "inv-a inv-b" ]
}

@test "default draw: one investigator of the previous pair busy — a new pair with the free ones, at once" {
  [ "$(review_pass 41)" != "" ]
  read -r a b < <(cat "$AGENTS_HOME/reviews/repo-41/A.agent" "$AGENTS_HOME/reviews/repo-41/B.agent" | paste -sd' ')
  local free
  free=$(printf '%s\n' inv-a inv-b inv-c inv-d | grep -vx "$b" | paste -sd' ')
  second=$(FREE="$free" review_pass 41)
  [ "$second" != "$a $b" ]
  [[ " $second " != *" $b "* ]]
}

@test "default draw: with two investigators their pair reviews again at once — the autopilot does not stall" {
  [ "$(FREE="inv-a inv-b" review_pass 51)" = "inv-a inv-b" ]
  [ "$(FREE="inv-a inv-b" review_pass 51)" = "inv-a inv-b" ]
}

@test "rotate: a PR's previous pair is not taken again, even when other PRs were reviewed since" {
  export AGENT_REVIEW_DRAW=rotate
  [ "$(FREE="inv-a inv-b inv-c" review_pass 61)" = "inv-a inv-b" ]
  [ "$(FREE="inv-a inv-b inv-c" review_pass 62)" = "inv-a inv-c" ]
  [ "$(FREE="inv-a inv-b inv-c" review_pass 63)" = "inv-b inv-c" ]
  [ "$(FREE="inv-a inv-b inv-c" review_pass 61)" != "inv-a inv-b" ]
}

@test "list: a PR's second review goes to the first free investigators outside its previous pair" {
  export AGENT_REVIEW_DRAW=list
  [ "$(review_pass 71)" = "inv-a inv-b" ]
  [ "$(review_pass 71)" = "inv-c inv-d" ]
  [ "$(review_pass 72)" = "inv-a inv-b" ]
}

@test "the supervise timer dispatches a pair that waits so as not to repeat its PR's pair (default draw)" {
  mkdir -p "$AGENTS_HOME/tasks/T1" "$AGENTS_HOME/reviews/repo-81"
  printf 'ID=T1\nSTATUS=done\nREPO_NAME=repo\nPR_NUMBER=81\nREVIEW_PAIR_PENDING=1\nREVIEW_PAIR_SINCE=%s\n' "$(date +%s)" \
    > "$AGENTS_HOME/tasks/T1/meta.env"
  echo inv-a > "$AGENTS_HOME/reviews/repo-81/A.agent"; echo inv-b > "$AGENTS_HOME/reviews/repo-81/B.agent"
  echo needs-fixes > "$AGENTS_HOME/reviews/repo-81/A.verdict"
  out=$( source "$REPO_ROOT/bin/agent"; dispatch() { echo dispatched; }; cmd__supervise )
  [[ $out == *dispatched* ]]
}
